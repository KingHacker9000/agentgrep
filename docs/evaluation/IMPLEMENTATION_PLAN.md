# Agentgrep Implementation Plan

Based on dogfood findings (see DOGFOOD_FINDINGS.md). Each phase has exact file locations, exact change descriptions, and predicted outcomes.

---

## Phase 1 — Make Scores Meaningful

**Goal**: Scores in `[0.0, 1.0]`, discriminating between strong (#1) and weak (#8) results.

### Root Cause

`src/rank.rs:1277–1310` — `finalize_ranked_candidate`:

```rust
// BUG: ranked.candidate.score is ALREADY clamped to [0,1] at line 441.
// Adding tier*0.15 pushes it to 1.15–1.75+.
if tier > 0 {
    let tier_score = tier as f64 * 0.15;
    ranked.candidate.score = round_score((ranked.candidate.score + tier_score).max(0.0));  // <-- broken
    ranked.candidate.confidence = if has_index_definition { Confidence::High } ...
}
```

The base signals in `build_candidate` also accumulate past 1.0 before clamping. A file with 3 term matches, a filename shape match, an exact phrase, a source role, and a symbol definition can easily accumulate 2.0+ before the clamp at line 441 swallows all discrimination.

### Changes

**1a. `src/rank.rs` — Budget-based score accumulation in `build_candidate`**

Replace the flat additive signals with a budgeted accumulator. Instead of adding to one `score` variable, track sub-budgets:

```rust
struct ScoreAccumulator {
    lexical: f64,      // filename tokens, path tokens, snippet terms, BM25 — cap 0.30
    phrase: f64,       // exact phrase, near phrase — cap 0.25
    symbol_def: f64,   // indexed_symbol_definition — cap 0.30
    reference: f64,    // indexed_symbol_reference + indexed_edge — cap 0.10
    role: f64,         // source/doc/test/config role bonus — cap 0.05
}

impl ScoreAccumulator {
    fn total(&self) -> f64 {
        (self.lexical.min(0.30)
            + self.phrase.min(0.25)
            + self.symbol_def.min(0.30)
            + self.reference.min(0.10)
            + self.role.min(0.05))
        .clamp(0.0, 1.0)
    }
}
```

Route each signal type to its bucket:
- `collect_token_matches` → `lexical` bucket
- `filename_shape_boost` → `lexical` bucket (counted toward 0.30 cap)
- `apply_lex_score` (BM25) → `lexical` bucket
- `exact_phrase_boost` / `near_phrase_boost` → `phrase` bucket
- `symbol_definition_signal` → `symbol_def` bucket
- `symbol_reference_signal` + `edge_signal` → `reference` bucket
- `apply_role_weight` → `role` bucket

**Implementation detail**: This requires changing `build_candidate` to use `ScoreAccumulator` internally and compute the final score at the end. The `raw_score` stored in `RankedCandidate` can be `accumulator.total()` before any penalty, which is used for sort tiebreaking.

**1b. `src/rank.rs:1277–1310` — Remove post-clamp tier addition**

In `finalize_ranked_candidate`:
- Delete lines 1296–1297 (the `tier_score` addition after clamp)
- Keep `tier` only for sort-order tiebreaking (already done at line 195–198 in the sort comparator)

The tier is already used in the sort comparator (`has_shape_tier_evidence`) and in the `raw_score` tiebreak. Removing the additive tier bonus doesn't lose ordering power — it just stops inflating scores.

**1c. `src/rank.rs:1298–1306` — Remove confidence override in `finalize_ranked_candidate`**

Currently: any file with `has_index_definition` gets forced to `Confidence::High` by `finalize_ranked_candidate`, regardless of what `confidence_for` computed.

Remove this override. Let `confidence_for` (which already checks evidence quality, phrase matches, and score thresholds) determine confidence on its own. After budget-based scoring, `score >= 0.60` will be a real threshold.

**1d. `src/rank.rs` — Evidence deduplication before output**

In `build_candidate`, before returning, deduplicate the evidence list:

```rust
// Deduplicate evidence by (evidence_type, detail) pair
let mut seen = BTreeSet::new();
evidence.retain(|e| seen.insert((e.evidence_type.clone(), e.detail.clone())));
```

Insert after line ~438, before building `RankedCandidate`.

**1e. `src/types.rs:100–103` — Update `SearchCoverage.finalize` signature**

`coverage.candidate_limit` in `SearchCoverage` hardcodes 8. After Phase 3 removes the limit, this field should reflect the actual limit (or `usize::MAX` when unlimited). Change in Phase 3 — note here for coordination.

### Tests to add

- `scores_stay_in_0_to_1_range`: Assert that finalized candidate scores are always in `[0.0, 1.0]` after introducing a file with multiple overlapping signals
- `confidence_calibration`: Assert source file with exact phrase AND symbol definition gets `high`; file with only rg_match gets `low`
- `evidence_no_duplicates`: Assert no two evidence items have identical `(type, detail)` pair

### Predicted Outcome

| Scenario | Before | After |
|---|---|---|
| Flask sessions.py (session query) | 1.30 / high | 0.82 / high |
| Flask app.py (session query) | 1.15 / low | 0.38 / medium |
| bat decorations.rs (line number query) | 1.30 / high | 0.79 / high |
| bat TypeScript fixture (#3) | 1.30 / high | 0.35 / low |
| ripgrep glue.rs (multi-line query) | 1.30 / high | 0.76 / high |
| Express test/app.use.js (middleware query) | 0.94 / medium | 0.68 / medium |
| Express lib/application.js (no symbol def) | ~0.80 / medium | 0.55 / medium |

**Key win**: bat TypeScript fixture drops from 1.30 to 0.35 — agent now knows to skip it. Flask app.py drops from 1.15 to 0.38 — agent knows sessions.py is the primary file. Score delta between #1 and #4 becomes meaningful (0.79 vs 0.35) rather than 0 (1.30 vs 1.30).

**Express ranking inversion not fixed by Phase 1 alone** — `lib/application.js` still lacks a symbol definition signal, so it stays below the test file. Fix comes in Phase 2.

---

## Phase 2 — Fix Symbol Extraction (Python + JavaScript)

**Goal**: `agentgrep symbol "push"` finds `RequestContext.push` in Flask; `agentgrep symbol "use"` finds `lib/application.js:194` in Express.

### 2a. Python: Class Method Extraction (`src/parser/python.rs`)

**Current behavior** (line 97–111): `class_definition` adds the class as a symbol and stops. The class body (the `block` node) is never visited.

**Change**: After pushing the class symbol, walk the class body to extract methods.

```rust
"class_definition" => {
    if let Some(name) = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
    {
        facts.symbols.push(symbol(
            name.to_string(),
            SymbolKind::Struct,
            file_path,
            node.start_position().row + 1,
            python_visibility(name),
            symbol_signature(source, node.start_position().row + 1, 120),
        ));
        // NEW: walk the class body to extract methods
        if let Some(body) = node.child_by_field_name("body") {
            walk_python_class_body(body, file_path, source, facts);
        }
    }
}
```

New function `walk_python_class_body`:

```rust
fn walk_python_class_body(
    body: Node,
    file_path: &str,
    source: &str,
    facts: &mut FileFacts,
) {
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                {
                    facts.symbols.push(symbol(
                        name.to_string(),
                        SymbolKind::Function,
                        file_path,
                        child.start_position().row + 1,
                        python_visibility(name),
                        symbol_signature(source, child.start_position().row + 1, 120),
                    ));
                }
            }
            "decorated_definition" => {
                // @staticmethod, @classmethod, @property — recurse into the inner definition
                for inner in named_children(child) {
                    if inner.kind() == "function_definition" {
                        if let Some(name) = inner
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                        {
                            facts.symbols.push(symbol(
                                name.to_string(),
                                SymbolKind::Function,
                                file_path,
                                inner.start_position().row + 1,
                                python_visibility(name),
                                symbol_signature(source, inner.start_position().row + 1, 120),
                            ));
                        }
                    }
                }
            }
            _ => {}  // Skip class variables, pass, nested classes for now
        }
    }
}
```

**Note on naming**: Methods are stored with just the method name (e.g., `push`, `pop`), not `RequestContext.push`. This means `agentgrep symbol "push"` will find all `push` methods across all classes. The `signature` field (e.g., `def push(self) -> None:`) and `file_path`+`line_number` give enough context to distinguish them. We do NOT need a `parent_class` field in `IndexedSymbol` for this to be useful.

**Edge case**: Nested classes (class inside class) — skip for now. Properties and class variables (not `function_definition`) — skip. Async methods have the same tree-sitter node kind `function_definition` so they work automatically.

**Test to add** in `src/parser/python.rs`:
```rust
#[test]
fn extracts_class_methods() {
    let source = r#"
class RequestContext:
    def push(self) -> None:
        pass
    def pop(self, exc=None):
        pass
    @staticmethod
    def from_environ(environ):
        pass
"#;
    let facts = extract_file_facts("ctx.py", source, &lookup(&["ctx.py"]));
    assert!(facts.symbols.iter().any(|s| s.name == "RequestContext"));
    assert!(facts.symbols.iter().any(|s| s.name == "push"));
    assert!(facts.symbols.iter().any(|s| s.name == "pop"));
    assert!(facts.symbols.iter().any(|s| s.name == "from_environ"));
}
```

### 2b. JavaScript: Prototype Assignment Extraction (`src/parser/javascript.rs`)

**Current behavior**: The `_` catch-all walks children of unrecognized nodes. An `expression_statement` is walked, its child `assignment_expression` is walked, but neither its `left` (member_expression) nor `right` (function_expression) match any handled kind, so nothing is extracted.

**Add a new match arm for `expression_statement`**:

```rust
"expression_statement" => {
    // Check for prototype/module method assignment: obj.method = function [name]() {}
    for child in named_children(node) {
        if child.kind() == "assignment_expression" {
            extract_js_assignment_symbol(child, file_path, source, exported, facts);
        }
    }
}
```

New function `extract_js_assignment_symbol`:

```rust
fn extract_js_assignment_symbol(
    node: Node,       // assignment_expression
    file_path: &str,
    source: &str,
    exported: bool,
    facts: &mut FileFacts,
) {
    let Some(left) = node.child_by_field_name("left") else { return };
    let Some(right) = node.child_by_field_name("right") else { return };

    // Only extract when right side is a function
    let kind = match right.kind() {
        "function_expression" | "generator_function_expression" => SymbolKind::Function,
        "arrow_function" => SymbolKind::Function,
        "class_expression" => SymbolKind::Struct,
        _ => return,
    };

    // Get the name: prefer the function expression's own name, fall back to
    // the property name from the left-side member_expression
    let name: Option<&str> = if right.kind() != "arrow_function" {
        // function_expression may have an optional name node
        right.child_by_field_name("name")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
    } else {
        None
    };

    let property_name: Option<&str> = if left.kind() == "member_expression" {
        left.child_by_field_name("property")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
    } else if left.kind() == "subscript_expression" {
        // obj["method"] = function() {} — skip (dynamic key)
        None
    } else {
        left.utf8_text(source.as_bytes()).ok()
    };

    let symbol_name = name.or(property_name).unwrap_or("").trim();
    if symbol_name.is_empty() || symbol_name.contains('.') {
        // If name still contains '.', it's something like `app.settings.use` — skip
        return;
    }

    facts.symbols.push(symbol(
        symbol_name.to_string(),
        kind,
        file_path,
        node.start_position().row + 1,
        js_visibility(symbol_name, exported),
        symbol_signature(source, node.start_position().row + 1, 120),
    ));
}
```

**Test to add** in `src/parser/javascript.rs`:
```rust
#[test]
fn extracts_prototype_assignment() {
    let source = r#"
app.use = function use(fn) {
    return this;
};
proto.handle = function handle(req, res, next) {};
const router = function router() {};
"#;
    let facts = extract_file_facts("lib/application.js", source,
        &lookup(&["lib/application.js"]));
    assert!(facts.symbols.iter().any(|s| s.name == "use"));
    assert!(facts.symbols.iter().any(|s| s.name == "handle"));
}
```

### 2c. Rebuild indexes and verify

After implementing 2a and 2b:
```bash
cd eval-worktree/flask && agentgrep index
agentgrep symbol "push"   # expect: ctx.py:367 RequestContext.push
agentgrep symbol "pop"    # expect: ctx.py:396 RequestContext.pop
agentgrep map src/flask/ctx.py  # expect: shows push, pop methods

cd eval-worktree/express && agentgrep index
agentgrep symbol "use"   # expect: lib/application.js:194
agentgrep find "middleware registration use"  # expect: lib/application.js at #1
```

### Index schema version

`src/index.rs:22`: `INDEX_SCHEMA_VERSION` must be bumped from `6` → `7` since the index now contains more symbols. Old indexes will be detected as schema-mismatch and will need to be rebuilt via `agentgrep index`.

### Predicted Outcome

| Scenario | Before | After |
|---|---|---|
| Flask Mode C commands (push/pop task) | 6 | **3** (find → symbol → peek or map) |
| Express `agentgrep find "middleware registration"` | test file #1 | lib/application.js #1 |
| Express `agentgrep symbol "use"` | Matches: none | lib/application.js:194 |
| Flask `agentgrep symbol "push"` | Matches: none | ctx.py:367 |
| Flask `agentgrep map ctx.py` | 7 symbols (classes only) | 30+ symbols (classes + methods) |

**Universality impact**: After Phase 2, agentgrep works structurally on Rust, Python, ES5/CommonJS JS, ES6 TypeScript, and Go (partially). The same `find`→`symbol`→`map` workflow becomes reliable across these languages. For Ruby/PHP/Java/C++: graceful degradation — `find` still works, `symbol`/`map`/`peek` are absent, agent falls back to rg within the file.

---

## Phase 3 — Remove Hard Candidate Limit, Add Tiered Density

**Goal**: Agents see the full ranked list; token budget controlled by compressing low-scoring results.

### Changes

**3a. `src/rank.rs:14`** — Remove `CANDIDATE_LIMIT: usize = 8`

Replace with a soft cap that only limits the `enum` tier to prevent flooding on huge repos:
```rust
pub const CANDIDATE_ENUM_LIMIT: usize = 50;  // hard floor: never more than 50 enum-tier results
```

**3b. `src/types.rs:16–25`** — Add `detail_level` to `FileCandidate`

```rust
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailLevel {
    Full,    // score >= 0.70: snippets + evidence + next_actions
    Medium,  // score >= 0.45: snippets only (no evidence)
    Minimal, // score >= 0.25: path + role + score + confidence only
    Enum,    // score <  0.25: path only
}

pub struct FileCandidate {
    pub path: String,
    pub kind: String,
    pub role: String,
    pub score: f64,
    pub confidence: Confidence,
    pub detail_level: DetailLevel,       // NEW
    pub line_ranges: Vec<LineRange>,
    pub snippets: Vec<Snippet>,
    pub evidence: Vec<Evidence>,
}
```

**3c. `src/rank.rs`** — Compute `detail_level` per candidate

After scoring is finalized, before returning the Vec:
```rust
fn assign_detail_level(score: f64) -> DetailLevel {
    if score >= 0.70 { DetailLevel::Full }
    else if score >= 0.45 { DetailLevel::Medium }
    else if score >= 0.25 { DetailLevel::Minimal }
    else { DetailLevel::Enum }
}
```

When serializing, strip snippets and evidence from non-Full candidates:
- `Medium`: keep `snippets`, clear `evidence`
- `Minimal`: clear `snippets` and `evidence`
- `Enum`: clear everything except `path`, `score`, `detail_level`

This can be done either in the serializer (by conditionally skipping fields based on `detail_level`) or by materializing it before output in `src/main.rs`.

**3d. `src/rank.rs:211`** — Remove `.take(CANDIDATE_LIMIT)` from the final collect

Replace:
```rust
.take(CANDIDATE_LIMIT)
.collect()
```
With:
```rust
.collect::<Vec<_>>()
// Then apply CANDIDATE_ENUM_LIMIT only to enum-tier results:
// truncate to enum_limit + count of non-enum results
```

Concretely: take all `Full/Medium/Minimal` tier results + up to 50 `Enum` tier results.

**3e. `src/types.rs:100–103`** — Update `SearchCoverage.finalize`

`candidate_limit` field: set to the effective limit used, or `usize::MAX` to signal "unlimited". Since the JSON field exists today as a stable field, change it to report the actual enum ceiling (50) rather than the old 8.

### Predicted Outcome

| Scenario | Before | After |
|---|---|---|
| Flask "render template" — candidates shown | 8 (limited: true) | ~12 (8 Full/Medium + 4 Enum) |
| bat "line number" — visible range | 8 | 20+ |
| ripgrep "multi line search" — last result detail | same as #1 | path+score only (Enum tier) |
| Agent decision quality | Must guess within 8 | Can see full ranked list and reason |

**Universality impact**: Improves universality for large repos. A monorepo with 1,000+ rg hits no longer gets silently truncated to 8. Enum-tier items are cheap (one line each) so token cost is controlled. Agent can skip Enum-tier results unless they have no better options.

**Risk**: For very broad queries ("import" in any JS codebase → 1,000 files), the Enum tier could still produce 50 one-line items. The agent skill docs (Phase 6) should teach agents to use more specific queries rather than single-token queries.

---

## Phase 4 — Peek Command

**Goal**: `agentgrep peek <file> <symbol-or-line>` returns the body of a named symbol without reading the whole file.

### 4a. `src/types.rs:359–367`** — Add `end_line` to `IndexedSymbol`

```rust
pub struct IndexedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line_number: usize,
    pub end_line: Option<usize>,   // NEW — None for languages without tree-sitter
    pub visibility: Visibility,
    pub signature: Option<String>,
}
```

JSON contract: `end_line` is best-effort (nullable), not stable. Add `#[serde(default, skip_serializing_if = "Option::is_none")]`.

### 4b. Populate `end_line` in each parser

In each tree-sitter parser's symbol extraction, change:
```rust
facts.symbols.push(symbol(
    name.to_string(),
    SymbolKind::Function,
    file_path,
    node.start_position().row + 1,
    ...
));
```
To:
```rust
facts.symbols.push(symbol_with_end(
    name.to_string(),
    SymbolKind::Function,
    file_path,
    node.start_position().row + 1,
    Some(node.end_position().row + 1),   // end_line
    ...
));
```

Update the `symbol()` constructor in `src/parser/extracted.rs` to accept `end_line: Option<usize>`.

Files to update: `src/parser/rust.rs`, `src/parser/python.rs`, `src/parser/javascript.rs`, `src/parser/typescript.rs`, `src/parser/go.rs`.

### 4c. `src/peek.rs` — New command

```rust
pub struct PeekReport {
    pub query: String,
    pub file_path: String,
    pub symbol_name: Option<String>,
    pub kind: Option<String>,
    pub start_line: usize,
    pub end_line: Option<usize>,
    pub signature: Option<String>,
    pub body: Vec<BodyLine>,
    pub truncated: bool,
    pub truncated_line_count: Option<usize>,
    pub index_status: String,
}

pub struct BodyLine {
    pub line_number: usize,
    pub text: String,
}
```

**Resolution order** (for `agentgrep peek ctx.py push`):

1. Resolve file path relative to repo root
2. Look up `index.symbols` for symbols in that file where `name` case-insensitively matches `push`
3. If multiple matches, pick the one whose `line_number` is closest to the query line (if a line was specified) or the first one
4. If `end_line` is known: read lines `[start_line, end_line]` from disk
5. If `end_line` is None (language not tree-sitter indexed): read from `start_line` and infer end by indentation heuristic (stop when indentation returns to start-line level or file ends)
6. Smart truncation: if body > 40 lines, show first 20 + last 5, with "... N lines omitted ..." separator
7. Return `PeekReport`

**Invocation forms**:
```bash
agentgrep peek src/flask/ctx.py push            # symbol in file
agentgrep peek src/flask/ctx.py:367             # line number in file
agentgrep peek RequestContext.push              # global symbol search
```

CLI parsing: if argument contains `:` with a digit suffix → line-in-file form. If argument contains `/` or `.py`/`.js`/`.rs` → file+symbol form. Otherwise → global symbol.

**What peek is NOT**: It does not call cat or display raw bytes. It uses the index to find the symbol extent and reads only those lines. For a function with 200 lines, it shows a truncated view. The agent gets the signature + call pattern without the full implementation unless the function is short.

### 4d. `src/cli.rs` + `src/main.rs` — Wire up `peek` subcommand

Add `Peek(PeekArgs)` variant to the `Commands` enum. Add `PeekArgs { query: String, json: bool }` struct. Dispatch in `main.rs`.

### 4e. Schema version

Bump `INDEX_SCHEMA_VERSION` from `7` → `8` (if Phase 2 was already at 7). Old indexes lack `end_line` on symbols. They can still be used — `peek` will use the indentation heuristic when `end_line` is None.

### Predicted Outcome

| Scenario | Before | After |
|---|---|---|
| Navigate to RequestContext.push body | cat entire ctx.py (400 lines) | peek ctx.py push (30 lines) |
| Find function body when file is large | Read 500+ line file | Read 20–40 line function body |
| Agent context tokens for "find + read one function" | ~800–1500 tokens | ~200–400 tokens |

**Universality impact**: peek requires `end_line` for best results. For unsupported languages, it falls back to indentation heuristic (works reasonably well for Python, JS, Rust, Go — all whitespace-structured). For truly arbitrary files (C with braces, Haskell, Lisp), the heuristic may produce incorrect extents. The agent skill docs should note: "peek works best in indexed repos; use rg within the file for unsupported languages."

---

## Phase 5 — Function Caller Tracking

**Goal**: `agentgrep symbol "push"` shows which functions call `push()` (the "Used by" section).

### Root Cause

`src/symbol.rs` populates `used_by` from `index.symbol_references`. But `symbol_references` only contains:
1. `import_statement` bindings (recorded by parsers as `ImportBinding`)
2. Type-level references (struct/class references)

It does NOT contain function call sites. `func_a() { push() }` never generates an `IndexedSymbolReference` entry for "push".

### Changes

**5a. Add call-site detection in each language parser**

In `src/parser/python.rs`, add a new pass that walks function bodies looking for `call` nodes:

```rust
fn extract_call_sites(
    body: Node,
    caller_file: &str,
    source: &str,
    facts: &mut FileFacts,
    context: ReferenceContext,
) {
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let callee_name = match func_node.kind() {
                    "identifier" => func_node.utf8_text(source.as_bytes()).ok(),
                    "attribute" => func_node.child_by_field_name("attribute")
                        .and_then(|n| n.utf8_text(source.as_bytes()).ok()),
                    _ => None,
                };
                if let Some(name) = callee_name {
                    facts.symbol_references.push(ImportBinding {
                        from_file: caller_file.to_string(),
                        symbol_name: name.to_string(),
                        target_file: None,  // call site — target unknown without type info
                        line_number: child.start_position().row + 1,
                        confidence: EdgeConfidence::Inferred,
                        reason: "call site".to_string(),
                    });
                }
            }
        }
        // Recurse into all child nodes
        extract_call_sites(child, caller_file, source, facts, context);
    }
}
```

This is expensive if every call is recorded. **Pragmatic constraint**: only record calls to identifiers that are 3+ characters AND not common builtins (len, str, int, print, list, dict, etc.). Use a small blocklist.

Same pattern for JS (`call_expression`) and Rust (`call_expression`).

**5b. `src/symbol.rs`** — Surface call-site references in "Used by"

The `symbol` command already reads `index.symbol_references`. After 5a, the index will contain call-site references (with `target_file: None` and `reason: "call site"`). These will naturally appear in the "Used by" section for the queried symbol.

**Display change**: Show a `[call]` tag vs `[import]` tag to differentiate:
```
Used by:
  - wsgi_app (src/flask/app.py:35)  [call]
  - test_push_pop (tests/test_ctx.py:12)  [call, test]
```

### Predicted Outcome

| Scenario | Before | After |
|---|---|---|
| `agentgrep symbol "push"` → "Used by" | "Used by: none" | wsgi_app (app.py:35), test_push_pop (test_ctx.py:12) |
| Agent traces data flow for `push` | Must use blast + rg | symbol command gives callers directly |

**Caveat**: Call-site detection is syntactic, not semantic. `push()` in Python might call `list.push`, `RequestContext.push`, or a user-defined `push` — the parser can't distinguish without type information. The `target_file: None` and `confidence: Inferred` signal this uncertainty. Agents should use this to find candidate callers, not as a definitive answer.

**Universality impact**: Works for any language with a tree-sitter grammar that has `call_expression` or `call` nodes. For languages without tree-sitter support, no change (still "Used by: none"). The signal is weak (inferred, no target file) but better than nothing.

---

## Phase 6 — Agent Skill Documentation

**Goal**: Agents know the decision tree, query formulation rules, and anti-patterns without trial and error.

### 6a. `docs/AGENT_SKILL.md` — Full skill document

Structure:

**1. What agentgrep is for**
```
agentgrep fills the gap between:
  "I know exactly where to look" → use Read/cat directly
  "I have no idea" → use agentgrep find
  "I found the file, need to navigate it" → use agentgrep peek/symbol/map
  "I know the exact string" → use rg
```

**2. Decision tree**

```
Need to find code?
├── Know the exact string/pattern to search? → rg "exact string"
├── Know the file but need a specific function? → agentgrep peek <file> <symbol>
├── Know a symbol name but not the file? → agentgrep symbol <name>
├── Concept-first search (what/where/how)? → agentgrep find "<concept phrase>"
└── Want to understand a file's structure? → agentgrep map <file>
```

**3. Query formulation rules for `find`**

GOOD (concept phrases):
```
agentgrep find "request context push pop lifecycle"    # noun phrases describing behavior
agentgrep find "session cookie secret key storage"     # nouns describing a subsystem
agentgrep find "middleware registration order chain"   # describe the concept, not the code
```

BAD (exact strings → use rg instead):
```
agentgrep find "def push"          # rg "def push" is better
agentgrep find "app.use"           # rg "app\.use" is better
agentgrep find "import flask"      # rg "import flask" is better
```

IDENTIFIER rule: If your query is a known symbol name (CamelCase or snake_case), use `agentgrep symbol` not `agentgrep find`:
```
agentgrep symbol "RequestContext"    # better than agentgrep find "RequestContext"
agentgrep symbol "use"               # search symbol index for 'use'
```

**4. Interpreting output (after Phase 1)**
```
score >= 0.70: Strong evidence — open this file first
score 0.45–0.70: Likely relevant — open after primary files
score 0.25–0.45: Weak signal — scan list, open if alternatives exhausted
score < 0.25: Low confidence — probably noise, skip unless top result is absent
```

**5. Anti-patterns**
- Don't stop at `find` — use `symbol`, `map`, `peek` to navigate within results
- Don't trust "Used by: none" for function callers until Phase 5 ships (use `blast` instead)
- Don't use `agentgrep find` for exact string searches (rg is faster and more precise)
- Don't use `agentgrep peek` on unsupported languages (falls back to heuristic)
- If `find` returns the right file but you can't find the function with `symbol`, use `rg "def funcname" <file>`

**6. Typical workflow patterns**

*Pattern A: Concept-first discovery*
```bash
agentgrep find "session push pop lifecycle"    # → ctx.py at 0.85
agentgrep symbol "push"                        # → ctx.py:367 RequestContext.push
agentgrep peek ctx.py push                     # → 30-line method body
```

*Pattern B: Known symbol, unknown file*
```bash
agentgrep symbol "RequestContext"              # → ctx.py:287
agentgrep map src/flask/ctx.py                 # → see all methods and edges
agentgrep blast ctx.py                         # → understand impact before changing
```

*Pattern C: Impact analysis*
```bash
agentgrep blast src/flask/ctx.py               # → which files import/use this
agentgrep related src/flask/ctx.py             # → full neighborhood graph
```

### 6b. Condensed system-prompt snippet

A separate `docs/AGENT_SYSTEM_PROMPT.md` with ≤ 500 tokens:

```markdown
## agentgrep — codebase search

Commands: find <concept> | symbol <name> | map <file> | peek <file> <symbol> | blast <file>

When to use:
- find: concept-first search ("session storage mechanism"). Use natural language, not exact code.
- symbol: known name, unknown file ("RequestContext", "push")  
- map: understand file structure (symbols, edges, callers)
- peek: read one function body without opening the whole file
- blast: understand what would break if you change a file/symbol

When NOT to use agentgrep:
- Exact string search → use rg
- Already know the file → use Read
- Language not supported → use rg within the file

Score guide: 0.8+ = open first, 0.5–0.8 = likely useful, <0.5 = weak signal
```

### Predicted Outcome (qualitative)

| Metric | Before Phase 6 | After Phase 6 |
|---|---|---|
| Mode C avg commands per task | 6 | 3–4 |
| "wrong direction" turns | 1–2 per task | 0–1 per task |
| Agent falls back to rg after agentgrep | frequently | only for unsupported languages |
| Time-to-correct-answer | 6 commands × N ms | 3 commands × N ms |

---

## Universality Assessment

**Definition**: agentgrep is "universal" if it works meaningfully on any codebase regardless of language, size, or structure, and degrades gracefully rather than silently failing.

### After Each Phase

| Phase | Universality Change |
|---|---|
| Phase 1 (scores) | **Improves for all languages**: scores become comparable across repos. An agent can apply the same 0.70/0.45/0.25 thresholds in a Rust repo and a Python repo. Currently the same threshold is meaningless because everything is 1.30. |
| Phase 2 (Python/JS extraction) | **Extends structural nav to the 2 biggest non-Rust language families**. After this: Rust (full), Python (full), ES5/CommonJS JS (good), TypeScript/ES6 (good), Go (partial). Ruby/Java/PHP/C/C++: no structural nav — graceful degradation. |
| Phase 3 (no hard limit) | **Improves for large repos**: a 50K-file monorepo no longer silently truncates to 8 results. Tiered density keeps token cost manageable. |
| Phase 4 (peek) | **Language-gated**: peek with `end_line` works on all indexed languages. Indentation heuristic works for Python, JS, Rust. Fails unpredictably for Lisp/Ruby/C brace-terminated code. Graceful degradation: tell agent "use rg within the file instead." |
| Phase 5 (call sites) | **Weakly universal**: call detection in Python, JS, Rust. No target_file → inferred confidence. Signal is always partial (can't distinguish overloaded names). |
| Phase 6 (docs) | **Language-neutral**: decision tree and query rules apply regardless of language. Docs explicitly tell agents what to fall back to for unsupported languages. |

### Universality Non-Goals

These are explicitly deferred and do not affect the core design:

- **Type-aware call resolution**: Knowing that `ctx.push()` calls `RequestContext.push` (not some other `push`) requires type inference. That needs a language server or compiler, not tree-sitter. Agentgrep will never guarantee this.
- **Cross-language symbol tracking**: A Python function calling a Rust FFI function. Out of scope.
- **Generated code**: Files generated by protobuf, sqlc, etc. Already handled via `FileRole::Generated`.
- **Languages without tree-sitter grammars**: The `rg`-backed `find` command still works. File-level search is always universal. Only symbol navigation is language-gated.

### Final Universality Verdict

After all 6 phases, agentgrep retains universality in the defined sense:

- **`find` works everywhere** (rg-backed, no language dependency)
- **`symbol`, `map`, `related`, `blast`** work for Rust, Python, JS/TS, Go — the major languages in large open-source and enterprise codebases
- **`peek`** works for all indexed languages, degrades to heuristic for others
- **Scores and confidence labels** are calibrated and comparable across repos
- **Agent skill docs** give explicit fallback paths ("use rg within the file") for unsupported languages

The tool is not universal in the sense of "equally powerful on every language" — it never will be without a full compiler frontend. It IS universal in the sense of "useful on any codebase, with known and documented limits."
