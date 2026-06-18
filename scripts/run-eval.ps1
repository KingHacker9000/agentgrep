<#
.SYNOPSIS
    Run the Agentgrep public retrieval benchmark across modes A-D.

.DESCRIPTION
    Clones (or reuses) the public repos named in a manifest, checks out their
    pinned commits, and runs each task in four modes:

        Mode A  rg baseline               (rg --json "<query>")
        Mode B  agentgrep find, no index  (agentgrep find "<query>" --json)
        Mode C  agentgrep find, indexed   (agentgrep index; find --json)
        Mode D  agentgrep find, semantic  (index --semantic; find --semantic --json)

    For every (task, mode) run it captures stdout, stderr, exit code, and
    wall-clock latency. Raw streams go under eval-results/<run-id>/raw/ and a
    structured record per run is appended to eval-results/<run-id>/parsed/results.jsonl.

    Mode D is SKIPPED unless -EnableSemantic is passed AND a semantic index can
    be built for the repo. This keeps the default run pure-deterministic and
    offline-model-free.

    See docs/evaluation/BENCHMARKS.md and docs/evaluation/TASK_SCHEMA.md.

.PARAMETER RepoManifest
    Path to the repo manifest JSONL. Default: docs/evaluation/public-repos.jsonl

.PARAMETER TaskFile
    Path to the task JSONL. Default: docs/evaluation/tasks/public-v0.1.jsonl

.PARAMETER LabelFile
    Path to the label JSONL. Recorded in run-meta.json; metrics are computed
    later by scripts/analyze-eval.py. Default: docs/evaluation/labels/public-v0.1.jsonl

.PARAMETER OutDir
    Root output directory. A per-run subdir <run-id> is created under it.
    Default: eval-results

.PARAMETER WorktreeDir
    Where public repos are cloned. Default: eval-worktree

.PARAMETER RunId
    Identifier for this run (the output subdir name). Default: timestamp.

.PARAMETER AgentgrepBin
    Path to the agentgrep binary. If omitted, uses 'agentgrep' on PATH, else
    builds target/release/agentgrep with cargo.

.PARAMETER Repo
    Comma-separated list of repo IDs to run (e.g. "agentgrep,ripgrep"). May be
    specified multiple times. When omitted, all repos with tasks are run.

.PARAMETER OnlyLabeled
    Restrict the task set to tasks that have at least one entry in LabelFile.
    Also skips repo checkout/indexing for repos that have no labeled tasks
    after filtering. Implies reading the label file before the run loop.

.PARAMETER EnableSemantic
    Enable Mode D. Builds a semantic index per repo (downloads the embedding
    model on first use). Without this switch, Mode D is skipped.

