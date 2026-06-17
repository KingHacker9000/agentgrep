# Evaluation Result Template

Copy this file for each evaluation run. Save to `manual-test/<repo-name>/result-<task-slug>.md`.

Fill in every field. Leave a field blank only if it genuinely does not apply, and note why.

---

## Run metadata

| Field | Value |
|---|---|
| **Date** | YYYY-MM-DD |
| **Repo** | repo name and URL or local path |
| **Repo size** | approximate lines of code and primary language |
| **Task category** | feature-localization / exact-error-lookup / symbol-tracing / impact-check / refactor-prep |
| **Task prompt** | the exact prompt used |
| **Agent** | agent name or "manual" if run by hand |
| **Mode** | A (rg baseline) / B (no index) / C (indexed) / D (future) |
| **Index freshness** | fresh / stale / not built / not applicable |
| **Agentgrep version** | output of `agentgrep --version` |

---

## Commands run

List every command in the order it was run. Include flags.

```
agentgrep find "..." --json
agentgrep map src/...
agentgrep symbol ...
agentgrep related ...
agentgrep blast ...
```

Total commands: N

---

## Top results

Paste the top 3–5 candidates from the first command output (or summarize if output is large).

```
1. path/to/file.rs  (score: X, why: ...)
2. path/to/other.rs (score: X, why: ...)
3. path/to/third.rs (score: X, why: ...)
```

---

## Files opened

List the files the agent actually read (in order) before reaching the correct answer.

```
1. path/to/file.rs
2. path/to/other.rs (opened but not the answer)
```

---

## Outcome

| Metric | Value |
|---|---|
| **First-file hit** | yes / no |
| **Top-3 hit** | yes / no |
| **Unnecessary file opens** | N (files opened before the correct one) |
| **Command count** | N |
| **Task completion quality** | 3 (complete) / 2 (partial) / 1 (failed) |
| **JSON parse error** | yes / no |
| **Blast false negative** | yes / no / not applicable |

---

## Notes

Free-text notes about what worked well or what was confusing. Keep brief.

---

## Failure category

If task completion quality is 1 or 2, record the primary failure category:

```
wrong-top-result
missing-in-results
noisy-evidence
missing-symbol
bad-next-action
incorrect-risk
stale-index
json-break
slow-command
blast-false-negative
```

Primary failure: [category here]

Secondary failure (if any): [category here]

---

## Raw output

Optionally attach or link the full JSON output for the first command.

Save to `manual-test/<repo-name>/<task-slug>-raw.json`.
