param(
    [ValidateSet('test', 'release')][string]$Profile = 'release',
    [ValidateRange(1, 20)][int]$Iterations = 3,
    [ValidateRange(1, 20)][int]$Rounds = 3,
    [ValidateRange(1, 100)][int]$PollMilliseconds = 5,
    [string]$Filter = 'framework_extract_and_incremental_publication_benchmark'
)

# Build outside the measured interval. Fixtures/indexes belong to the Rust test
# and are temporary; this script never initializes the repository's index.
$ErrorActionPreference = 'Stop'
$benchmarkRoot = Split-Path -Parent $PSScriptRoot
Push-Location $benchmarkRoot
$previousIterations = [Environment]::GetEnvironmentVariable('HARNESS_FRAMEWORK_BENCH_ITERATIONS')
try {
    $buildArguments = @('test', '--locked', '--test', 'framework_benchmark', '--no-run', '--message-format=json', '-j2')
    if ($Profile -eq 'release') { $buildArguments += '--release' }
    $buildOutput = & cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw 'Benchmark build failed' }
    $artifacts = @($buildOutput | ForEach-Object {
        $record = $_ | ConvertFrom-Json
        if ($record.reason -eq 'compiler-artifact' -and $record.target.name -eq 'framework_benchmark' -and $record.executable) {
            $record.executable
        }
    })
    if ($artifacts.Count -ne 1) { throw 'Expected exactly one benchmark test executable' }
    $env:HARNESS_FRAMEWORK_BENCH_ITERATIONS = "$Iterations"
    $revision = (& git rev-parse HEAD).Trim()
    $dirty = @(& git status --porcelain).Count -gt 0
    $compiler = (& rustc --version).Trim()
    # Identify measured sources even before a candidate commit exists. Excludes
    # build outputs, local configuration and documentation-only changes.
    $fingerprintInput = [Text.StringBuilder]::new()
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $sourcePaths = @(& git ls-files -c -o --exclude-standard -- src crates vendor Cargo.toml Cargo.lock tests/framework_benchmark.rs scripts/benchmark-framework.ps1) | Sort-Object -Unique
        foreach ($sourcePath in $sourcePaths) {
            if (Test-Path -LiteralPath $sourcePath -PathType Leaf) {
                $digest = [BitConverter]::ToString($sha.ComputeHash([IO.File]::ReadAllBytes((Join-Path $benchmarkRoot $sourcePath)))).Replace('-', '').ToLowerInvariant()
                [void]$fingerprintInput.Append("$sourcePath`0$digest`n")
            }
        }
        $sourceFingerprint = [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($fingerprintInput.ToString()))).Replace('-', '').ToLowerInvariant()
    } finally { $sha.Dispose() }
    for ($round = 1; $round -le $Rounds; $round++) {
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $artifacts[0]
        if ($Filter -notmatch '^[A-Za-z0-9_:]+$') { throw 'Filter must be a Rust test name' }
        $startInfo.Arguments = "--ignored --nocapture --test-threads=1 --exact $Filter"
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        $watch = [Diagnostics.Stopwatch]::StartNew()
        [void]$process.Start()
        # Drain asynchronously to avoid stdout/stderr pipe deadlocks.
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $peakBytes = 0L
        $memorySamples = 0
        while (-not $process.HasExited) {
            try {
                $process.Refresh()
                $peakBytes = [Math]::Max($peakBytes, $process.PeakWorkingSet64)
                $memorySamples++
            } catch [InvalidOperationException] {
                if (-not $process.HasExited) { throw }
            }
            [Threading.Thread]::Sleep($PollMilliseconds)
        }
        $process.WaitForExit()
        $watch.Stop()
        $output = $stdout.GetAwaiter().GetResult()
        $errors = $stderr.GetAwaiter().GetResult()
        Write-Output $output
        if ($output -match 'running 0 tests') { throw 'Benchmark filter matched no tests' }
        if ($errors) { [Console]::Error.WriteLine($errors) }
        if ($process.ExitCode -ne 0) { throw "Benchmark exited with $($process.ExitCode)" }
        # This is a process-wide observed OS high-water mark, NOT per-operation
        # allocation. A final burst after the last poll may be missed.
        [pscustomobject]@{
            probe = 'framework-process-sample'
            revision = $revision
            worktreeDirty = $dirty
            sourceFingerprintSha256 = $sourceFingerprint
            compiler = $compiler
            profile = $Profile
            round = $round
            iterations = $Iterations
            filter = $Filter
            elapsedMs = [Math]::Round($watch.Elapsed.TotalMilliseconds, 3)
            observedPeakWorkingSetBytes = $(if ($peakBytes -gt 0) { $peakBytes } else { $null })
            memorySamples = $memorySamples
            pollMilliseconds = $PollMilliseconds
            memoryScope = 'whole-test-process-observed-high-water-lower-bound'
            productionIndexOpened = $false
        } | ConvertTo-Json -Compress
        $process.Dispose()
    }
} finally {
    [Environment]::SetEnvironmentVariable('HARNESS_FRAMEWORK_BENCH_ITERATIONS', $previousIterations)
    Pop-Location
}
