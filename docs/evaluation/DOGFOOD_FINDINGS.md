# Agentgrep Dogfood Findings

Repos tested: flask, express, bat, fd, ripgrep (eval-worktree/).
Method: agentgrep + rg/cat on same tasks; one Codex gpt-5.4-mini agent run per mode (Mode A = rg only, Mode C = agentgrep preferred).

---

## Empirical Test: Flask Request Context Push/Pop

**Task**: Find exact function name + file where request context is pushed and popped.
**Correct answer**: `RequestContext.push()` and `RequestContext.pop()` in `src/flask/ctx.py`.

| Mode | Commands | Correct? | How |
|------|----------|----------|-----|
| A (rg only) | **2** | ✓ | `rg "def push\|def pop" ctx.py` → lines 367, 396 directly |
| C (agentgrep) | **6** | ✓ | `find` → `symbol` → `map` → fell back to `rg` and `Get-Content` |

**Mode C used 3× more tool calls and still fell back to rg.** Root cause: `RequestContext.push()` and `RequestContext.pop()` are Python class methods — they are completely absent from the index. `agentgrep symbol "push"` returns "Matches: none". The agent correctly found ctx.py at #1 with `find`, then hit a dead end when navigating within the file.

---

## Critical Failure: Express Middleware Registration (Full Inversion)

**Task**: Find where `app.use` is defined — the middleware registration entry point.
**Correct answer**: `lib/application.js:194` (`app.use = function use(fn) {...}`).

| Tool | Result |
|------|--------|
| `rg "app\.use" lib/application.js` | Line 194, immediate |
| `agentgrep find "middleware registration use"` | **#1 = test/app.use.js (WRONG), #2 = examples/ (WRONG)** |

This is a **full ranking inversion** — the test file outranks the source definition file. Root causes:
1. JS parser doesn't index `app.use = function use(fn)` (prototype assignment, not a function declaration) → lib/application.js has zero symbol-definition boost
2. `test/app.use.js` has 3 `same_area` edge boosts (test→source relationship) + many "middleware" text hits → wins on score despite being the wrong file
3. The test file's same_area edges are supposed to be "weak supporting evidence" but they dominate when the definition file has no index signals

**For this task, rg is dramatically better than agentgrep.**

---

## Systematic Results by Repo

### Flask

| Query | ag #1 | Correct? | rg breadth (tokens alone) |
|-------|--------|----------|--------------------------|
| "session cookie secret key" | src/flask/sessions.py ✓ | ✓ | 46 files for 'session', 19 for 'cookie' |
| "render template jinja" | src/flask/templating.py ✓ | ✓ | 44 files for 'render', 55 for 'template' |
| "request context push pop" | src/flask/ctx.py ✓ | Partial — file is right but methods not findable | — |

Flask finding: `find` correctly identifies the right file. But **navigating within the file fails for class methods** (`push`, `pop`, `teardown_request`). The agent must open the full file.

### bat

| Query | ag #1 | ag #3 (noise) | rg breadth |
|-------|--------|---------------|------------|
| "line number column width calculation" | src/decorations.rs ✓ | tests/syntax-tests/TypeScript/example.ts ✗ | 193 files for 'line' |
| "output colorized ANSI terminal" | src/printer.rs ✓ | — | 41 files for 'ansi', 37 for 'terminal' |

bat finding: Top result is good. **Test fixtures leak into #3** (a TypeScript example that has `width?: number` and line-related code). Score is 1.30 for ALL results — agent can't tell #1 from #3.

### ripgrep

| Query | ag #1 | Correct? |
|-------|--------|----------|
| "multi line search across newlines" | crates/searcher/src/searcher/glue.rs | ✓ |
| "after before context lines flag" | crates/printer/src/util.rs | ✓ |

ripgrep finding: Works well for concept-first queries in Rust repos (Rust symbol extraction is the most complete). Still all scores 1.30.

### Express

- `app.use`, `proto.use`, `app.get` etc. are ALL defined via prototype assignment (`obj.method = function() {}`) — **not indexed as symbols at all**
- Only ES6 class syntax is extracted by the JS parser
- agentgrep `find` returns the test file first, which is the worst possible outcome (sends the agent the wrong direction)

---

## Gap Inventory

### Gap 1 — Score Inflation (All Results Score 1.30) [CRITICAL]

**Root cause**: `finalize_ranked_candidate` adds `tier × 0.15` AFTER `score.clamp(0.0, 1.0)`. Any file with an rg match gets tier bonus pushed above 1.0. Result: every candidate in every repo shows `score 1.30, confidence high`.

**Impact**: Agents cannot distinguish between a #1 exact match and a #8 noise hit. The discrimination that should drive agent decisions is gone.

**Fix**: Budget-based score normalization. Each signal type has a hard ceiling; total stays within [0.0, 1.0].

