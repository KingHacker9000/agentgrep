$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir '..')
Set-Location $repoRoot

$manualTestDir = Join-Path $repoRoot 'manual-test'
if (-not (Test-Path $manualTestDir)) {
    New-Item -ItemType Directory -Path $manualTestDir | Out-Null
}

function Invoke-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [scriptblock]$Action
    )
    Write-Host "==> $Name"
    & $Action
}

# Check that evaluation docs exist
Invoke-Step 'check: docs/evaluation/README.md' {
    if (-not (Test-Path (Join-Path $repoRoot 'docs\evaluation\README.md'))) {
        throw 'docs/evaluation/README.md is missing'
    }
    Write-Host 'ok'
}

Invoke-Step 'check: docs/evaluation/TASKS.md' {
    if (-not (Test-Path (Join-Path $repoRoot 'docs\evaluation\TASKS.md'))) {
        throw 'docs/evaluation/TASKS.md is missing'
    }
    Write-Host 'ok'
}

Invoke-Step 'check: docs/evaluation/METRICS.md' {
    if (-not (Test-Path (Join-Path $repoRoot 'docs\evaluation\METRICS.md'))) {
        throw 'docs/evaluation/METRICS.md is missing'
    }
    Write-Host 'ok'
}

Invoke-Step 'check: docs/evaluation/RESULT_TEMPLATE.md' {
    if (-not (Test-Path (Join-Path $repoRoot 'docs\evaluation\RESULT_TEMPLATE.md'))) {
        throw 'docs/evaluation/RESULT_TEMPLATE.md is missing'
    }
    Write-Host 'ok'
}

# Run a few agentgrep commands against this repo and save output to manual-test/
# These are the same manual checks called out in the milestone spec.

Invoke-Step 'agentgrep index' {
    cargo run -- index | Out-File -FilePath (Join-Path $manualTestDir 'eval-smoke-index.txt') -Encoding utf8
    Get-Content (Join-Path $manualTestDir 'eval-smoke-index.txt') | Write-Host
}

Invoke-Step 'agentgrep find "first-file hit rate" --json' {
    cargo run -- find 'first-file hit rate' --json |
        Out-File -FilePath (Join-Path $manualTestDir 'eval-smoke-find-metric.json') -Encoding utf8
    Write-Host 'output written to eval-smoke-find-metric.json'
}

Invoke-Step 'agentgrep find "rg baseline" --json' {
    cargo run -- find 'rg baseline' --json |
        Out-File -FilePath (Join-Path $manualTestDir 'eval-smoke-find-rg-baseline.json') -Encoding utf8
    Write-Host 'output written to eval-smoke-find-rg-baseline.json'
}

Invoke-Step 'agentgrep find "future semantic mode" --json' {
    cargo run -- find 'future semantic mode' --json |
        Out-File -FilePath (Join-Path $manualTestDir 'eval-smoke-find-semantic.json') -Encoding utf8
    Write-Host 'output written to eval-smoke-find-semantic.json'
}

Invoke-Step 'agentgrep map docs/evaluation/README.md' {
    cargo run -- map docs/evaluation/README.md |
        Out-File -FilePath (Join-Path $manualTestDir 'eval-smoke-map-readme.txt') -Encoding utf8
    Get-Content (Join-Path $manualTestDir 'eval-smoke-map-readme.txt') | Write-Host
}

Invoke-Step 'agentgrep map docs/evaluation/RESULT_TEMPLATE.md' {
    cargo run -- map docs/evaluation/RESULT_TEMPLATE.md |
        Out-File -FilePath (Join-Path $manualTestDir 'eval-smoke-map-template.txt') -Encoding utf8
    Get-Content (Join-Path $manualTestDir 'eval-smoke-map-template.txt') | Write-Host
}

Invoke-Step 'render-eval-report.py --help (syntax check)' {
    python (Join-Path $scriptDir 'render-eval-report.py') --help | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "render-eval-report.py --help failed (exit $LASTEXITCODE)"
    }
    Write-Host 'ok'
}

