param(
    [string]$Workspace = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path,
    [string]$Candidate = (Join-Path $PSScriptRoot 'src\call_candidate.rs'),
    [string]$TargetBase = (Join-Path $env:TEMP 'harness-ra-syntax-compare-target\writer-replay'),
    [string]$SnapshotId = [guid]::NewGuid().ToString('N'),
    [switch]$ReuseSnapshot
)

$ErrorActionPreference = 'Stop'
$snapshotId = $SnapshotId
$snapshot = Join-Path $env:TEMP "harness-ra-writer-replay-$snapshotId"
$target = Join-Path $TargetBase $snapshotId
New-Item -ItemType Directory -Path $target -Force | Out-Null

function Get-InputManifest([string]$Root) {
    $files = [System.Collections.Generic.List[object]]::new()
    foreach ($directory in @('src', 'crates', 'vendor')) {
        $path = Join-Path $Root $directory
        Get-ChildItem -LiteralPath $path -File -Recurse -Force | ForEach-Object {
            $relative = $_.FullName.Substring($Root.Length).TrimStart('\','/') -replace '\\','/'
            $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            $files.Add([pscustomobject]@{ path = $relative; sha256 = $hash })
        }
    }
    foreach ($file in @('Cargo.toml', 'Cargo.lock', 'contracts/tool-contract.json', 'docs/TOOL-CONTRACT.md', 'tests/fixtures/datatables-helper.js')) {
        $path = Join-Path $Root $file
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        $files.Add([pscustomobject]@{ path = $file; sha256 = $hash })
    }
    @($files | Sort-Object path)
}

function Copy-DirectoryContents([string]$From, [string]$To) {
    New-Item -ItemType Directory -Path $To -Force | Out-Null
    Get-ChildItem -LiteralPath $From -Force | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $To -Recurse -Force
    }
}

function Assert-SameManifest($Expected, $Actual, [string]$Message) {
    if ((ConvertTo-Json -InputObject $Expected -Depth 4 -Compress) -cne
        (ConvertTo-Json -InputObject $Actual -Depth 4 -Compress)) {
        throw $Message
    }
}
if ($ReuseSnapshot) {
    if (-not (Test-Path -LiteralPath (Join-Path $snapshot 'source-manifest.json'))) {
        throw "Cannot reuse missing frozen snapshot $snapshot."
    }
    $baselineCopy = "$snapshot.baseline"
    Copy-Item -LiteralPath (Join-Path $baselineCopy 'src\adapters\index_rust.rs') -Destination (Join-Path $snapshot 'src\adapters\index_rust.rs') -Force
    Copy-Item -LiteralPath (Join-Path $baselineCopy 'Cargo.toml') -Destination (Join-Path $snapshot 'Cargo.toml') -Force
    Copy-Item -LiteralPath (Join-Path $baselineCopy 'Cargo.lock') -Destination (Join-Path $snapshot 'Cargo.lock') -Force
}
else {
    New-Item -ItemType Directory -Path $snapshot -Force | Out-Null
    $before = Get-InputManifest $Workspace
    foreach ($directory in @('src', 'crates', 'vendor')) {
        Copy-DirectoryContents (Join-Path $Workspace $directory) (Join-Path $snapshot $directory)
    }
    foreach ($file in @('Cargo.toml', 'Cargo.lock', 'contracts/tool-contract.json', 'docs/TOOL-CONTRACT.md', 'tests/fixtures/datatables-helper.js')) {
        $sourceFile = Join-Path $Workspace $file
        $destinationFile = Join-Path $snapshot $file
        New-Item -ItemType Directory -Path (Split-Path -Parent $destinationFile) -Force | Out-Null
        Copy-Item -LiteralPath $sourceFile -Destination $destinationFile
    }
    $after = Get-InputManifest $Workspace
    Assert-SameManifest $before $after 'Workspace sources changed while the temporary writer snapshot was copied; discard this run and recapture.'
    $snapshotManifest = Get-InputManifest $snapshot
    Assert-SameManifest $before $snapshotManifest 'The temporary snapshot differs from its frozen workspace manifest.'
    $before | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $snapshot 'source-manifest.json') -Encoding utf8
}

$replaySource = Join-Path $PSScriptRoot 'writer_replay.rs'
$storeModule = Join-Path $snapshot 'src\adapters\index_store.rs'
$replayModule = Join-Path $snapshot 'src\adapters\index_store\rust_replay_tests.rs'
New-Item -ItemType Directory -Path (Split-Path -Parent $replayModule) -Force | Out-Null
Copy-Item -LiteralPath $replaySource -Destination $replayModule
$testDeclaration = "`r`n#[cfg(all(test, windows))]`r`n#[path = `"index_store/rust_replay_tests.rs`"]`r`nmod rust_replay_tests;`r`n"
$storeText = [System.IO.File]::ReadAllText($storeModule)
 $declarationPattern = '(?:\r?\n#\[cfg\(all\(test, windows\)\)\]\r?\n#\[path = "index_store/rust_replay_tests\.rs"\]\r?\nmod rust_replay_tests;\r?\n)+'
 $existingDeclarations = [regex]::Matches($storeText, $declarationPattern)
 if ($existingDeclarations.Count -gt 0) {
    $storeText = [regex]::Replace($storeText, $declarationPattern, "`r`n#[cfg(all(test, windows))]`r`n#[path = `"index_store/rust_replay_tests.rs`"]`r`nmod rust_replay_tests;`r`n", 1)
    [System.IO.File]::WriteAllText($storeModule, $storeText, [System.Text.UTF8Encoding]::new($false))
 }
