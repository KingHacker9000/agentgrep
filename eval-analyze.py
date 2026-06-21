"""
Analysis script for codex exec JSONL logs.
Measures token efficiency, tool usage patterns, and workflow weakpoints.
"""
import json
import re
import os
import sys
from pathlib import Path
from collections import Counter, defaultdict

LOG_DIR = Path(r"C:\Dev\Tools\agentgrep\logs")

TASK_META = {
    "task1_flask_teardown": {
        "repo": "flask",
        "query": "Request teardown context execution flow",
        "correct_files": ["ctx.py"],
        "key_symbols": ["pop", "teardown_request", "_cv_tokens"],
    },
    "task2_express_router": {
        "repo": "express",
        "query": "Router middleware dispatch logic",
        "correct_files": ["router/index.js", "router/layer.js"],
        "key_symbols": ["Router.handle", "next", "layer.handle_request"],
    },
    "task3_flask_blueprint": {
        "repo": "flask",
        "query": "Blueprint URL rule registration",
        "correct_files": ["sansio/scaffold.py", "sansio/blueprints.py"],
        "key_symbols": ["add_url_rule", "register", "url_prefix"],
    },
    "task4_bat_syntax": {
        "repo": "bat",
        "query": "Syntax definition selection by extension",
        "correct_files": ["assets.rs", "syntax_mapping.rs"],
        "key_symbols": ["SyntaxSet", "find_syntax_for_file", "SyntaxMapping"],
    },
    "task5_fd_filetype": {
        "repo": "fd",
        "query": "File type filter implementation",
        "correct_files": ["filter/", "filetypes.rs"],
        "key_symbols": ["FileType", "is_file", "filter_entry"],
    },
    "task6_rg_color": {
        "repo": "ripgrep",
        "query": "Color application in match output",
        "correct_files": ["printer/", "color.rs"],
        "key_symbols": ["ColorSpecs", "ColorSpec", "color_match"],
    },
}


def parse_log(path: Path) -> dict:
    events = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return events


def analyze_run(task_id: str, events: list, meta: dict) -> dict:
    tool_calls = []
    messages = []
    usage = {}
    files_read = set()
    search_commands = []
    dead_ends = []  # commands with empty/trivial output
    all_outputs = []

    for ev in events:
        if ev.get("type") == "turn.completed":
            usage = ev.get("usage", {})

        if ev.get("type") == "item.completed":
            item = ev.get("item", {})
            if item.get("type") == "agent_message":
                messages.append(item.get("text", ""))

            if item.get("type") == "command_execution":
                cmd = item.get("command", "")
                output = item.get("aggregated_output", "")
                exit_code = item.get("exit_code", -1)
                tool_calls.append({
                    "cmd": cmd,
                    "output": output,
                    "exit_code": exit_code,
                    "output_len": len(output),
                })
                all_outputs.append(output)

                # Classify command
                cmd_lower = cmd.lower()
                if any(x in cmd_lower for x in ["grep", "rg ", "ripgrep", "findstr", "select-string"]):
                    search_commands.append(cmd)
                if any(x in cmd_lower for x in ["cat ", "type ", "get-content", "head ", "tail "]):
                    # Extract file being read
                    fname = extract_filename(cmd)
                    if fname:
                        files_read.add(fname)

                # Dead end: empty output or error
                if not output.strip() or exit_code not in (0, None):
                    dead_ends.append({"cmd": cmd, "output": output[:100], "exit_code": exit_code})

    # Final answer (last agent message)
    final_answer = messages[-1] if messages else ""

    # Check if correct files are mentioned in answer or commands run
    all_text = final_answer + " " + " ".join(c["cmd"] for c in tool_calls) + " ".join(all_outputs)
    correct_hits = [f for f in meta.get("correct_files", []) if f.lower() in all_text.lower()]
    key_symbol_hits = [s for s in meta.get("key_symbols", []) if s.lower() in all_text.lower()]

    # Command type breakdown
    cmd_types = Counter()
    for tc in tool_calls:
        cmd_lower = tc["cmd"].lower()
        if any(x in cmd_lower for x in ["grep", " rg ", "select-string", "findstr"]):
            cmd_types["search"] += 1
        elif any(x in cmd_lower for x in ["cat ", "get-content", "type ", "head", "tail"]):
            cmd_types["read_file"] += 1
        elif any(x in cmd_lower for x in ["ls", "dir", "get-childitem", "find "]):
            cmd_types["list_dir"] += 1
        elif any(x in cmd_lower for x in ["cd ", "set-location"]):
            cmd_types["navigate"] += 1
        else:
            cmd_types["other"] += 1

    # Calculate total bytes read from files
    total_bytes_read = sum(
        tc["output_len"] for tc in tool_calls
        if any(x in tc["cmd"].lower() for x in ["cat ", "get-content", "type "])
    )

    # Large reads (> 3KB — likely reading entire files unnecessarily)
    large_reads = [
        {"cmd": tc["cmd"][:80], "size": tc["output_len"]}
        for tc in tool_calls
        if tc["output_len"] > 3000 and any(x in tc["cmd"].lower() for x in ["cat ", "get-content", "type "])
    ]

    # Re-reads: same output length appearing multiple times (heuristic for re-reading same file)
    output_lens = [tc["output_len"] for tc in tool_calls if tc["output_len"] > 500]
    reread_count = len(output_lens) - len(set(output_lens))

    return {
        "task_id": task_id,
        "repo": meta["repo"],
        "query": meta["query"],
        "total_tool_calls": len(tool_calls),
        "tool_call_breakdown": dict(cmd_types),
        "search_commands": len(search_commands),
        "files_read_count": len(files_read),
        "dead_ends": len(dead_ends),
        "dead_end_rate": len(dead_ends) / max(1, len(tool_calls)),
        "total_bytes_from_reads": total_bytes_read,
        "large_reads": large_reads,
        "large_read_count": len(large_reads),
        "reread_heuristic": reread_count,
        "correct_file_hits": correct_hits,
        "correct_file_hit_count": len(correct_hits),
        "key_symbol_hits": key_symbol_hits,
        "key_symbol_hit_count": len(key_symbol_hits),
        "answer_correct": len(correct_hits) >= 1 and len(key_symbol_hits) >= 2,
        "usage": usage,
        "input_tokens": usage.get("input_tokens", 0),
        "output_tokens": usage.get("output_tokens", 0),
        "reasoning_tokens": usage.get("reasoning_output_tokens", 0),
        "messages": len(messages),
        "final_answer_len": len(final_answer),
        # Full command trace for deep analysis
        "command_trace": [{"cmd": tc["cmd"][:120], "out_len": tc["output_len"], "exit": tc["exit_code"]} for tc in tool_calls],
        "search_command_list": search_commands[:10],
        "dead_end_list": dead_ends[:5],
    }