# Full report-generation smoke: create a minimal fixture, compute summary via
# analyze-eval.py (so summary.json is always correct by construction), then
# generate the HTML+Markdown report and verify all expected output files exist.
Invoke-Step 'render-eval-report: full pipeline smoke' {
    $fixtureDir  = Join-Path $repoRoot 'eval-results\_smoke-fixture'
    $parsedDir   = Join-Path $fixtureDir 'parsed'
    $labelsFile  = Join-Path $repoRoot 'docs\evaluation\labels\public-v0.1.jsonl'
    New-Item -ItemType Directory -Force -Path $parsedDir | Out-Null

    # 3-task fixture (6 records: 3 tasks × modes C and D).
    # Task layout chosen so the report demonstrates all table types:
    #   agentgrep-feat-001  C hit@1, D hit@1  → no semantic-only win
    #   agentgrep-feat-002  C miss,  D hit@1  → semantic-only WIN
    #   agentgrep-err-001   C miss,  D miss   → no-hit task in both modes
    $records = @(
        '{"run_id":"smoke","task_id":"agentgrep-feat-001","repo_id":"agentgrep","task_type":"feature-localization","mode":"C","query":"how are search result files ranked and scored","exit_code":0,"latency_ms":142,"json_parse_ok":true,"ranked_paths":["src/rank.rs","src/search.rs","src/index.rs"],"semantic_status":null,"raw_stdout_path":null,"raw_stderr_path":null,"skipped":false,"skip_reason":null}',
        '{"run_id":"smoke","task_id":"agentgrep-feat-001","repo_id":"agentgrep","task_type":"feature-localization","mode":"D","query":"how are search result files ranked and scored","exit_code":0,"latency_ms":310,"json_parse_ok":true,"ranked_paths":["src/rank.rs","src/search.rs","src/index.rs"],"semantic_status":"active","raw_stdout_path":null,"raw_stderr_path":null,"skipped":false,"skip_reason":null}',
        '{"run_id":"smoke","task_id":"agentgrep-feat-002","repo_id":"agentgrep","task_type":"feature-localization","mode":"C","query":"where is the semantic embedding provider and model configured","exit_code":0,"latency_ms":156,"json_parse_ok":true,"ranked_paths":["src/search.rs","src/rank.rs","src/index.rs"],"semantic_status":null,"raw_stdout_path":null,"raw_stderr_path":null,"skipped":false,"skip_reason":null}',
        '{"run_id":"smoke","task_id":"agentgrep-feat-002","repo_id":"agentgrep","task_type":"feature-localization","mode":"D","query":"where is the semantic embedding provider and model configured","exit_code":0,"latency_ms":320,"json_parse_ok":true,"ranked_paths":["src/semantic.rs","src/cli.rs","src/search.rs"],"semantic_status":"active","raw_stdout_path":null,"raw_stderr_path":null,"skipped":false,"skip_reason":null}',
        '{"run_id":"smoke","task_id":"agentgrep-err-001","repo_id":"agentgrep","task_type":"exact-error-lookup","mode":"C","query":"rg was not found on PATH","exit_code":0,"latency_ms":145,"json_parse_ok":true,"ranked_paths":["src/runner.rs","src/error.rs"],"semantic_status":null,"raw_stdout_path":null,"raw_stderr_path":null,"skipped":false,"skip_reason":null}',
        '{"run_id":"smoke","task_id":"agentgrep-err-001","repo_id":"agentgrep","task_type":"exact-error-lookup","mode":"D","query":"rg was not found on PATH","exit_code":0,"latency_ms":315,"json_parse_ok":true,"ranked_paths":["src/runner.rs","src/error.rs"],"semantic_status":"active","raw_stdout_path":null,"raw_stderr_path":null,"skipped":false,"skip_reason":null}'
    )
    $records | Out-File -LiteralPath (Join-Path $parsedDir 'results.jsonl') -Encoding utf8

    $metaObj = [ordered]@{
        run_id            = 'smoke'
        timestamp_utc     = '2026-06-18T00:00:00.000Z'
        agentgrep_version = 'smoke-fixture'
        semantic_enabled  = $true
        task_file         = 'docs/evaluation/tasks/public-v0.1.jsonl'
        label_file        = 'docs/evaluation/labels/public-v0.1.jsonl'
    }
    $metaObj | ConvertTo-Json | Out-File -LiteralPath (Join-Path $fixtureDir 'run-meta.json') -Encoding utf8

    # Compute summary.json with analyze-eval.py so it is always correct by
    # construction — never hand-written values that can drift from the data.
    python (Join-Path $scriptDir 'analyze-eval.py') `
        --run-dir $fixtureDir `
        --labels  $labelsFile | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "analyze-eval.py failed on smoke fixture" }

    # Generate the static report.
    python (Join-Path $scriptDir 'render-eval-report.py') `
        --run-dir $fixtureDir `
        --labels  $labelsFile | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "render-eval-report.py failed on smoke fixture" }

    # Verify expected output files exist.
    $required = @(
        (Join-Path $fixtureDir 'summary.json'),
        (Join-Path $fixtureDir 'report\index.html'),
        (Join-Path $fixtureDir 'report\report.md'),
        (Join-Path $fixtureDir 'report\assets\hit_by_mode.svg'),
        (Join-Path $fixtureDir 'report\assets\semantic_deltas.svg')
    )
    foreach ($f in $required) {
        if (-not (Test-Path $f)) { throw "Expected output missing: $f" }
    }
    Write-Host "ok (report at eval-results\_smoke-fixture\report\index.html)"
}

Write-Host ''
Write-Host "Eval smoke complete. Outputs in: $manualTestDir"