.PARAMETER Help
    Print this help and exit.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/run-eval.ps1 -Help

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/run-eval.ps1 `
      -RepoManifest docs/evaluation/public-repos.jsonl `
      -TaskFile docs/evaluation/tasks/public-v0.1.jsonl `
      -LabelFile docs/evaluation/labels/public-v0.1.jsonl

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/run-eval.ps1 -Repo agentgrep,ripgrep

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/run-eval.ps1 -OnlyLabeled

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts/run-eval.ps1 -Repo ripgrep -OnlyLabeled
#>
[CmdletBinding()]
param(
    [string]$RepoManifest = 'docs/evaluation/public-repos.jsonl',
    [string]$TaskFile = 'docs/evaluation/tasks/public-v0.1.jsonl',
    [string]$LabelFile = 'docs/evaluation/labels/public-v0.1.jsonl',
    [string]$OutDir = 'eval-results',
    [string]$WorktreeDir = 'eval-worktree',
    [string]$RunId = (Get-Date -Format 'yyyy-MM-dd-HHmmss'),
    [string]$AgentgrepBin = '',
    [string[]]$Repo = @(),
    [switch]$OnlyLabeled,
    [switch]$EnableSemantic,
    [switch]$Help
)

if ($Help) {
    Get-Help -Detailed $MyInvocation.MyCommand.Path
    exit 0
}

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir '..')).Path
Set-Location $repoRoot

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

function Read-Jsonl {
    param([string]$Path)
    if (-not (Test-Path $Path)) { throw "File not found: $Path" }
    $items = @()
    foreach ($line in Get-Content -LiteralPath $Path) {
        $trim = $line.Trim()
        if ($trim.Length -eq 0) { continue }
        $items += ($trim | ConvertFrom-Json)
    }
    return $items
}

function Assert-RequiredProps {
    # Drops rows that are null or missing any required property; warns per dropped row.
    # Returns only rows that have every required property — safe to access under StrictMode.
    param(
        [object[]]$Items,
        [string[]]$Required,
        [string]$Source
    )
    $valid = @()
    foreach ($item in $Items) {
        if ($null -eq $item) {
            Write-Warning "$($Source): dropping null row."
            continue
        }
        $propNames = $item.PSObject.Properties.Name
        $missing = @($Required | Where-Object { $propNames -notcontains $_ })
        if ($missing.Count -gt 0) {
            Write-Warning "$($Source): dropping row missing field(s): $($missing -join ', '). Row: $($item | ConvertTo-Json -Compress -Depth 2)"
            continue
        }
        $valid += $item
    }
    return $valid
}

function Get-JsonProp {
    # Safely reads a named property from a PSCustomObject without throwing under
    # Set-StrictMode -Version Latest. Returns $null if the object is null or the
    # property does not exist.
    #
    # Checks two layers:
    #   1. $Object.PSObject.Properties  — normal path for fresh PSCustomObjects.
    #   2. $Object.PSObject.BaseObject.PSObject.Properties — PS 5.1 wraps objects
    #      in a PSObject shell when they pass through certain pipeline stages; the
    #      NoteProperties from ConvertFrom-Json end up on the inner BaseObject.
    param([object]$Object, [string]$Name)
    if ($null -eq $Object) { return $null }
    # Layer 1: top-level PSObject (covers most cases).
    if ($Object.PSObject.Properties.Name -contains $Name) {
        return $Object.PSObject.Properties[$Name].Value
    }
    # Layer 2: unwrap one PSObject shell (covers pipeline-wrapped objects).
    $base = $Object.PSObject.BaseObject
    if ($null -ne $base -and -not [object]::ReferenceEquals($base, $Object)) {
        if ($base.PSObject.Properties.Name -contains $Name) {
            return $base.PSObject.Properties[$Name].Value
        }
    }
    return $null
}

function Get-RelPath {
    # Normalize to repo-relative forward-slash path.
    param([string]$Path)
    if ($null -eq $Path) { return $null }
    $p = $Path -replace '\\', '/'
    $p = $p -replace '^\./', ''
    return $p
}

function Resolve-AgentgrepBin {
    param([string]$Explicit)
    if ($Explicit -and (Test-Path $Explicit)) { return (Resolve-Path $Explicit).Path }
    $onPath = Get-Command 'agentgrep' -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    Write-Host '==> agentgrep not on PATH; building release binary (cargo build --release)...'
    cargo build --release | Out-Null
    $candidates = @(
        (Join-Path $repoRoot 'target/release/agentgrep.exe'),
        (Join-Path $repoRoot 'target/release/agentgrep')
    )
    foreach ($c in $candidates) { if (Test-Path $c) { return (Resolve-Path $c).Path } }
    throw 'Could not locate built agentgrep binary under target/release.'
}

function Invoke-Capture {
    # Runs an executable in $WorkDir, capturing stdout/stderr to files and
    # measuring wall-clock latency. Returns a result object.
    param(
        [string]$FilePath,
        [string[]]$Arguments,
        [string]$WorkDir,
        [string]$OutFile,
        [string]$ErrFile
    )
    # PS 5.1 Start-Process does not quote ArgumentList elements that contain
    # spaces, so a multi-word query would be split into separate args. Quote
    # any arg with whitespace (or empty) before handing it over.
    $quoted = @($Arguments | ForEach-Object {
            if ($_ -eq '' -or $_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
        })
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $exit = $null
    try {
        $proc = Start-Process -FilePath $FilePath -ArgumentList $quoted `
            -WorkingDirectory $WorkDir -NoNewWindow -PassThru -Wait `
            -RedirectStandardOutput $OutFile -RedirectStandardError $ErrFile
        $exit = $proc.ExitCode
    }
    catch {
        $exit = -1
        Set-Content -LiteralPath $ErrFile -Value "harness: failed to launch: $($_.Exception.Message)" -Encoding utf8
    }
    $sw.Stop()
    return [pscustomobject]@{
        ExitCode  = $exit
        LatencyMs = [int]$sw.Elapsed.TotalMilliseconds
    }
}

function Parse-FindRanking {
    # Reads agentgrep find --json output; returns @{ ok; paths; semantic }.
    param([string]$OutFile)
    $result = @{ ok = $false; paths = @(); semantic = $null }
    if (-not (Test-Path $OutFile)) { return $result }
    $raw = Get-Content -LiteralPath $OutFile -Raw
    if (-not $raw -or $raw.Trim().Length -eq 0) { return $result }
    try {
        $obj = $raw | ConvertFrom-Json
        $result.ok = $true
        $paths = @()
        if ($obj.PSObject.Properties.Name -contains 'candidates' -and $obj.candidates) {
            foreach ($c in $obj.candidates) { $paths += (Get-RelPath $c.path) }
        }
        $result.paths = $paths
        if ($obj.PSObject.Properties.Name -contains 'coverage' -and $obj.coverage) {
            if ($obj.coverage.PSObject.Properties.Name -contains 'semantic_status') {
                $result.semantic = $obj.coverage.semantic_status
            }
        }
    }
    catch { $result.ok = $false }
    return $result
}

function Parse-RgRanking {
    # Reads rg --json (JSON lines); ranks files by match count desc.
    param([string]$OutFile)
    $result = @{ ok = $false; paths = @() }
    if (-not (Test-Path $OutFile)) { return $result }
    $counts = @{}
    $anyLine = $false
    $allOk = $true
    foreach ($line in Get-Content -LiteralPath $OutFile) {
        $t = $line.Trim()
        if ($t.Length -eq 0) { continue }
        $anyLine = $true
        try {
            $evt = $t | ConvertFrom-Json
            if ($evt.type -eq 'match' -and $evt.data -and $evt.data.path) {
                $p = Get-RelPath $evt.data.path.text
                if ($counts.ContainsKey($p)) { $counts[$p]++ } else { $counts[$p] = 1 }
            }
        }
        catch { $allOk = $false }
    }
    $result.ok = ($anyLine -and $allOk)
    $ranked = $counts.GetEnumerator() |
        Sort-Object @{ Expression = { $_.Value }; Descending = $true }, @{ Expression = { $_.Key }; Descending = $false }
    $result.paths = @($ranked | ForEach-Object { $_.Key })
    return $result
}

function Write-Result {
    param([hashtable]$Record, [string]$ResultsFile)
    $json = $Record | ConvertTo-Json -Depth 6 -Compress
    Add-Content -LiteralPath $ResultsFile -Value $json -Encoding utf8
}

function Remove-Index {
    param([string]$RepoDir)
    foreach ($d in @('.git/agentgrep', '.agentgrep')) {
        $full = Join-Path $RepoDir $d
        if (Test-Path $full) { Remove-Item -Recurse -Force $full }
    }
}

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

$bin = Resolve-AgentgrepBin -Explicit $AgentgrepBin
Write-Host "==> agentgrep binary: $bin"

$repos = Assert-RequiredProps `
    -Items (Read-Jsonl $RepoManifest) `
    -Required @('repo_id', 'url', 'commit') `
    -Source $RepoManifest
