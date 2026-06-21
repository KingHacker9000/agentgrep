"""
Secondary analysis: compare old workflow (raw rg) vs agentgrep 0.3.0 output
for the same tasks. Measures context bytes, calls needed, vocabulary accuracy.
"""
import json, subprocess, sys, time
from pathlib import Path

AG = r"C:\Dev\Tools\agentgrep\target\release\agentgrep.exe"
REPOS = {
    "flask":   r"C:\Dev\Tools\agentgrep\eval-worktree\flask",
    "bat":     r"C:\Dev\Tools\agentgrep\eval-worktree\bat",
    "fd":      r"C:\Dev\Tools\agentgrep\eval-worktree\fd",
    "ripgrep": r"C:\Dev\Tools\agentgrep\eval-worktree\ripgrep",
}

TASKS = [
    {
        "id": "task3_flask_blueprint",
        "repo": "flask",
        "find_query": "blueprint URL rule registration",
        "files_query": "blueprints",
        "trace_symbol": "add_url_rule",
        "peek_symbol": "add_url_rule",
        "peek_file": "sansio/scaffold.py",
        "correct_files": ["sansio/blueprints.py", "sansio/scaffold.py"],
        "correct_symbols": ["add_url_rule", "register", "BlueprintSetupState"],
        # Old workflow cost (from analysis)
        "old_calls": 17,
        "old_bytes": 89668,
        "old_tokens": 441155,
    },
    {
        "id": "task4_bat_syntax",
        "repo": "bat",
        "find_query": "syntax definition selection by file extension",
        "files_query": "syntax_mapping",
        "trace_symbol": "get_syntax_for_path",
        "peek_symbol": "SyntaxMapping",
        "peek_file": "syntax_mapping.rs",
        "correct_files": ["assets.rs", "syntax_mapping.rs"],
        "correct_symbols": ["get_syntax_for_path", "SyntaxMapping", "SyntaxSet"],
        "old_calls": 22,
        "old_bytes": 633150,
        "old_tokens": 477377,
    },
    {
        "id": "task5_fd_filetype",
        "repo": "fd",
        "find_query": "file type filter implementation",
        "files_query": "filetype",
        "trace_symbol": "FileType",
        "peek_symbol": "FileType",
        "peek_file": "cli.rs",
        "correct_files": ["filetypes.rs", "cli.rs"],
        "correct_symbols": ["FileType", "is_file", "filter_entry"],
        "old_calls": 8,
        "old_bytes": 149525,
        "old_tokens": 154556,
    },
    {
        "id": "task6_rg_color",
        "repo": "ripgrep",
        "find_query": "color application to matching output lines",
        "files_query": "color",
        "trace_symbol": "ColorSpecs",
        "peek_symbol": "ColorSpecs",
        "peek_file": "color.rs",
        "correct_files": ["printer/", "color.rs"],
        "correct_symbols": ["ColorSpecs", "ColorSpec", "color_match"],
        "old_calls": 14,
        "old_bytes": 191498,
        "old_tokens": 253209,
    },
]


def run_ag(args: list, cwd: str) -> dict:
    """Run agentgrep with JSON output, return parsed JSON + byte count."""
    cmd = [AG] + args + ["--json"]
    t0 = time.time()
    result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=30)
    elapsed = time.time() - t0
    raw = result.stdout
    try:
        data = json.loads(raw)
    except Exception:
        data = {"error": raw[:200]}
    return {"data": data, "bytes": len(raw.encode()), "elapsed": elapsed, "ok": result.returncode == 0}


def run_ag_text(args: list, cwd: str) -> dict:
    """Run agentgrep without JSON, return text + byte count."""
    cmd = [AG] + args
    result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=30)
    raw = result.stdout
    return {"text": raw, "bytes": len(raw.encode()), "ok": result.returncode == 0}