```
lexical BM25:          max 0.30
exact phrase:          max 0.20
symbol definition:     max 0.30
symbol reference/edge: max 0.15
role bonus:            max 0.05
─────────────────────────────────
total cap:             1.00
```

Tier boost replaces the `+0.15 after clamp` with a pre-normalization weight applied to the right sub-budget. The final score must be meaningful: 0.85 = very strong signal, 0.30 = weak noise match.

### Gap 2 — Python Class Methods Not Indexed [CRITICAL]

**Root cause**: Python parser extracts top-level `def` functions and `class` definitions but does NOT walk into class bodies to extract methods.

**Evidence**: Flask `ctx.py` has 7 indexed symbols (`RequestContext`, `AppContext`, etc.) but zero methods. `agentgrep symbol "push"` → "Matches: none".

**Impact**: For any Python OOP codebase, the most important symbols (class methods) are invisible to `symbol`, `map`, and `related`. Agent must open the full file instead of using structured navigation.

**Fix**: Extend `src/parser/python.rs` to walk `class_definition` body and extract child `function_definition` nodes as `SymbolKind::Function` with the parent class as a name prefix (e.g., `RequestContext::push`).

### Gap 3 — JavaScript Prototype Assignment Not Indexed [CRITICAL]

**Root cause**: JS parser handles `function foo()` declarations and ES6 class methods but misses `obj.method = function name() {}` (assignment expression).

**Evidence**: Express `lib/application.js:194` `app.use = function use(fn)` — not indexed. `lib/router/index.js:439` `proto.use = function use(fn)` — not indexed. These are the primary API entry points of the entire framework.

**Impact**: For pre-ES6 JavaScript codebases (Express, Connect, many Node.js libs), virtually no functions are indexed. agentgrep ranks test files above source files because test files have `same_area` edge boosts that the source file lacks.

**Fix**: Extend `src/parser/javascript.rs` to detect `assignment_expression` where the right side is a `function` — extract the function name from the right-hand `function` node's optional name, or fall back to the property access chain on the left.

### Gap 4 — Candidate Hard Limit of 8 [HIGH]

**Root cause**: `CANDIDATE_LIMIT: usize = 8` at `src/rank.rs:14` hard-caps results. User confirmed this was not an intentional policy.

**Impact**: Agents see at most 8 files regardless of how many are relevant. For broad queries ("render template") where many files share code, the 8th result may still be important.

**Fix**: Remove hard cap. Replace with tiered density: return all rg-hit candidates but reduce detail as score drops.

```
score ≥ 0.70  → detail_level: "full"    (snippets + evidence + next_actions)
score ≥ 0.45  → detail_level: "medium"  (snippets only, condensed evidence)
score ≥ 0.25  → detail_level: "minimal" (path + role + score only)
score <  0.25  → detail_level: "enum"   (path only, no snippets)
```

Add a `detail_level` field to `FileCandidate` JSON. Agents can reason on the full ranked list and open only what they need. Token budget stays controlled because lower-ranked files carry less data.

### Gap 5 — Confidence Labels Meaningless (Nearly Always "high") [HIGH]

**Root cause**: `confidence_for()` assigns `Confidence::High` whenever the index has any match. The threshold is too permissive.

**Impact**: Agents can't use confidence as a signal for how much to trust a result. When confidence is "high" for everything, it conveys nothing.

**Fix**: Calibrate confidence to evidence quality:
- `high`: has indexed_symbol_definition OR exact_phrase_match AND score > 0.6
- `medium`: has rg_match with lexical score > 0.3 OR any indexed evidence
- `low`: only rg_match with weak term overlap, no index signals

### Gap 6 — Symbol "Used by: none" — Function Callers Invisible [HIGH]

**Root cause**: `symbol` command only tracks `IndexedSymbolReference` (type/struct references), not function call sites. The "Used by" section shows zero callers for most functions.

**Impact**: Agents trying to trace data flow using `agentgrep symbol <fn>` get no caller information. They must use `blast` instead, which only shows files (not function names) and has a 5-file display limit.

**Fix**: During indexing, detect function call patterns and record them as `IndexedSymbolReference` with type `call`. In Rust this means detecting `function_name(...)` call expressions in tree-sitter. In Python/JS, same pattern.

### Gap 7 — Test File Noise in Rankings [MEDIUM]

**Root cause**: Test files match many source terms AND receive `same_area` edge boosts. After score inflation (Gap 1), they appear equal to or above correct source files.

**Evidence**: bat search for "line number" returns `tests/syntax-tests/TypeScript/example.ts` at #3 (a fixture that has `width?: number`). Express search ranks test file #1.

**Fix**: Two-part fix:
1. Fix Gap 1 (score normalization) — source files with symbol definitions will naturally score higher once scoring is correct
2. Apply a role penalty to `FileRole::Test` candidates when query contains definition-like terms ("where is", "define", "implementation of")

