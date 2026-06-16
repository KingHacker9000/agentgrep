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
