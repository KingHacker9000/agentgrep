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

Invoke-Step 'cargo fmt --check' { cargo fmt --check }
Invoke-Step 'cargo check' { cargo check }
Invoke-Step 'cargo test' { cargo test }

Invoke-Step 'cargo run -- --help' {
    cargo run -- --help | Out-File -FilePath (Join-Path $manualTestDir 'smoke-help-root.txt') -Encoding utf8
}

Invoke-Step 'cargo run -- find --help' {
    cargo run -- find --help | Out-File -FilePath (Join-Path $manualTestDir 'smoke-help-find.txt') -Encoding utf8
}

Invoke-Step 'cargo run -- index --help' {
    cargo run -- index --help | Out-File -FilePath (Join-Path $manualTestDir 'smoke-help-index.txt') -Encoding utf8
}

Invoke-Step 'cargo run -- map --help' {
    cargo run -- map --help | Out-File -FilePath (Join-Path $manualTestDir 'smoke-help-map.txt') -Encoding utf8
}

Invoke-Step 'cargo run -- symbol --help' {
    cargo run -- symbol --help | Out-File -FilePath (Join-Path $manualTestDir 'smoke-help-symbol.txt') -Encoding utf8
}

Invoke-Step 'cargo run -- related --help' {
    cargo run -- related --help | Out-File -FilePath (Join-Path $manualTestDir 'smoke-help-related.txt') -Encoding utf8
}

Invoke-Step 'cargo run -- blast --help' {
    cargo run -- blast --help | Out-File -FilePath (Join-Path $manualTestDir 'smoke-help-blast.txt') -Encoding utf8
}

# Functional self-test: run against the agentgrep repo itself

Invoke-Step 'cargo run -- --version' {
    cargo run -- --version | Out-File -FilePath (Join-Path $manualTestDir 'smoke-version.txt') -Encoding utf8
    Get-Content (Join-Path $manualTestDir 'smoke-version.txt') | Write-Host
}

Invoke-Step 'cargo run -- index' {
    cargo run -- index | Out-File -FilePath (Join-Path $manualTestDir 'smoke-index.txt') -Encoding utf8
    Get-Content (Join-Path $manualTestDir 'smoke-index.txt') | Write-Host
}

Invoke-Step 'cargo run -- index --status' {
    cargo run -- index --status | Out-File -FilePath (Join-Path $manualTestDir 'smoke-index-status.txt') -Encoding utf8
    Get-Content (Join-Path $manualTestDir 'smoke-index-status.txt') | Write-Host
}

Invoke-Step 'cargo run -- find "SearchResult" --json' {
    cargo run -- find 'SearchResult' --json | Out-File -FilePath (Join-Path $manualTestDir 'smoke-find-searchresult.json') -Encoding utf8
    Write-Host "find output written to smoke-find-searchresult.json"
}

Invoke-Step 'cargo run -- map src/rank.rs' {
    cargo run -- map src/rank.rs | Out-File -FilePath (Join-Path $manualTestDir 'smoke-map-rank.txt') -Encoding utf8
    Get-Content (Join-Path $manualTestDir 'smoke-map-rank.txt') | Write-Host
}

Write-Host ""
Write-Host "Smoke complete. Outputs in: $manualTestDir"