### Gap 8 — Evidence Duplication in Output [MEDIUM]

**Root cause**: Evidence list accumulates duplicate entries when the same symbol appears in multiple evidence passes.

**Evidence**: bat decorations.rs evidence shows "defines symbol LineNumberDecoration; defines symbol LineNumberDecoration; defines symbol Decoration for LineNumberDecoration" — the first two are identical.

**Fix**: Deduplicate evidence list by `(evidence_type, detail)` pair before formatting output.

### Gap 9 — map Hides Methods (MAP_SYMBOL_DISPLAY_LIMIT = 5) [MEDIUM]

**Root cause**: `map` shows at most 5 symbols per file. For files with many top-level symbols or (once Gap 2 is fixed) many class methods, agents see a truncated view.

**Evidence**: Flask `ctx.py` `agentgrep map` shows 7 top-level symbols (class definitions) but ZERO methods from any of those classes.

**Fix**: After Gap 2 is fixed (class methods indexed), increase limit and show methods indented under their class. Add a "N more symbols" count at the bottom so agents know there's more.

### Gap 10 — No Peek Command [MEDIUM]

**Root cause**: No command exists to show a specific symbol's body without opening the whole file.

**Impact**: When an agent finds the right file and knows the symbol name (e.g., `RequestContext.push`), they must `cat` the full file to see the body. For a 400-line file this costs significant context tokens.

**Design**: 
```bash
agentgrep peek ctx.py push           # symbol name in file
agentgrep peek ctx.py:367            # line number in file  
agentgrep peek RequestContext.push   # repo-wide symbol search
```
Uses tree-sitter symbol extents. Requires adding `end_line` to `IndexedSymbol`. Returns:
- signature line
- body (smart-truncated at 40 lines for large functions, with "... N lines omitted" marker)
- calls[] extracted from index
- called_by[] extracted from index

NOT a reimplementation of cat. Leverages structural knowledge — returns just the named unit.

### Gap 11 — No Agent Skill Documentation [MEDIUM]

**Root cause**: Agents using agentgrep are told "it's a search tool" but not given a mental model for when and how to use it vs rg/cat.

**Impact**: Agents in Mode C (given agentgrep access) used it as a supplement to rg rather than as the primary search tool. The agent ran 4 agentgrep commands and then 2 rg commands for a task that rg could do in 2 commands.

**Design**: A skill document that explains:
1. **Decision tree**: Use agentgrep when you need to find WHERE something is conceptually. Use rg when you already know a precise string. Use cat/peek when you know which file to read.
2. **Query formulation rules**: Use noun phrases describing what the code does, not exact tokens. "request context push" not "def push". Identifier-like queries (CamelCase) get extra expansion.
3. **Output interpretation**: Score > 0.7 = high confidence, open it. Score 0.4–0.7 = likely relevant. Score < 0.4 = weak signal, scan for alternatives. `next_actions` tells you what to run next.
4. **Anti-patterns**: Don't use agentgrep to find exact strings (use rg). Don't trust "Used by: none" (Gap 6). Don't stop at `find` — use `symbol` and `map` to navigate within the results.

### Gap 12 — No-Index Mode: Missing Identifier Expansion [LOW]

**Root cause**: Without index, `find` uses rg with token splitting but no identifier expansion. Query "RequestContext" doesn't expand to "request_context" or "request context".

**Fix**: Cheap identifier dictionary: split CamelCase/snake_case tokens, add as additional rg patterns. Already partially implemented via `tokenize_terms` in `search.rs` — verify it handles CamelCase → multi-word expansion properly.

---

## What Works Well (Don't Change)

1. **File discovery for conceptual queries**: Flask session → sessions.py, Flask template → templating.py, bat ANSI → printer.rs, ripgrep multi-line → glue.rs. All correct at #1. This is the core value proposition and it works.

2. **rg as recall floor**: No false negatives on file discovery. The right file always appears somewhere in results.

3. **Evidence trail**: Per-result evidence is the right design. Agents can see why a file was ranked.

4. **Next actions**: "Next: agentgrep symbol X" is useful and agents do follow these.

5. **JSON contract**: Clean, stable. The agent in Mode C parsed it correctly.

6. **Rust symbol extraction**: Fully functional — impl block methods, function signatures, visibility all correct.

---

## Where agentgrep wins vs rg/cat

| Scenario | Winner | Why |
|----------|--------|-----|
| Concept-first file discovery (session management, template rendering) | **agentgrep** | rg requires knowing the exact term; agentgrep expands and ranks |
| Finding a specific identifier (def push, app.use) | **rg** | Direct, 1 command, no index required |
| Navigating within a file | **agentgrep peek** (not yet built) | Once peek exists, can show just the function body with calls |
| Understanding file relationships | **agentgrep map/related** | rg has no graph awareness |
| Finding callers of a function | **agentgrep blast** (currently) / **agentgrep symbol** (after Gap 6 fix) | rg returns all text matches including comments/strings |
| Understanding impact of changing a file | **agentgrep blast** | rg can't reason about import graphs |
| OOP-heavy Python/pre-ES6 JS | **rg** (currently) | Missing class method indexing (Gap 2/3) |

