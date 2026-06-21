"""
Pass 2: Cross-run pattern analysis outside the initial FOV.
Focuses on:
- Context accumulation dynamics (how prior outputs inflate later token counts)
- Cargo.lock/CHANGELOG pollution rate
- Output truncation thresholds
- Vocabulary fishing (multi-term OR blasts)
- Path-guessing failure types
- Re-read cascade depth
"""
import json, re
from pathlib import Path
from collections import defaultdict

LOG_DIR = Path(r"C:\Dev\Tools\agentgrep\logs")

def parse_log(path):
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

def extract_cmds(events):
    cmds = []
    for ev in events:
        if ev.get("type") == "item.completed":
            item = ev.get("item", {})
            if item.get("type") == "command_execution":
                cmd = item.get("command", "")
                output = item.get("aggregated_output", "")
                exit_code = item.get("exit_code", -1)
                # Extract inner command
                m = re.search(r"-Command\s+['\"]?(.+?)['\"]?\s*$", cmd)
                inner = m.group(1) if m else cmd
                cmds.append({
                    "raw": cmd,
                    "inner": inner,
                    "output": output,
                    "exit_code": exit_code,
                    "out_len": len(output),
                })
    return cmds

def analyze_pollution(cmds):
    """Check how much of search output is non-source-code files."""
    NOISE_PATTERNS = [
        (r"Cargo\.lock", "cargo.lock"),
        (r"CHANGES\.rst|CHANGELOG\.md|CHANGELOG\.txt", "changelog"),
        (r"README\.md|README\.rst", "readme"),
        (r"\.json\b", "json_data"),
        (r"target/", "build_artifacts"),
        (r"tests?/", "test_files"),
        (r"doc[s]?/", "docs"),
    ]
    searches = [c for c in cmds if "rg " in c["inner"].lower() and "-l" not in c["inner"]]
    results = []
    for s in searches:
        lines = s["output"].split("\n")
        total = len(lines)
        noise_by_type = defaultdict(int)
        for line in lines:
            for pattern, label in NOISE_PATTERNS:
                if re.search(pattern, line, re.IGNORECASE):
                    noise_by_type[label] += 1
                    break
        noise_total = sum(noise_by_type.values())
        results.append({
            "cmd": s["inner"][:80],
            "total_lines": total,
            "noise_lines": noise_total,
            "noise_pct": noise_total / max(1, total) * 100,
            "noise_types": dict(noise_by_type),
            "bytes": s["out_len"],
        })
    return results

def analyze_context_growth(events):
    """Track how turn input_tokens grows as outputs accumulate."""
    turns = []
    cumulative_output = 0
    current_turn_output = 0

    for ev in events:
        if ev.get("type") == "turn.started":
            current_turn_output = 0
        if ev.get("type") == "item.completed":
            item = ev.get("item", {})
            if item.get("type") == "command_execution":
                out = item.get("aggregated_output", "")
                current_turn_output += len(out)
                cumulative_output += len(out)
        if ev.get("type") == "turn.completed":
            usage = ev.get("usage", {})
            turns.append({
                "input_tokens": usage.get("input_tokens", 0),
                "cached_tokens": usage.get("cached_input_tokens", 0),
                "output_tokens": usage.get("output_tokens", 0),
                "cumulative_output_bytes": cumulative_output,
                "this_turn_output_bytes": current_turn_output,
            })
    return turns

def analyze_or_searches(cmds):
    """Find multi-term OR searches and count their terms."""
    searches = [c for c in cmds if "rg " in c["inner"].lower()]
    vocab_blasts = []
    for s in searches:
        # Count | in the search pattern (OR terms)
        m = re.search(r'rg\s+.*?["\']([^"\']+)["\']', s["inner"])
        if m:
            pattern = m.group(1)
            or_terms = [t.strip() for t in pattern.split("|")]
            if len(or_terms) >= 3:
                vocab_blasts.append({
                    "terms": or_terms,
                    "count": len(or_terms),
                    "bytes_returned": s["out_len"],
                    "cmd": s["inner"][:100],
                })
    return vocab_blasts

def analyze_reruns(cmds):
    """Detect repeated reads of the same file."""
    file_reads = defaultdict(list)
    for c in cmds:
        if "get-content" in c["inner"].lower() or "cat " in c["inner"].lower():
            # Extract base file path (without Select-Object)
            inner = c["inner"]
            base = re.split(r'\s*\|\s*', inner)[0].strip()
            m = re.search(r'Get-Content\s+["\']?([^\s"\'|]+)["\']?', base, re.IGNORECASE)
            if m:
                fpath = m.group(1).replace("\\", "/")
                file_reads[fpath].append({"out_len": c["out_len"], "cmd": inner[:80]})

    reruns = {k: v for k, v in file_reads.items() if len(v) > 1}
    return reruns

