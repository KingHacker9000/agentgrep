# Evaluation Metrics

This document defines two complementary metric families:

1. **Automated retrieval metrics** — computed by `scripts/analyze-eval.py` from
   captured command output and labeled tasks. These power the public benchmark
   (see [BENCHMARKS.md](./BENCHMARKS.md)) and require no human judgment per run.
2. **Manual reviewer metrics** — measurable by a human inspecting command output,
   used for qualitative review and for task types the automated harness does not
   yet score (`map`, `symbol`, `related`, `blast` graph quality).

The automated metrics are the ones reported in published benchmark tables. The
manual metrics remain useful for diagnosing *why* a number is low.

> **Report every metric broken down by task type.** A tool can be excellent at
> exact error lookup and weak at feature localization; a single aggregate hides
> that. The analyzer always groups by repo, task type, and mode. Aggregate-only
> numbers are not acceptable benchmark claims.

---

## Automated retrieval metrics

These are computed against a ranked list of file paths produced by each mode
(see [TASK_SCHEMA.md](./TASK_SCHEMA.md) for the mode output schema) and the
labeled relevant files for the task. Unless stated otherwise, ranking position
is 1-based.

Label-to-relevance mapping used by the metrics below:

| Label | Counts as a "hit"? | Graded gain (nDCG) | In relevant set (P/R)? |
|---|---|---|---|
| `primary` | yes | 3 | yes |
| `acceptable` | yes | 2 | yes |
| `supporting` | no | 1 | yes |
| `irrelevant` | no | 0 | no |
| unlabeled | no | 0 | no |

A **hit** means a `primary` or `acceptable` file — a file that genuinely answers
the task. `supporting` files are relevant context but do not by themselves
satisfy the task, so they count toward Precision/Recall/nDCG but not Hit@k/MRR.

### Hit@1, Hit@3, Hit@8

**Definition:** fraction of tasks where a hit (primary/acceptable) appears within
the top 1, top 3, or top 8 ranked files respectively.

**Why it matters:** agents read top candidates first. Hit@1 is the strongest
single signal; Hit@3 and Hit@8 capture "found it within a few opens."

### MRR (Mean Reciprocal Rank)

**Definition:** mean of `1 / rank_of_first_hit` across tasks. If no hit appears
in the ranked list, the task contributes 0.

**Why it matters:** rewards putting the right file high without collapsing to a
binary cutoff.

### nDCG@8 (normalized Discounted Cumulative Gain)

**Definition:** DCG@8 using graded gains (primary=3, acceptable=2, supporting=1),
normalized by the ideal DCG@8 for the task's labels. Gain discount is
`gain / log2(rank + 1)`.

**Why it matters:** graded, rank-sensitive quality measure. Distinguishes "right
file at rank 1" from "right file at rank 7" and credits useful supporting files.

### Precision@8

**Definition:** of the top 8 ranked files, the fraction that are in the relevant
set (primary/acceptable/supporting).

**Why it matters:** measures noise. A high Hit@8 with low Precision@8 means the
agent still wades through junk.

### Recall@8

**Definition:** of all relevant files labeled for the task, the fraction that
appear in the top 8 ranked files.

**Why it matters:** for refactor/impact tasks the agent needs *all* the sites,
not just one. Recall@8 captures coverage.

### Unnecessary files before first hit

**Definition:** number of ranked files above the first hit (i.e.
`rank_of_first_hit - 1`). If there is no hit in the list, record as a miss
(excluded from the mean, counted in a separate `miss` tally).

**Why it matters:** each file above the answer is a wasted context read. This is
the concrete cost metric an agent owner cares about.

### JSON parse success

**Definition:** fraction of mode runs whose `--json` output parsed as valid JSON.
Mode A (rg) is parsed as rg's JSON-lines; modes B/C/D as the agentgrep `find`
contract.

**Why it matters:** an agent's tool call fails entirely on unparseable output.
**Target: 100%.** Any parse failure is a blocking bug, not a quality issue.

### Latency p50 / p95

**Definition:** median and 95th-percentile wall-clock latency per mode, in
milliseconds, measured by the harness around each command invocation. Report
index build time separately from per-query time.

**Why it matters:** retrieval that is accurate but slow still costs the agent.
p95 surfaces tail behavior that a mean hides.

---

## Semantic metrics (Mode D vs Mode C)

Semantic mode (`--semantic`, Mode D) is only worth shipping if it helps more than
it hurts. These metrics are computed by comparing each task's Mode D result
against its Mode C result on the **same task and repo**. They are meaningless in
isolation — always paired C↔D.

### Semantic-only helpful hit rate

**Definition:** fraction of tasks where a hit (primary/acceptable) appears in
Mode D's top 8 but **not** in Mode C's top 8. These are wins that only semantic
retrieval delivered.

**Why it matters:** this is the entire upside case for semantic mode. If it is
near zero, semantic mode is not earning its complexity.

### Bad promotion rate

**Definition:** fraction of tasks where an `irrelevant`-labeled file appears in
Mode D's top 8 but not in Mode C's top 8 (semantic pulled in noise).

**Why it matters:** semantic recall can surface plausible-but-wrong files. This
is the primary downside to watch.

### Exact-query regression rate

**Definition:** fraction of tasks (especially `exact-error-lookup`) where Mode C
had Hit@1 but Mode D did **not** — i.e. semantic expansion demoted an
exact-match answer.

**Why it matters:** semantic mode must never degrade the deterministic strength
of exact matches. **Target: 0.** Any regression here is a design failure, since
deterministic evidence is supposed to dominate ranking.

### Semantic latency overhead

**Definition:** per-task latency delta `latency(D) - latency(C)`, reported as p50
and p95 deltas. Report semantic index build time separately.

**Why it matters:** quantifies the runtime cost the helpful-hit rate must justify.

---

## Manual reviewer metrics

These metrics are measurable by a human reviewer inspecting command output,
without requiring automated harness infrastructure. They cover graph-quality
dimensions the automated retrieval metrics do not score.

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
- **Do not report Mode D (semantic) results in isolation.** Mode D is experimental; only report it paired against Mode C on the same tasks (see the semantic metrics above), never as a standalone claim.
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