---

## Prioritized Implementation Plan

### Phase 1 — Make Scores Meaningful (Unblocks Everything Else)

**Target**: Scores in [0.0, 1.0], discriminating between #1 and #8.

1. Fix score normalization in `src/rank.rs` — budget-based model (Gap 1)
2. Calibrate confidence thresholds (Gap 5)
3. Deduplicate evidence lists (Gap 8)
4. Add role penalty for test files when definition-seeking (Gap 7, partial)

**Validation**: Run `agentgrep find "request context push"` in Flask — check that ctx.py is clearly above test files by score, and score < 1.0.

### Phase 2 — Fix Symbol Extraction for Python and JavaScript

**Target**: Push/pop visible in Flask index; app.use visible in Express index.

5. Python class method extraction in `src/parser/python.rs` (Gap 2)
6. JS prototype assignment extraction in `src/parser/javascript.rs` (Gap 3)
7. Rebuild eval-worktree indexes, rerun Flask Mode A vs Mode C comparison

**Validation**: `agentgrep symbol "push"` in Flask returns `RequestContext::push` at ctx.py:367. `agentgrep symbol "use"` in Express returns lib/application.js:194.

### Phase 3 — Remove Hard Candidate Limit, Add Tiered Density

**Target**: Agents see the full ranked list with less data at lower ranks.

8. Remove `CANDIDATE_LIMIT = 8` from `src/rank.rs` (Gap 4)
9. Implement tiered density — `detail_level` field in `FileCandidate` (Gap 4)
10. Update JSON contract doc

### Phase 4 — Peek Command

**Target**: `agentgrep peek <file> <symbol>` returns just the function body.

11. Add `end_line: Option<usize>` to `IndexedSymbol` in `src/index.rs`
12. Populate `end_line` during tree-sitter extraction in each parser
13. Implement `src/peek.rs` command — symbol lookup, line-range read, smart truncation
14. Add `peek` subcommand to `src/cli.rs` and `src/main.rs`
15. Document JSON shape in `docs/JSON_CONTRACT.md`

### Phase 5 — Fix Function Call References

**Target**: `agentgrep symbol "push"` shows which functions call push().

16. Detect function call sites in tree-sitter passes (Gap 6)
17. Store as `IndexedSymbolReference` with type `call`
18. Update `symbol` output "Used by" section

### Phase 6 — Agent Skill Documentation

**Target**: Agents know exactly how to use agentgrep effectively without trial and error.

19. Write `docs/AGENT_SKILL.md` — decision tree, query formulation rules, output interpretation, anti-patterns
20. Write condensed "system prompt snippet" version for agents to include in context
21. Ensure the skill doc is indexed by agentgrep itself (dogfood)

---

## Raw Test Data

### Mode A (rg only) — Codex gpt-5.4-mini, Flask push/pop task
```
Turn 1: rg -n "def push\(|def pop\(|class RequestContext" src\flask\ctx.py
Turn 2: rg -n -C 2 "ctx\.push\(|ctx\.pop\(" src\flask\app.py
Answer: RequestContext.push at ctx.py:367, RequestContext.pop at ctx.py:396 ✓
Total: 2 commands, 21 JSONL events
```

### Mode C (agentgrep preferred) — Codex gpt-5.4-mini, Flask push/pop task
```
Turn 1: agentgrep find "request context push" --include "src/**/*.py" --match any
Turn 2: agentgrep find "request context pop" --include "src/**/*.py" --match any
Turn 3: agentgrep symbol RequestContext
Turn 4: agentgrep map src/flask/ctx.py
Turn 5: rg -n "def (push|pop)\b|RequestContext|request context" src/flask/ctx.py src/flask/app.py
Turn 6: Get-Content src/flask/ctx.py | Select-Object -Index (360..410)
Answer: RequestContext.push at ctx.py:367, RequestContext.pop at ctx.py:396 ✓
Total: 6 commands, 43 JSONL events
```

**Observation**: Mode C found the right file (ctx.py) on Turn 1. But because push/pop aren't in the index, the agent needed 4 more turns to navigate within the file. Mode A went straight to the definition with `rg "def push"` in 2 turns.

After Phase 2 (class method indexing), Mode C should reduce to 3–4 turns:
1. `agentgrep find "request context push"` → ctx.py
2. `agentgrep symbol "RequestContext.push"` → ctx.py:367, signature, callers
3. `agentgrep peek ctx.py:RequestContext.push` → full method body