def analyze_task(task: dict) -> dict:
    repo_dir = REPOS[task["repo"]]

    print(f"\n{'='*70}")
    print(f"TASK: {task['id']}")
    print(f"{'='*70}")

    results = {}
    total_ag_bytes = 0
    total_ag_calls = 0
    correct_file_hits = []
    vocab_hits = []

    # 1. find --brief (replaces first O(500KB) rg search)
    r = run_ag_text(["find", task["find_query"], "--brief"], repo_dir)
    total_ag_bytes += r["bytes"]
    total_ag_calls += 1
    results["find_brief"] = r
    print(f"\n[find --brief]  {r['bytes']} bytes in {r['elapsed'] if 'elapsed' in r else '?'}s")
    print(r["text"][:600])

    # 2. find --json for vocabulary extraction
    r2 = run_ag(["find", task["find_query"]], repo_dir)
    total_ag_bytes += r2["bytes"]
    total_ag_calls += 1

    vocab = r2["data"].get("vocabulary", [])
    candidates = r2["data"].get("candidates", [])
    print(f"\n[find --json vocabulary]  vocab={vocab[:8]}")

    # Check if correct files are in candidates
    for cand in candidates:
        for cf in task["correct_files"]:
            if cf in cand.get("path", ""):
                if cf not in correct_file_hits:
                    correct_file_hits.append(cf)

    # Check vocab accuracy
    for sym in task["correct_symbols"]:
        if any(sym.lower() in v.lower() for v in vocab):
            vocab_hits.append(sym)

    results["find_json"] = {"bytes": r2["bytes"], "vocab": vocab, "correct_files": correct_file_hits}

    # 3. files command (replaces path guessing dead ends)
    r3 = run_ag_text(["files", task["files_query"]], repo_dir)
    total_ag_bytes += r3["bytes"]
    total_ag_calls += 1
    print(f"\n[files {task['files_query']!r}]  {r3['bytes']} bytes")
    print(r3["text"][:300])

    # 4. trace (replaces re-read cascades for callers/callees)
    r4 = run_ag_text(["trace", task["trace_symbol"]], repo_dir)
    total_ag_bytes += r4["bytes"]
    total_ag_calls += 1
    print(f"\n[trace {task['trace_symbol']!r}]  {r4['bytes']} bytes")
    print(r4["text"][:500] if r4["ok"] else f"  ERROR: {r4['text'][:100]}")

    # 5. peek with context (replaces whole-file reads + range re-reads)
    r5 = run_ag_text(["peek", task["peek_symbol"], "--file", task["peek_file"], "--context", "5"], repo_dir)
    total_ag_bytes += r5["bytes"]
    total_ag_calls += 1
    print(f"\n[peek {task['peek_symbol']!r} --context 5]  {r5['bytes']} bytes")
    print(r5["text"][:400] if r5["ok"] else f"  ERROR: {r5['text'][:100]}")

    print(f"\n--- SUMMARY for {task['id']} ---")
    print(f"  agentgrep calls:   {total_ag_calls}  (old: {task['old_calls']})")
    print(f"  agentgrep bytes:   {total_ag_bytes:,}  (old: {task['old_bytes']:,})")
    print(f"  byte reduction:    {(1 - total_ag_bytes/task['old_bytes'])*100:.0f}%")
    print(f"  correct file hits: {correct_file_hits}/{task['correct_files']}")
    print(f"  vocab symbol hits: {vocab_hits}/{task['correct_symbols']}")
    print(f"  old input tokens:  {task['old_tokens']:,}")

    # Estimated token reduction: each byte ≈ 0.25 tokens (rough estimate for code)
    est_new_tokens = int(total_ag_bytes * 0.25)
    print(f"  est. new tokens:   ~{est_new_tokens:,}  ({(1-est_new_tokens/task['old_tokens'])*100:.0f}% reduction)")

    return {
        "task_id": task["id"],
        "ag_calls": total_ag_calls,
        "ag_bytes": total_ag_bytes,
        "old_calls": task["old_calls"],
        "old_bytes": task["old_bytes"],
        "old_tokens": task["old_tokens"],
        "byte_reduction_pct": (1 - total_ag_bytes / task["old_bytes"]) * 100,
        "correct_file_hits": correct_file_hits,
        "vocab_hits": vocab_hits,
        "correct_files": task["correct_files"],
        "correct_symbols": task["correct_symbols"],
    }


def main():
    print("Secondary Analysis: agentgrep 0.3.0 vs traditional rg/cat workflow")
    print(f"Binary: {AG}")

    results = []
    for task in TASKS:
        try:
            r = analyze_task(task)
            results.append(r)
        except Exception as e:
            print(f"\nERROR in {task['id']}: {e}")

    print(f"\n\n{'='*70}")
    print("CROSS-TASK SUMMARY")
    print(f"{'='*70}")

    total_old_calls = sum(r["old_calls"] for r in results)
    total_ag_calls = sum(r["ag_calls"] for r in results)
    total_old_bytes = sum(r["old_bytes"] for r in results)
    total_ag_bytes = sum(r["ag_bytes"] for r in results)
    total_old_tokens = sum(r["old_tokens"] for r in results)

    print(f"\n{'TASK':<35} {'OLD_CALLS':>10} {'AG_CALLS':>9} {'OLD_BYTES':>11} {'AG_BYTES':>10} {'REDUCTION':>10}")
    print("-"*85)
    for r in results:
        print(f"  {r['task_id'][:33]:<33} {r['old_calls']:>10} {r['ag_calls']:>9} {r['old_bytes']:>11,} {r['ag_bytes']:>10,} {r['byte_reduction_pct']:>9.0f}%")
    print("-"*85)
    print(f"  {'TOTALS':<33} {total_old_calls:>10} {total_ag_calls:>9} {total_old_bytes:>11,} {total_ag_bytes:>10,} {(1-total_ag_bytes/total_old_bytes)*100:>9.0f}%")

    print(f"\nEstimated token impact:")
    print(f"  Old workflow total tokens: {total_old_tokens:,}")
    est_new_tokens = int(total_ag_bytes * 0.25)
    print(f"  agentgrep total est:       ~{est_new_tokens:,}")
    print(f"  Token reduction est:        {(1 - est_new_tokens/total_old_tokens)*100:.0f}%")

    print(f"\nVocabulary accuracy:")
    for r in results:
        hit_rate = len(r["vocab_hits"]) / max(1, len(r["correct_symbols"]))
        print(f"  {r['task_id']}: {len(r['vocab_hits'])}/{len(r['correct_symbols'])} symbols in vocab ({hit_rate*100:.0f}%)")

    print(f"\nCorrect file hit rate from find alone:")
    for r in results:
        hit_rate = len(r["correct_file_hits"]) / max(1, len(r["correct_files"]))
        print(f"  {r['task_id']}: {r['correct_file_hits']} ({hit_rate*100:.0f}%)")


if __name__ == "__main__":
    main()
