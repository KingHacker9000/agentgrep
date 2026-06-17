# Evaluation Task Definitions

Tasks are designed to reflect real coding agent work, not toy examples.

Tasks are written repo-agnostic where possible. Replace bracketed terms with repo-specific names when running.

---

## Task categories

### Category 1 — Feature localization

The agent needs to find which files implement a named feature or behavior.

**Why it matters:** agents spend most context budget on finding the right file before they can read or edit it.

**Example prompts:**

1. "Find the code that handles user authentication."
2. "Where does this project process incoming webhook events?"
3. "Which files implement the retry logic for failed requests?"

**What to measure:**

- Is the correct file the top candidate?
- How many files does the agent open before finding the right one?
- Are unrelated files ranked above the correct one?

---

### Category 2 — Exact error lookup

The agent has a known error string from a log or test failure and needs to find where it originates.

**Why it matters:** exact string matches should be the strongest possible signal. A failure here indicates a basic retrieval problem.

**Example prompts:**

1. "Where is the error 'rg was not found' generated?"
2. "Find where the message 'index is stale' is produced."
3. "Which file emits the panic 'expected JSON object, got array'?"

**What to measure:**

- Does `find "<error string>"` return the right file as the first candidate?
- Is the line range in the result correct?
- Are there false positives from comment blocks or test fixtures?

---

### Category 3 — Symbol tracing

The agent needs to find all definitions and usages of a named symbol across the codebase.

**Why it matters:** before editing a type, function, or constant, an agent must know all the places it is defined and used. Missing a usage causes silent breakage.

**Example prompts:**

1. "Find all places where `[SymbolName]` is defined or called."
2. "Which files import or re-export `[TypeName]`?"
3. "Show me every file that references `[ConfigKey]`."

**What to measure:**

- Does `symbol [Name]` find the definition?
- Are production references separated from test references?
- Are there false positives in unrelated files (e.g., string literals, comments)?
- Does `related [Name]` show files that import or use it?

---

### Category 4 — Impact check before edit

The agent plans to edit a file or symbol and needs to know what else might be affected.

**Why it matters:** editing without impact awareness is the primary cause of silent regressions in agentic coding sessions.

**Example prompts:**

1. "I am about to edit `[src/important.rs]`. What else might break?"
2. "Which files depend on `[ModuleName]`?"
3. "If I change the shape of `[StructName]`, which callers need to be updated?"

**What to measure:**

- Does `blast [file-or-symbol]` list the actually-impacted files?
- Does it miss important dependents (false negatives)?
- Does it list unrelated files (false positives)?
- Is the risk level reasonable for the size of the change?

---

### Category 5 — Refactor preparation

The agent wants to rename or restructure something and needs a complete picture before starting.

**Why it matters:** a refactor that misses one usage site causes a compile error or silent bug. The agent should be able to survey the full scope before touching anything.

**Example prompts:**

1. "I want to rename `[OldName]` to `[NewName]`. Where is it defined and used?"
2. "List all the files I would need to touch to move `[ModuleName]` to a new location."
3. "I want to split `[LargeFile]` into two files. Which other files import from it?"

**What to measure:**

- Does `symbol [Name]` find all definition sites?
- Does `related [Name]` show all import/reference sites?
- Does `blast [Name]` estimate the correct scope of the refactor?
- Are the `next_actions` suggestions useful for planning the rename?

---

## Repo size guidance

### Small repos (< 5k lines)

- All five task categories apply.
- Mode B (no index) should work reasonably.
- Mode C (indexed) should be clearly better for symbol tracing and impact checks.
- Expected: fast commands (<1s), accurate top-1 hits for exact strings.

### Medium repos (5k–50k lines)

- All five task categories apply.
- Mode B will start to show ranking noise.
- Mode C ranking and graph evidence matter more.
- Expected: top-3 hits for feature localization, accurate symbol results.
- Watch for: slow index build, large JSON output, symbol extraction gaps in non-Rust files.

### Large repos (50k+ lines)

- Focus on Category 2 (exact error lookup) and Category 3 (symbol tracing) where precision matters most.
- Feature localization (Category 1) on large repos is harder — expect top-3 rather than top-1.
- Impact checks (Category 4) on large repos may have high blast false positive rates — document the rate rather than treating it as a bug.
- Expected: index build takes seconds, `blast` lists many files, agent still benefits from having a ranked list vs. raw `rg` output.
- Watch for: index size, memory usage, output truncation.