$tasks = Assert-RequiredProps `
    -Items (Read-Jsonl $TaskFile) `
    -Required @('task_id', 'repo_id', 'task_type', 'query') `
    -Source $TaskFile

# ---------------------------------------------------------------------------
# Filtering: -Repo and/or -OnlyLabeled
# ---------------------------------------------------------------------------
# All filtering uses explicit foreach loops rather than Where-Object pipelines
# so that $repos/$tasks always remain arrays of the original full manifest
# objects. Pipeline Where-Object in PS 5.1 can return PSObject-wrapped copies
# whose NoteProperties are not visible to Get-JsonProp in a subsequent foreach
# iterator — the explicit loop avoids that wrapping entirely.

# Normalise -Repo: split any comma-joined strings so both
#   -Repo agentgrep,ripgrep  and  -Repo agentgrep -Repo ripgrep  work.
$repoFilter = @($Repo | ForEach-Object { $_ -split ',' } | Where-Object { $_ -ne '' })

if ($repoFilter.Count -gt 0) {
    # Build a hashtable for O(1) membership tests.
    $repoFilterSet = @{}
    foreach ($id in $repoFilter) { $repoFilterSet[$id] = $true }

    $filteredTasks = @()
    foreach ($t in $tasks) {
        if ($repoFilterSet.ContainsKey((Get-JsonProp $t 'repo_id'))) { $filteredTasks += $t }
    }
    $tasks = $filteredTasks

    $filteredRepos = @()
    foreach ($r in $repos) {
        if ($repoFilterSet.ContainsKey((Get-JsonProp $r 'repo_id'))) { $filteredRepos += $r }
    }
    $repos = $filteredRepos

    # Warn about requested IDs that matched no manifest entry.
    $matchedIds = @{}
    foreach ($r in $repos) { $matchedIds[(Get-JsonProp $r 'repo_id')] = $true }
    foreach ($id in $repoFilter) {
        if (-not $matchedIds.ContainsKey($id)) {
            Write-Warning "-Repo filter '$id' did not match any repo in the manifest."
        }
    }
}

if ($OnlyLabeled) {
    if (-not (Test-Path $LabelFile)) { throw "-OnlyLabeled: label file not found: $LabelFile" }
    $labels = Assert-RequiredProps `
        -Items (Read-Jsonl $LabelFile) `
        -Required @('task_id') `
        -Source $LabelFile

    # Build a hashtable of labeled task IDs for O(1) lookup.
    $labeledIdSet = @{}
    foreach ($lbl in $labels) {
        $tid = Get-JsonProp $lbl 'task_id'
        if ($null -ne $tid) { $labeledIdSet[$tid] = $true }
    }

    # 1. Filter $tasks to labeled task IDs only.
    $filteredTasks = @()
    foreach ($t in $tasks) {
        if ($labeledIdSet.ContainsKey((Get-JsonProp $t 'task_id'))) { $filteredTasks += $t }
    }
    $tasks = $filteredTasks

    # 2. Compute surviving repo IDs from the now-filtered $tasks.
    $survivingRepoIds = @{}
    foreach ($t in $tasks) {
        $rid = Get-JsonProp $t 'repo_id'
        if ($null -ne $rid) { $survivingRepoIds[$rid] = $true }
    }

    # 3. Filter $repos to full manifest objects whose repo_id is in that set.
    $filteredRepos = @()
    foreach ($r in $repos) {
        if ($survivingRepoIds.ContainsKey((Get-JsonProp $r 'repo_id'))) { $filteredRepos += $r }
    }
    $repos = $filteredRepos

    Write-Host "==> [filter] -OnlyLabeled: $($labeledIdSet.Count) labeled task(s) in label file"
}

