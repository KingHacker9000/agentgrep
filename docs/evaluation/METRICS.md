# Evaluation Metrics

These metrics are designed to be measurable by a human reviewer inspecting command output, without requiring automated harness infrastructure.

---

## Primary metrics

### First-file hit rate

**Definition:** the percentage of tasks where the correct file appears as the first candidate in the result.

**How to measure:** run the task command, check whether `candidates[0].path` is the file a human would go to first.

**Why it matters:** coding agents typically read the top candidate first. A miss at position 0 costs extra context budget.

**Mode comparison:** compare Mode A (rg `-l` first match), Mode B (`find` without index, first candidate), Mode C (`find` with index, first candidate).

---

### Top-3 hit rate

**Definition:** the percentage of tasks where the correct file appears in the first three candidates.

**How to measure:** check whether the correct file appears at `candidates[0]`, `candidates[1]`, or `candidates[2]`.

**Why it matters:** a top-3 hit is still useful — the agent opens at most 3 files before finding the right one.

**Complement to first-file hit rate:** report both. A tool with 40% first-file and 90% top-3 is different from one with 40% first-file and 50% top-3.

---

### Unnecessary file opens avoided

**Definition:** the number of files above the correct file in the ranked list (i.e., files the agent would open and discard before reaching the correct one).

**How to measure:** find the position of the correct file in the candidate list. Subtract 1. If the correct file is not in the list, record as "miss."

**Why it matters:** each unnecessary open costs context tokens and latency. A tool that surfaces the right file at position 0 instead of position 4 saves the agent 4 context reads.

---

### Command count

**Definition:** the total number of Agentgrep (or rg) commands the agent runs before it has enough information to act.

**How to measure:** count commands manually during a session. A typical mode C session for a feature localization task might be: `find` → `map` → done (2 commands).

**Why it matters:** fewer commands = less latency and less prompt overhead. Mode C with graph context should require fewer follow-up commands than Mode A.

---

### Time to first useful file

**Definition:** elapsed wall time from starting the session to the moment the agent has the correct file open and can begin reading or editing.

**How to measure:** approximate, manual timing. Not automated. Note index build time separately from command time.

**Why it matters:** fast tools let agents iterate faster. Slow index builds reduce the value of Mode C for short sessions.

**Record separately:** index build time (one-time cost) vs. per-command time (per-task cost).

---

### JSON parse failures

**Definition:** the number of times `--json` output could not be parsed as valid JSON by `jq` or a standard JSON parser.

**How to measure:** pipe `agentgrep [cmd] --json` to `jq .` and check for parse errors.

**Why it matters:** agents depend on stable JSON. A parse failure means the agent's tool call fails entirely.

**Target:** zero. Any JSON parse failure is a blocking bug, not a quality issue.

---

### Blast false negatives

**Definition:** the number of times `blast` fails to list a file that was actually impacted by an edit to the target file or symbol.

**How to measure:** run `blast [target]`, then make a real edit and check which files broke. Compare the broken files against the blast output.

**Why it matters:** `blast` is a conservative likely-impact estimate. Missing a heavily-impacted file makes the estimate unsafe. False negatives are more dangerous than false positives.

**Note:** false positives (over-reporting) are expected and acceptable. Document the false negative rate, not the false positive rate.

---

### Subjective task completion quality

**Definition:** a 1–3 rating for whether the agent could complete the task using only the output from the tested mode.

| Rating | Meaning |
|---|---|
| 3 — complete | Agent found the correct file and had enough context to act without additional searches. |
| 2 — partial | Agent found the correct file but needed additional commands to get enough context. |
| 1 — failed | Agent could not find the correct file within the top results, or the output was misleading. |

**How to assign:** reviewer judgment. One reviewer per session is fine for a scaffold. Flag disagreements.

---

## What not to overclaim

- **Do not claim percentages from fewer than 10 task runs per mode.** Small samples have high variance. Report counts, not rates, when N < 10.
- **Do not compare modes A vs. C without running the same tasks on the same repos.** Different repos and tasks make comparisons meaningless.
- **Do not claim latency improvements without measuring.** Perceived speed is not the same as measured time.
- **Do not treat blast false positives as failures.** `blast` is explicitly described as a conservative estimate. A long list is expected.
- **Do not report Mode D (semantic) results.** Mode D is not yet implemented.
- **Do not conflate index freshness issues with retrieval quality issues.** If the index is stale, note it as an index issue, not a ranking failure.
- **Do not extrapolate from one repo to all repos.** Record the repo name, size, and language in every result.

---

## Failure categories

When a task does not reach rating 3, record one of these failure categories:

| Category | Description |
|---|---|
| `wrong-top-result` | A clearly unrelated file ranked above the correct one. |
| `missing-in-results` | The correct file did not appear in the output at all. |
| `noisy-evidence` | Evidence labels (`Why` field) were inaccurate or misleading. |
| `missing-symbol` | `symbol [name]` did not find the known definition. |
| `bad-next-action` | `next_actions` suggestions were unhelpful or incorrect. |
| `incorrect-risk` | `blast` risk level was clearly wrong (too high or too low). |
| `stale-index` | Index was outdated and produced stale results. |
| `json-break` | `--json` output failed to parse. |
| `slow-command` | Command took more than 5 seconds on a repo under 50k lines. |
| `blast-false-negative` | `blast` missed a file that was actually impacted. |