def extract_filename(cmd: str) -> str | None:
    """Heuristically extract filename from cat/read command."""
    patterns = [
        r"cat\s+['\"]?([^\s'\"]+)['\"]?",
        r"Get-Content\s+['\"]?([^\s'\"]+)['\"]?",
        r"type\s+['\"]?([^\s'\"]+)['\"]?",
    ]
    for pat in patterns:
        m = re.search(pat, cmd, re.IGNORECASE)
        if m:
            return m.group(1)
    return None


def print_report(results: list[dict]):
    print("\n" + "="*80)
    print("CODEX EXEC WORKFLOW ANALYSIS — gpt-5.4-mini @ low reasoning")
    print("="*80)

    print(f"\n{'TASK':<35} {'CALLS':>5} {'SEARCH':>6} {'READS':>5} {'DEAD%':>6} {'IN_TOK':>7} {'CORRECT':>8}")
    print("-"*80)

    total_tool_calls = 0
    total_dead_ends = 0
    total_tokens = 0
    total_large_reads = 0
    correct_count = 0

    for r in results:
        dead_pct = f"{r['dead_end_rate']*100:.0f}%"
        correct = "YES" if r["answer_correct"] else "NO "
        print(f"  {r['task_id'][:33]:<33} {r['total_tool_calls']:>5} {r['search_commands']:>6} {r['large_read_count']:>5} {dead_pct:>6} {r['input_tokens']:>7} {correct:>8}")
        total_tool_calls += r["total_tool_calls"]
        total_dead_ends += r["dead_ends"]
        total_tokens += r["input_tokens"]
        total_large_reads += r["large_read_count"]
        if r["answer_correct"]:
            correct_count += 1

    print("-"*80)
    print(f"  {'TOTALS':<33} {total_tool_calls:>5} {'':>6} {total_large_reads:>5} {total_dead_ends/max(1,total_tool_calls)*100:.0f}% {'':>7} {correct_count}/{len(results)}")

    print("\n\n=== PER-TASK DEEP DIVE ===\n")
    for r in results:
        print(f"\n{'-'*70}")
        print(f"TASK: {r['task_id']}  ({r['repo']})")
        print(f"Query: {r['query']}")
        print(f"Correct: {'YES' if r['answer_correct'] else 'NO '}  |  Correct file hits: {r['correct_file_hits']}  |  Symbol hits: {r['key_symbol_hits']}")
        print(f"Tool calls: {r['total_tool_calls']}  |  Breakdown: {r['tool_call_breakdown']}")
        print(f"Dead ends: {r['dead_ends']} ({r['dead_end_rate']*100:.0f}%)  |  Large reads: {r['large_read_count']}")
        print(f"Tokens: input={r['input_tokens']} output={r['output_tokens']} reasoning={r['reasoning_tokens']}")
        print(f"Bytes from file reads: {r['total_bytes_from_reads']:,}")

        print(f"\n  Command trace:")
        for i, tc in enumerate(r["command_trace"], 1):
            print(f"  {i:2}. [{tc['exit']}] {tc['cmd'][:80]}  -> {tc['out_len']} bytes")

        if r["dead_end_list"]:
            print(f"\n  Dead ends:")
            for de in r["dead_end_list"]:
                print(f"    [exit={de['exit_code']}] {de['cmd'][:70]}")
                if de["output"]:
                    print(f"      output: {de['output'][:60]}")

        if r["large_reads"]:
            print(f"\n  Large file reads (>3KB):")
            for lr in r["large_reads"]:
                print(f"    {lr['size']:,} bytes — {lr['cmd'][:60]}")

    print("\n\n=== WEAKPOINT PATTERN ANALYSIS ===\n")
    analyze_patterns(results)