# Print selection summary — read repo_id from each full object.
$selectedRepoIds = @()
foreach ($r in $repos) { $selectedRepoIds += (Get-JsonProp $r 'repo_id') }
Write-Host "==> [filter] repos selected  : $($selectedRepoIds.Count) ($($selectedRepoIds -join ', '))"
Write-Host "==> [filter] tasks selected  : $($tasks.Count)"

$runRoot = Join-Path $OutDir $RunId
$rawDir = Join-Path $runRoot 'raw'
$parsedDir = Join-Path $runRoot 'parsed'
New-Item -ItemType Directory -Force -Path $rawDir | Out-Null
New-Item -ItemType Directory -Force -Path $parsedDir | Out-Null
$resultsFile = Join-Path $parsedDir 'results.jsonl'
if (Test-Path $resultsFile) { Remove-Item -Force $resultsFile }

if (-not (Test-Path $WorktreeDir)) { New-Item -ItemType Directory -Force -Path $WorktreeDir | Out-Null }

# run-meta
$agVersion = (& $bin --version) 2>$null | Select-Object -First 1
$rgVersion = $null
$rgCmd = Get-Command 'rg' -ErrorAction SilentlyContinue
if ($rgCmd) { $rgVersion = (& rg --version | Select-Object -First 1) }
$meta = [ordered]@{
    run_id            = $RunId
    timestamp_utc     = (Get-Date).ToUniversalTime().ToString('o')
    agentgrep_bin     = $bin
    agentgrep_version = "$agVersion"
    rg_version        = "$rgVersion"
    os                = "$([System.Environment]::OSVersion.VersionString)"
    repo_manifest     = $RepoManifest
    task_file         = $TaskFile
    label_file        = $LabelFile
    semantic_enabled  = [bool]$EnableSemantic
    filter_repos      = if ($repoFilter.Count -gt 0) { $repoFilter } else { $null }
    filter_only_labeled = [bool]$OnlyLabeled
    selected_repos    = @(foreach ($r in $repos) { Get-JsonProp $r 'repo_id' })
    selected_task_count = $tasks.Count
}
$meta | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $runRoot 'run-meta.json') -Encoding utf8

