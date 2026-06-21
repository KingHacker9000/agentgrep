"""
Deep dive into the actual commands run by the model in each task.
Extracts the full command text and outputs to understand strategy.
"""
import json, re, sys
from pathlib import Path

LOG_DIR = Path(r"C:\Dev\Tools\agentgrep\logs")

def extract_inner_cmd(full_cmd: str) -> str:
    """Extract the inner command from powershell wrapper."""
    m = re.search(r"-Command ['\"](.+?)['\"]$", full_cmd, re.DOTALL)
    if m:
        return m.group(1).strip()
    m = re.search(r"-Command (.+)$", full_cmd, re.DOTALL)
    if m:
        return m.group(1).strip("'\"").strip()
    return full_cmd

def classify_cmd(cmd: str) -> str:
    cl = cmd.lower()
    if "rg " in cl or "ripgrep" in cl:
        return "SEARCH"
    if "get-content" in cl or "cat " in cl or "type " in cl:
        return "READ"
    if "get-childitem" in cl or "dir " in cl or "ls " in cl or "find " in cl:
        return "LIST"
    if "cd " in cl or "set-location" in cl:
        return "NAVIGATE"
    return "OTHER"

def parse_log(path: Path):
    events = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                try:
                    events.append(json.loads(line))
                except:
                    pass
    return events

def analyze_deep(task_id: str, path: Path):
    events = parse_log(path)
    if not events:
        print(f"\n[{task_id}] EMPTY LOG")
        return

    print(f"\n{'='*72}")
    print(f"TASK: {task_id}")
    print(f"{'='*72}")

    tool_calls = []
    messages = []
    usage = {}

    for ev in events:
        if ev.get("type") == "turn.completed":
            usage = ev.get("usage", {})
        if ev.get("type") == "item.completed":
            item = ev.get("item", {})
            if item.get("type") == "agent_message":
                text = item.get("text", "")
                messages.append(text)
                # Print reasoning steps (short)
                if len(text) < 300:
                    print(f"\n  [MODEL] {text}")
                else:
                    print(f"\n  [MODEL] {text[:200]}...")

            if item.get("type") == "command_execution":
                cmd = item.get("command", "")
                output = item.get("aggregated_output", "")
                exit_code = item.get("exit_code", -1)
                inner = extract_inner_cmd(cmd)
                cmd_type = classify_cmd(inner)
                out_len = len(output)

                tool_calls.append({
                    "type": cmd_type, "cmd": inner,
                    "exit": exit_code, "out_len": out_len,
                    "output_preview": output[:200]
                })

                status = "OK" if exit_code == 0 else f"ERR({exit_code})"
                print(f"\n  [{cmd_type:>7}|{status}|{out_len:>8}B] {inner[:90]}")
                if out_len > 0:
                    preview = output.replace("\n", " | ")[:150]
                    print(f"              -> {preview}")

    print(f"\n  --- SUMMARY ---")
    print(f"  Tool calls: {len(tool_calls)}")
    type_counts = {}
    for tc in tool_calls:
        type_counts[tc["type"]] = type_counts.get(tc["type"], 0) + 1
    print(f"  By type: {type_counts}")
    total_bytes = sum(tc["out_len"] for tc in tool_calls)
    file_bytes = sum(tc["out_len"] for tc in tool_calls if tc["type"] == "READ")
    search_bytes = sum(tc["out_len"] for tc in tool_calls if tc["type"] == "SEARCH")
    print(f"  Total output bytes: {total_bytes:,}")
    print(f"    From reads:   {file_bytes:,}")
    print(f"    From search:  {search_bytes:,}")
    print(f"  Tokens: in={usage.get('input_tokens',0):,} out={usage.get('output_tokens',0):,} reasoning={usage.get('reasoning_output_tokens',0):,}")

    # Identify the BIGGEST single cost
    if tool_calls:
        biggest = max(tool_calls, key=lambda x: x["out_len"])
        print(f"\n  BIGGEST SINGLE CALL: {biggest['out_len']:,} bytes")
        print(f"    Type: {biggest['type']}")
        print(f"    Cmd: {biggest['cmd'][:120]}")
        print(f"    Preview: {biggest['output_preview'][:200]}")

    # Find search without -l (file-list only) flag
    unfiltered_searches = [tc for tc in tool_calls
                          if tc["type"] == "SEARCH" and "-l" not in tc["cmd"] and tc["out_len"] > 5000]
    if unfiltered_searches:
        print(f"\n  WARNING: {len(unfiltered_searches)} UNFILTERED SEARCH(ES) (no -l, >5KB output):")
        for s in unfiltered_searches:
            print(f"    {s['out_len']:,}B: {s['cmd'][:100]}")

    # Dead ends
    dead = [tc for tc in tool_calls if tc["exit"] != 0 or tc["out_len"] == 0]
    if dead:
        print(f"\n  DEAD ENDS ({len(dead)}):")
        for d in dead:
            print(f"    [{d['exit']}] {d['cmd'][:80]}")

    return tool_calls


def main():
    logs = sorted(LOG_DIR.glob("task*.jsonl"))
    for log in logs:
        if log.stat().st_size > 0:
            analyze_deep(log.stem, log)
        else:
            print(f"\n[{log.stem}] EMPTY FILE - skipping")


if __name__ == "__main__":
    main()
