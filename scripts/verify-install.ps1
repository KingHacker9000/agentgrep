$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir '..')
Set-Location $repoRoot

$manualTestDir = Join-Path $repoRoot 'manual-test'
if (-not (Test-Path $manualTestDir)) {
    New-Item -ItemType Directory -Path $manualTestDir | Out-Null
}

$outFile = Join-Path $manualTestDir 'verify-install.txt'
$lines = [System.Collections.Generic.List[string]]::new()

function Log {
    param([string]$msg)
    Write-Host $msg
    $lines.Add($msg)
}

function LogSection {
    param([string]$title)
    Log ""
    Log "==> $title"
}

$passed = 0
$failed = 0

function Pass {
    param([string]$msg)
    Log "  [PASS] $msg"
    $script:passed++
}

function Fail {
    param([string]$msg)
    Log "  [FAIL] $msg"
    $script:failed++
}

# --- 1. Locate installed binary ---
LogSection 'Locate installed binary'

$installedPath = $null
try {
    $installedPath = (Get-Command agentgrep -ErrorAction Stop).Source
    Pass "installed binary found: $installedPath"
} catch {
    Fail "agentgrep not found on PATH. Run: cargo install --path . --force"
}

# --- 2. Installed binary version ---
LogSection 'Installed binary version'

$installedVersion = $null
if ($installedPath) {
    try {
        $installedVersion = (& agentgrep --version 2>&1) | Select-Object -First 1
        Pass "agentgrep --version: $installedVersion"
    } catch {
        Fail "agentgrep --version failed: $_"
    }
}

# --- 3. Dev binary version (cargo run) ---
LogSection 'Dev binary version (cargo run)'

$devVersion = $null
try {
    # cargo writes progress to stderr; stdout carries only the version line.
    # Do not redirect stderr — let it flow to the terminal to avoid PS 5.1
    # NativeCommandError wrapping that trips $ErrorActionPreference = 'Stop'.
    $devVersion = cargo run -- --version | Where-Object { $_ -match '^agentgrep ' } | Select-Object -First 1
    if ($devVersion) {
        Pass "cargo run -- --version: $devVersion"
    } else {
        Fail "cargo run -- --version produced no version line on stdout"
    }
} catch {
    Fail "cargo run -- --version failed: $_"
}

# --- 4. Compare versions ---
LogSection 'Version match check'

if ($installedVersion -and $devVersion) {
    if ($installedVersion -eq $devVersion) {
        Pass "installed and dev versions match: $installedVersion"
    } else {
        Fail "version mismatch: installed='$installedVersion'  dev='$devVersion'"
        Log "  Fix: cargo install --path . --force"
    }
} else {
    Log "  (skipped - one or both versions unavailable)"
}

# --- 5. Functional checks using installed binary ---
LogSection 'Functional: agentgrep index'

if ($installedPath) {
    try {
        $indexOut = agentgrep index 2>&1
        $indexOut | Out-File -FilePath (Join-Path $manualTestDir 'verify-index.txt') -Encoding utf8
        if ($indexOut -match 'files indexed') {
            Pass "index built successfully"
        } else {
            Fail "index output did not contain 'files indexed'"
            $indexOut | ForEach-Object { Log "  $_" }
        }
    } catch {
        Fail "agentgrep index failed: $_"
    }
}

LogSection 'Functional: agentgrep index --status'

if ($installedPath) {
    try {
        $statusOut = agentgrep index --status 2>&1
        $statusOut | Out-File -FilePath (Join-Path $manualTestDir 'verify-index-status.txt') -Encoding utf8
        if ($statusOut -match 'fresh|stale') {
            Pass "index --status returned a recognized state"
        } else {
            Fail "index --status output not recognized"
            $statusOut | ForEach-Object { Log "  $_" }
        }
    } catch {
        Fail "agentgrep index --status failed: $_"
    }
}

LogSection 'Functional: agentgrep find "SearchResult" --json'

if ($installedPath) {
    try {
        $findOut = agentgrep find 'SearchResult' --json 2>&1
        $findOut | Out-File -FilePath (Join-Path $manualTestDir 'verify-find.json') -Encoding utf8
        $json = $findOut | ConvertFrom-Json
        if ($json.PSObject.Properties['candidates'] -and $json.PSObject.Properties['query']) {
            Pass "find --json returned valid JSON with 'candidates' and 'query' fields"
        } else {
            Fail "find --json output missing expected fields"
        }
    } catch {
        Fail "agentgrep find failed or returned invalid JSON: $_"
    }
}

LogSection 'Functional: agentgrep map src/rank.rs'

if ($installedPath) {
    try {
        $mapOut = agentgrep map src/rank.rs 2>&1
        $mapOut | Out-File -FilePath (Join-Path $manualTestDir 'verify-map-rank.txt') -Encoding utf8
        if ($mapOut -match 'Symbols|role:') {
            Pass "map src/rank.rs returned file card output"
        } else {
            Fail "map output not recognized"
            $mapOut | Select-Object -First 5 | ForEach-Object { Log "  $_" }
        }
    } catch {
        Fail "agentgrep map failed: $_"
    }
}

# --- 6. Shell completions check ---
LogSection 'Shell completions: agentgrep completions bash'

if ($installedPath) {
    try {
        $completions = agentgrep completions bash 2>&1
        if ($completions -match '_agentgrep|agentgrep') {
            Pass "completions bash generated output"
        } else {
            Fail "completions bash output not recognized"
        }
    } catch {
        Fail "agentgrep completions bash failed: $_"
    }
}

# --- Summary ---
Log ""
Log "=============================="
Log "Result: $passed passed, $failed failed"
Log "=============================="
Log "Outputs in: $manualTestDir"

$lines | Out-File -FilePath $outFile -Encoding utf8

if ($failed -gt 0) {
    Write-Host ""
    Write-Host "VERIFICATION FAILED. See above for details." -ForegroundColor Red
    exit 1
}
