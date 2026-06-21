import subprocess, json, sys, os

AG = r"C:\Dev\Tools\agentgrep\target\release\agentgrep.exe"
BASE = r"C:\Dev\Tools\agentgrep\eval-worktree"

benchmarks = [
    # (repo, query, expected_top_path_fragment)
    ("flask", "request context", "ctx.py"),
    ("flask", "application context push pop", "ctx.py"),
    ("flask", "blueprint registration", "blueprints"),
    ("flask", "template rendering", "templating"),
    ("flask", "error handler", "app.py"),
    ("express", "middleware chain next", "router"),
    ("express", "route handler request response", "router"),
    ("express", "application mount", "application.js"),
    ("bat", "syntax highlighting theme", "theme"),
    ("bat", "line number output", "printer"),
    ("fd", "file type filter", "filter"),
    ("fd", "regex search pattern", "pattern"),
    ("ripgrep", "searcher builder", "search"),
    ("ripgrep", "output color config", "color"),
]

results = []
for (repo, query, expected) in benchmarks:
    repo_path = os.path.join(BASE, repo)
    try:
        r = subprocess.run(
            [AG, "find", query, "--json"],
            capture_output=True, text=True, cwd=repo_path, timeout=15
        )
        data = json.loads(r.stdout)
        candidates = data.get("candidates", [])
        note = data.get("note", "")
        top = candidates[0] if candidates else None
        top_score = top["score"] if top else 0.0
        top_path = top["path"] if top else "(none)"
        top_conf = top["confidence"] if top else "-"
        hit = expected.lower() in top_path.lower() if top else False
        results.append({
            "repo": repo, "query": query, "expected": expected,
            "top_score": top_score, "top_path": top_path,
            "top_conf": top_conf, "hit": hit, "note": note,
            "count": len(candidates)
        })
    except Exception as e:
        results.append({"repo": repo, "query": query, "error": str(e)})

print("\n=== BENCHMARK RESULTS ===\n")
hits = 0
total = 0
for r in results:
    if "error" in r:
        print(f"  ERROR {r['repo']}: {r['query']} -> {r['error']}")
        continue
    total += 1
    marker = "HIT " if r["hit"] else "MISS"
    if r["hit"]:
        hits += 1
    score_str = f"{r['top_score']:.2f}"
    note_str = " [MISMATCH_NOTE]" if r["note"] else ""
    print(f"  {marker} {r['repo']:10} [{score_str}] {r['top_path'][:60]}{note_str}")
    if not r["hit"]:
        print(f"       expected: {r['expected']}")

print(f"\n  Hit@1: {hits}/{total} ({100*hits//total}%)")

# Also test symbol dotted-query on flask
print("\n=== DOTTED SYMBOL QUERY: flask RequestContext.push ===")
try:
    r = subprocess.run(
        [AG, "symbol", "RequestContext.push", "--json"],
        capture_output=True, text=True,
        cwd=os.path.join(BASE, "flask"), timeout=15
    )
    data = json.loads(r.stdout)
    for m in data.get("matches", [])[:3]:
        sym = m["symbol"]
        parent = sym.get("parent_class", "(none)")
        print(f"  {sym['name']} in {sym['file_path']}:{sym['line_number']} [parent={parent}]")
    print(f"  match_mode: {data.get('match_mode', '?')}")
except Exception as e:
    print(f"  ERROR: {e}")

# Test vocabulary mismatch note
print("\n=== VOCAB MISMATCH NOTE: flask 'xyzzy frobnicate' ===")
try:
    r = subprocess.run(
        [AG, "find", "xyzzy frobnicate", "--json"],
        capture_output=True, text=True,
        cwd=os.path.join(BASE, "flask"), timeout=15
    )
    data = json.loads(r.stdout)
    print(f"  candidates: {len(data.get('candidates', []))}")
    note = data.get("note", "")
    print(f"  note: {note[:120] if note else '(none)'}")
except Exception as e:
    print(f"  ERROR: {e}")
