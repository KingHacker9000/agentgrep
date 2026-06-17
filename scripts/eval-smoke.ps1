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

Write-Host ''
Write-Host "Eval smoke complete. Outputs in: $manualTestDir"