else {
    [System.IO.File]::AppendAllText($storeModule, $testDeclaration, [System.Text.UTF8Encoding]::new($false))
}

function Invoke-WriterReplay([string]$Root, [bool]$Locked) {
    $arguments = @(
        'test', '--manifest-path', (Join-Path $Root 'Cargo.toml'),
        '--target-dir', $target, '--jobs', '2', '--package', 'agent-harness', '--lib',
        'rust_writer_replay', '--', '--nocapture', '--test-threads=1'
    )
    if ($Locked) { $arguments = @('test', '--locked') + $arguments[1..($arguments.Length - 1)] }
    $savedErrorPreference = $ErrorActionPreference
    try {
        # Cargo writes normal build progress to stderr. In PowerShell 7 that can
        # become NativeCommandError; it is output here, not a terminating error.
        $ErrorActionPreference = 'Continue'
        $output = @(& cargo @arguments 2>&1 | ForEach-Object { $_.ToString() })
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $savedErrorPreference
    }
    $output | ForEach-Object { Write-Host $_ }
    if ($exitCode -ne 0) {
        throw "Writer replay failed in $Root with exit code $exitCode. Snapshot kept at $Root."
    }
    if (-not ($output | Where-Object { $_ -match '^running 2 tests$' })) {
        throw "Expected exactly two writer tests in $Root. Snapshot kept at $Root."
    }
    if (-not ($output | Where-Object { $_ -match '^test result: ok\. 2 passed; 0 failed;' })) {
        throw "Expected both writer tests to pass in $Root. Snapshot kept at $Root."
    }
    $events = @(
        $output | ForEach-Object {
            if ($_ -match '.*RA_WRITER_REPLAY:(\{.*\})') { $Matches[1] }
        }
    )
    $events
}

Write-Host "Temporary source snapshot: $snapshot"
Write-Host "Isolated Cargo target: $target"
Write-Host "Frozen source manifest: $(Join-Path $snapshot 'source-manifest.json')"
Write-Host 'Running the unchanged syn writer snapshot first.'
$synEvents = @(Invoke-WriterReplay $snapshot $true)
if ($synEvents.Count -ne 12) {
    throw "Expected 12 syn replay checkpoints but captured $($synEvents.Count). Snapshot kept at $snapshot."
}
$synLockHash = (Get-FileHash -LiteralPath (Join-Path $snapshot 'Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()

$candidateHash = (Get-FileHash -LiteralPath $Candidate -Algorithm SHA256).Hash.ToLowerInvariant()
$candidateCopy = Join-Path $snapshot 'src\adapters\index_rust_ra_candidate.rs'
Copy-Item -LiteralPath $Candidate -Destination $candidateCopy
if ((Get-FileHash -LiteralPath $candidateCopy -Algorithm SHA256).Hash.ToLowerInvariant() -cne $candidateHash) {
    throw 'RA candidate changed while the temporary snapshot was prepared.'
}
$shim = @'
use super::{Failure, OperationContext};
#[path = "index_rust_ra_candidate.rs"]
mod candidate;
pub(super) use candidate::Facts;
pub(super) fn analyze(source: &str, context: &OperationContext) -> Result<Facts, Failure> {
    candidate::analyze(source, context)
}
'@
$shimPath = Join-Path $snapshot 'src\adapters\index_rust.rs'
[System.IO.File]::WriteAllText($shimPath, $shim, [System.Text.UTF8Encoding]::new($false))
$rootManifest = Join-Path $snapshot 'Cargo.toml'
$manifestText = [System.IO.File]::ReadAllText($rootManifest)
if (-not $manifestText.Contains('ra_ap_syntax')) {
    $manifestText = $manifestText.Replace('[dependencies]', "[dependencies]`r`nra_ap_syntax = `"=0.0.289`"")
    [System.IO.File]::WriteAllText($rootManifest, $manifestText, [System.Text.UTF8Encoding]::new($false))
}
Write-Host "RA candidate source SHA-256: $candidateHash"
Write-Host 'Running the same writer tests with only index_rust replaced in the temporary snapshot.'
$raEvents = @(Invoke-WriterReplay $snapshot $false)
$raLockHash = (Get-FileHash -LiteralPath (Join-Path $snapshot 'Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()
if ($raEvents.Count -ne 12) {
    throw "Expected 12 RA replay checkpoints but captured $($raEvents.Count). Snapshot kept at $snapshot."
}
if ($synEvents.Count -ne $raEvents.Count) {
    throw "Replay event counts differ: syn=$($synEvents.Count), RA=$($raEvents.Count). Snapshot kept at $snapshot."
}
for ($i = 0; $i -lt $synEvents.Count; $i++) {
    if ($synEvents[$i] -cne $raEvents[$i]) {
        throw "Persisted writer state differs at replay event $i. Snapshot kept at $snapshot."
    }
}
Write-Host "PASS: $($synEvents.Count) persisted graph checkpoints match between syn and RA."
Write-Host "Syn lock SHA-256: $synLockHash"
Write-Host "RA snapshot lock SHA-256: $raLockHash"
Write-Host "Snapshot retained for inspection: $snapshot"