def main():
    logs = sorted(LOG_DIR.glob("task*.jsonl"))
    all_pollution = []
    all_blasts = []
    all_reruns = {}
    all_context_growth = {}

    for log in logs:
        if log.stat().st_size == 0:
            continue

        events = parse_log(log)
        cmds = extract_cmds(events)
        task_id = log.stem

        print(f"\n{'='*70}")
        print(f"PASS 2: {task_id}")
        print(f"{'='*70}")

        # Context growth
        turns = analyze_context_growth(events)
        all_context_growth[task_id] = turns
        print(f"\n[Context Growth]")
        for i, t in enumerate(turns, 1):
            cache_hit_pct = t["cached_tokens"] / max(1, t["input_tokens"]) * 100
            print(f"  Turn {i}: in={t['input_tokens']:,} (cache={t['cached_tokens']:,}, {cache_hit_pct:.0f}%) cumulative_output={t['cumulative_output_bytes']:,}B")

        # Pollution analysis
        pollution = analyze_pollution(cmds)
        all_pollution.extend([(task_id, p) for p in pollution])
        print(f"\n[Search Pollution]")
        for p in pollution:
            print(f"  {p['bytes']:,}B | {p['noise_pct']:.0f}% noise ({p['noise_lines']}/{p['total_lines']} lines)")
            print(f"    Cmd: {p['cmd']}")
            print(f"    Noise: {p['noise_types']}")

        # Vocabulary blast (OR searches)
        blasts = analyze_or_searches(cmds)
        all_blasts.extend([(task_id, b) for b in blasts])
        print(f"\n[Vocabulary OR Blasts (>=3 terms)]")
        for b in blasts:
            print(f"  {len(b['terms'])} terms -> {b['bytes_returned']:,}B")
            print(f"    Terms: {b['terms']}")

        # Re-reads
        reruns = analyze_reruns(cmds)
        all_reruns[task_id] = reruns
        print(f"\n[File Re-reads]")
        if reruns:
            for fpath, reads in reruns.items():
                total_bytes = sum(r["out_len"] for r in reads)
                print(f"  {fpath}: read {len(reads)}x = {total_bytes:,}B total")
        else:
            print("  None")

    # Cross-run summary
    print(f"\n\n{'='*70}")
    print(f"CROSS-RUN PATTERNS (Pass 2)")
    print(f"{'='*70}")

    # P2-A: Context accumulation rate
    print(f"\nP2-A: Context Accumulation Rate")
    for task_id, turns in all_context_growth.items():
        if turns:
            growth = turns[-1]["input_tokens"] - turns[0]["input_tokens"] if len(turns) > 1 else 0
            print(f"  {task_id}: {turns[0]['input_tokens']:,} -> {turns[-1]['input_tokens']:,} tokens ({growth:,} growth)")

    # P2-B: Pollution rate
    print(f"\nP2-B: Average Search Pollution Rate")
    if all_pollution:
        avg_noise = sum(p["noise_pct"] for _, p in all_pollution) / len(all_pollution)
        high_noise = [(tid, p) for tid, p in all_pollution if p["noise_pct"] > 20]
        print(f"  Avg noise in unfiltered searches: {avg_noise:.1f}%")
        print(f"  High-noise searches (>20%): {len(high_noise)}")
        for tid, p in high_noise:
            print(f"    {tid}: {p['noise_pct']:.0f}% noise, {p['bytes']:,}B total")
            print(f"      {p['cmd']}")

    # P2-C: Vocabulary blast frequency
    print(f"\nP2-C: Vocabulary OR Blast Frequency")
    print(f"  Total multi-term searches: {len(all_blasts)}")
    if all_blasts:
        avg_terms = sum(b["count"] for _, b in all_blasts) / len(all_blasts)
        avg_bytes = sum(b["bytes_returned"] for _, b in all_blasts) / len(all_blasts)
        print(f"  Avg OR terms per blast: {avg_terms:.1f}")
        print(f"  Avg bytes returned per blast: {avg_bytes:,.0f}")
        max_blast = max(all_blasts, key=lambda x: x[1]["bytes_returned"])
        print(f"  Worst blast: {max_blast[1]['bytes_returned']:,}B from {max_blast[1]['count']} terms ({max_blast[0]})")

    # P2-D: Re-read cascade depth
    print(f"\nP2-D: File Re-read Cascades")
    total_reread_files = sum(len(v) for v in all_reruns.values())
    if total_reread_files > 0:
        for task_id, reruns in all_reruns.items():
            if reruns:
                total_wasted = sum(sum(r["out_len"] for r in reads) for reads in reruns.values())
                print(f"  {task_id}: {len(reruns)} files re-read, ~{total_wasted:,}B re-injected")
                for fpath, reads in reruns.items():
                    print(f"    {fpath}: {len(reads)}x reads")
    else:
        print("  No significant re-reads detected")

    # P2-E: Effectiveness of first search (does it immediately find the right file?)
    print(f"\nP2-E: First Search Relevance")
    print(f"  (Manual review needed — see per-task logs above)")

    # P2-F: Does the model iterate narrowingly or broadly?
    print(f"\nP2-F: Search Term Breadth Trajectory (per task)")
    for task_id, turns in all_context_growth.items():
        if turns and len(turns) > 1:
            first_tok = turns[0]["input_tokens"]
            last_tok = turns[-1]["input_tokens"]
            ratio = last_tok / first_tok if first_tok > 0 else 1
            print(f"  {task_id}: {ratio:.1f}x token expansion over {len(turns)} turns")


if __name__ == "__main__":
    main()
