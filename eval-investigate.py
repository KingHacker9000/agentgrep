import subprocess, json, os

AG = r"C:\Dev\Tools\agentgrep\target\release\agentgrep.exe"
BASE = r"C:\Dev\Tools\agentgrep\eval-worktree"

def find_top5(repo, query):
    r = subprocess.run([AG, "find", query, "--json"], capture_output=True, text=True,
                       cwd=os.path.join(BASE, repo), timeout=15)
    data = json.loads(r.stdout)
    return data.get("candidates", [])[:5]

# Investigate misses
cases = [
    ("flask", "error handler"),
    ("express", "route handler request response"),
    ("bat", "syntax highlighting theme"),
    ("bat", "line number output"),
    ("fd", "file type filter"),
    ("fd", "regex search pattern"),
    ("ripgrep", "output color config"),
]

for (repo, query) in cases:
    print(f"\n--- {repo}: '{query}' ---")
    for c in find_top5(repo, query):
        ev = [e["type"] for e in c.get("evidence", [])]
        print(f"  {c['score']:.2f}  {c['path']}")
        print(f"        ev: {ev[:4]}")