Write-Host "==> run-id: $RunId"
Write-Host "==> output: $runRoot"
if (-not $rgCmd) { Write-Warning 'rg not found on PATH; Mode A will be empty.' }

# ---------------------------------------------------------------------------
# Run loop, grouped by repo so the index is built once per checkout
# ---------------------------------------------------------------------------

function New-BaseRecord {
    param($Task, [string]$Mode, [string]$Query, [string]$Command)
    return @{
        run_id          = $RunId
        task_id         = $Task.task_id
        repo_id         = $Task.repo_id
        task_type       = $Task.task_type
        mode            = $Mode
        query           = $Query
        command         = $Command
        exit_code       = $null
        latency_ms      = $null
        json_parse_ok   = $false
        ranked_paths    = @()
        semantic_status = $null
        raw_stdout_path = $null
        raw_stderr_path = $null
        skipped         = $false
        skip_reason     = $null
    }
}

# ---------------------------------------------------------------------------
# Defensive pre-flight before the run loop
# ---------------------------------------------------------------------------

# 1. Every selected repo object must be a full manifest object with repo_id,
#    url, and commit. This catches pipeline wrapping or any other loss of the
#    original PSCustomObject fields before the loop ever runs.
$badRepos = @()
foreach ($r in $repos) {
    $rid     = Get-JsonProp $r 'repo_id'
    $rurl    = Get-JsonProp $r 'url'
    $rcommit = Get-JsonProp $r 'commit'
    if ($null -eq $rid -or $null -eq $rurl -or $null -eq $rcommit) {
        $typeName = if ($null -eq $r) { '<null>' } else { $r.GetType().FullName }
        $repr = try { $r | ConvertTo-Json -Compress -Depth 2 } catch { '<unserializable>' }
        $badRepos += "  type=$typeName  rid='$rid'  repr=$repr"
    }
}
if ($badRepos.Count -gt 0) {
    Write-Host "ERROR: $($badRepos.Count) repo object(s) are missing required fields (repo_id/url/commit):"
    foreach ($msg in $badRepos) { Write-Host $msg }
    throw "run-eval: aborting - repo objects above are malformed. Check $RepoManifest."
}
$verifiedRepoIds = @()
foreach ($r in $repos) { $verifiedRepoIds += (Get-JsonProp $r 'repo_id') }
Write-Host "==> [pre-flight] repo objects OK: $($verifiedRepoIds -join ', ')"

# 2. Every remaining task row must have a non-null repo_id.
$badTasks = @()
foreach ($t in $tasks) {
    if ($null -eq (Get-JsonProp $t 'repo_id')) { $badTasks += $t }
}
if ($badTasks.Count -gt 0) {
    Write-Host "ERROR: $($badTasks.Count) task row(s) are missing 'repo_id':"
    foreach ($bt in $badTasks) {
        $typeName = if ($null -eq $bt) { '<null>' } else { $bt.GetType().FullName }
        $repr = try { $bt | ConvertTo-Json -Compress -Depth 2 } catch { '<unserializable>' }
        Write-Host "  type=$typeName  repr=$repr"
    }
    throw "run-eval: aborting - task rows above are missing 'repo_id'. Check $TaskFile."
}

# 3. Print task counts grouped by repo_id via Get-JsonProp (same helper used in
#    the loop). If a selected repo shows 0 here, the loop will also see 0 — fail
#    now with a diagnostic dump of the first few task objects.
$taskCountByRepo = @{}
foreach ($t in $tasks) {
    $rid = Get-JsonProp $t 'repo_id'
    if ($null -ne $rid) {
        if ($taskCountByRepo.ContainsKey($rid)) { $taskCountByRepo[$rid]++ }
        else                                     { $taskCountByRepo[$rid] = 1 }
    }
}
$taskCountParts = @()
foreach ($entry in ($taskCountByRepo.GetEnumerator() | Sort-Object Key)) {
    $taskCountParts += "$($entry.Key)=$($entry.Value)"
}
Write-Host "==> [pre-flight] task counts by repo: $($taskCountParts -join ', ')"