def analyze_patterns(results: list[dict]):
    # Pattern 1: Tool call overhead per question answered
    total_calls = sum(r["total_tool_calls"] for r in results)
    correct = [r for r in results if r["answer_correct"]]
    incorrect = [r for r in results if not r["answer_correct"]]

    print(f"P1: Avg tool calls per task: {total_calls/len(results):.1f}")
    if correct:
        print(f"    Correct answers: avg {sum(r['total_tool_calls'] for r in correct)/len(correct):.1f} calls")
    if incorrect:
        print(f"    Wrong answers:   avg {sum(r['total_tool_calls'] for r in incorrect)/len(incorrect):.1f} calls")

    # Pattern 2: Dead end rate
    avg_dead = sum(r["dead_end_rate"] for r in results) / len(results)
    print(f"\nP2: Avg dead-end rate: {avg_dead*100:.1f}% of tool calls produce empty/error output")

    # Pattern 3: Token cost vs correctness
    avg_tokens = sum(r["input_tokens"] for r in results) / len(results)
    print(f"\nP3: Avg input tokens: {avg_tokens:,.0f}")
    print(f"    Correct tasks:   {sum(r['input_tokens'] for r in correct)/max(1,len(correct)):,.0f} avg tokens")
    print(f"    Incorrect tasks: {sum(r['input_tokens'] for r in incorrect)/max(1,len(incorrect)):,.0f} avg tokens")

    # Pattern 4: Large reads (whole-file reads when only section needed)
    total_large = sum(r["large_read_count"] for r in results)
    print(f"\nP4: Large file reads (>3KB): {total_large} across {len(results)} tasks ({total_large/len(results):.1f}/task)")

    # Pattern 5: Search command frequency
    total_searches = sum(r["search_commands"] for r in results)
    print(f"\nP5: Search commands: {total_searches} total ({total_searches/len(results):.1f}/task)")

    # Pattern 6: Bytes read from files
    total_bytes = sum(r["total_bytes_from_reads"] for r in results)
    print(f"\nP6: Total bytes read from files: {total_bytes:,} ({total_bytes/len(results):,.0f}/task avg)")

    # Pattern 7: Command type distribution
    all_types = Counter()
    for r in results:
        for k, v in r["tool_call_breakdown"].items():
            all_types[k] += v
    print(f"\nP7: Command type distribution across all tasks:")
    for cmd_type, count in all_types.most_common():
        print(f"    {cmd_type:<15} {count:>4} ({count/total_calls*100:.1f}%)")

    # Pattern 8: What the model got wrong
    print(f"\nP8: Correct answer rate: {len(correct)}/{len(results)}")
    for r in results:
        status = "OK" if r["answer_correct"] else "NO"
        print(f"    {status} {r['task_id']}: hit {len(r['correct_file_hits'])}/{len(TASK_META[r['task_id']].get('correct_files',['?']))} correct files, {len(r['key_symbol_hits'])}/{len(TASK_META[r['task_id']].get('key_symbols',['?']))} symbols")


def main():
    log_files = sorted(LOG_DIR.glob("task*.jsonl"))
    if not log_files:
        print("No log files found in", LOG_DIR)
        sys.exit(1)

    results = []
    for log_path in log_files:
        task_id = log_path.stem
        if task_id not in TASK_META:
            print(f"Skipping unknown task: {task_id}")
            continue
        meta = TASK_META[task_id]
        print(f"Parsing {log_path.name} ...", end=" ", flush=True)
        events = parse_log(log_path)
        if not events:
            print("EMPTY")
            continue
        result = analyze_run(task_id, events, meta)
        results.append(result)
        print(f"OK ({result['total_tool_calls']} tool calls, {result['input_tokens']} tokens)")

    if not results:
        print("No results to analyze")
        sys.exit(1)

    print_report(results)

    # Save raw analysis JSON
    out_path = LOG_DIR / "analysis_results.json"
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2, default=str)
    print(f"\nRaw analysis saved to: {out_path}")


if __name__ == "__main__":
    main()