foreach ($r in $repos) {
    $rid = Get-JsonProp $r 'repo_id'
    $cnt = if ($taskCountByRepo.ContainsKey($rid)) { $taskCountByRepo[$rid] } else { 0 }
    if ($cnt -eq 0) {
        Write-Host "ERROR: selected repo '$rid' has zero tasks after Get-JsonProp grouping."
        Write-Host "       First 3 task objects for diagnosis:"
        $diagIdx = 0
        foreach ($t in $tasks) {
            if ($diagIdx -ge 3) { break }
            $typeName = if ($null -eq $t) { '<null>' } else { $t.GetType().FullName }
            $repr = try { $t | ConvertTo-Json -Compress -Depth 2 } catch { '<unserializable>' }
            Write-Host "  [$diagIdx] type=$typeName  repo_id_via_helper='$(Get-JsonProp $t 'repo_id')'  repr=$repr"
            $diagIdx++
        }
        throw "run-eval: aborting - task/repo grouping mismatch. Check $TaskFile and $RepoManifest."
    }
}

foreach ($repoRec in $repos) {
    $repoId = Get-JsonProp $repoRec 'repo_id'
    if ($null -eq $repoId -or $repoId -eq '') {
        $typeName = if ($null -eq $repoRec) { '<null>' } else { $repoRec.GetType().FullName }
        throw "run-eval: repo object has null/empty repo_id inside loop (type=$typeName). Check $RepoManifest."
    }

    $repoTasks = foreach ($task in $tasks) {
        if ((Get-JsonProp $task 'repo_id') -eq $repoId) { $task }
    }
    $repoTasks = @($repoTasks)

    if ($repoTasks.Count -eq 0) {
        throw "run-eval: selected repo '$repoId' has zero matching tasks inside loop. This should not happen after pre-flight."
    }

    $repoCommit = Get-JsonProp $repoRec 'commit'
    $repoUrl    = Get-JsonProp $repoRec 'url'
    $repoDir = Join-Path $WorktreeDir $repoId
    Write-Host "==> [$repoId] preparing checkout at $repoCommit"
    if (-not (Test-Path (Join-Path $repoDir '.git'))) {
        git clone --quiet $repoUrl $repoDir
    }
    git -C $repoDir fetch --quiet --tags origin 2>$null | Out-Null
    git -C $repoDir checkout --quiet $repoCommit
    $repoDirFull = (Resolve-Path $repoDir).Path

    # Mode A + B run on a fresh (index-free) checkout.
    Remove-Index -RepoDir $repoDirFull

    foreach ($task in $repoTasks) {
        $q = $task.query
        $slug = "$($task.task_id)"

        # ---- Mode A: rg baseline ----
        $aOut = Join-Path $rawDir "$slug-A.out"
        $aErr = Join-Path $rawDir "$slug-A.err"
        $recA = New-BaseRecord -Task $task -Mode 'A' -Query $q -Command "rg --json `"$q`" ."
        if ($rgCmd) {
            # Explicit '.' search path: without a path rg reads from stdin, which
            # blocks forever when the harness redirects standard streams.
            $cap = Invoke-Capture -FilePath $rgCmd.Source -Arguments @('--json', $q, '.') -WorkDir $repoDirFull -OutFile $aOut -ErrFile $aErr
            $rank = Parse-RgRanking -OutFile $aOut
            $recA.exit_code = $cap.ExitCode
            $recA.latency_ms = $cap.LatencyMs
            $recA.json_parse_ok = $rank.ok
            $recA.ranked_paths = $rank.paths
            $recA.raw_stdout_path = "raw/$slug-A.out"
            $recA.raw_stderr_path = "raw/$slug-A.err"
        }
        else {
            $recA.skipped = $true; $recA.skip_reason = 'rg not on PATH'
        }
        Write-Result -Record $recA -ResultsFile $resultsFile

        # ---- Mode B: agentgrep find, no index ----
        $bOut = Join-Path $rawDir "$slug-B.out"
        $bErr = Join-Path $rawDir "$slug-B.err"
        $cap = Invoke-Capture -FilePath $bin -Arguments @('find', $q, '--json') -WorkDir $repoDirFull -OutFile $bOut -ErrFile $bErr
        $rank = Parse-FindRanking -OutFile $bOut
        $recB = New-BaseRecord -Task $task -Mode 'B' -Query $q -Command "agentgrep find `"$q`" --json"
        $recB.exit_code = $cap.ExitCode
        $recB.latency_ms = $cap.LatencyMs
        $recB.json_parse_ok = $rank.ok
        $recB.ranked_paths = $rank.paths
        $recB.semantic_status = $rank.semantic
        $recB.raw_stdout_path = "raw/$slug-B.out"
        $recB.raw_stderr_path = "raw/$slug-B.err"
        Write-Result -Record $recB -ResultsFile $resultsFile
    }

    # ---- Build index for Mode C ----
    Write-Host "==> [$repoId] building index (Mode C)"
    $idxOut = Join-Path $rawDir "_index-$repoId.out"
    $idxErr = Join-Path $rawDir "_index-$repoId.err"
    $idxCap = Invoke-Capture -FilePath $bin -Arguments @('index') -WorkDir $repoDirFull -OutFile $idxOut -ErrFile $idxErr
    $indexOk = ($idxCap.ExitCode -eq 0)
    if (-not $indexOk) { Write-Warning "[$repoId] index build failed (exit $($idxCap.ExitCode)); Mode C will reflect that." }

    # ---- Build semantic index for Mode D (optional) ----
    $semanticOk = $false
    if ($EnableSemantic) {
        Write-Host "==> [$repoId] building semantic index (Mode D)"
        $semOut = Join-Path $rawDir "_semantic-$repoId.out"
        $semErr = Join-Path $rawDir "_semantic-$repoId.err"
        $semCap = Invoke-Capture -FilePath $bin -Arguments @('index', '--semantic', '--yes') -WorkDir $repoDirFull -OutFile $semOut -ErrFile $semErr
        $semanticOk = ($semCap.ExitCode -eq 0)
        if (-not $semanticOk) { Write-Warning "[$repoId] semantic index unavailable; Mode D skipped." }
    }

    foreach ($task in $repoTasks) {
        $q = $task.query
        $slug = "$($task.task_id)"

        # ---- Mode C: agentgrep find, indexed ----
        $cOut = Join-Path $rawDir "$slug-C.out"
        $cErr = Join-Path $rawDir "$slug-C.err"
        $cap = Invoke-Capture -FilePath $bin -Arguments @('find', $q, '--json') -WorkDir $repoDirFull -OutFile $cOut -ErrFile $cErr
        $rank = Parse-FindRanking -OutFile $cOut
        $recC = New-BaseRecord -Task $task -Mode 'C' -Query $q -Command "agentgrep find `"$q`" --json"
        $recC.exit_code = $cap.ExitCode
        $recC.latency_ms = $cap.LatencyMs
        $recC.json_parse_ok = $rank.ok
        $recC.ranked_paths = $rank.paths
        $recC.semantic_status = $rank.semantic
        $recC.raw_stdout_path = "raw/$slug-C.out"
        $recC.raw_stderr_path = "raw/$slug-C.err"
        Write-Result -Record $recC -ResultsFile $resultsFile

        # ---- Mode D: agentgrep find, semantic ----
        $recD = New-BaseRecord -Task $task -Mode 'D' -Query $q -Command "agentgrep find `"$q`" --semantic --json"
        if ($EnableSemantic -and $semanticOk) {
            $dOut = Join-Path $rawDir "$slug-D.out"
            $dErr = Join-Path $rawDir "$slug-D.err"
            $cap = Invoke-Capture -FilePath $bin -Arguments @('find', $q, '--semantic', '--json') -WorkDir $repoDirFull -OutFile $dOut -ErrFile $dErr
            $rank = Parse-FindRanking -OutFile $dOut
            $recD.exit_code = $cap.ExitCode
            $recD.latency_ms = $cap.LatencyMs
            $recD.json_parse_ok = $rank.ok
            $recD.ranked_paths = $rank.paths
            $recD.semantic_status = $rank.semantic
            $recD.raw_stdout_path = "raw/$slug-D.out"
            $recD.raw_stderr_path = "raw/$slug-D.err"
        }
        else {
            $recD.skipped = $true
            $recD.skip_reason = if (-not $EnableSemantic) { 'semantic not enabled (-EnableSemantic)' } else { 'semantic index unavailable' }
        }
        Write-Result -Record $recD -ResultsFile $resultsFile
    }

    # Leave the checkout in its index-free state for the next run.
    Remove-Index -RepoDir $repoDirFull
}

Write-Host ''
Write-Host "Run complete. Results: $resultsFile"
Write-Host "Next: python scripts/analyze-eval.py --run-dir $runRoot --labels $LabelFile"
