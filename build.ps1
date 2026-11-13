param(
    [ValidateSet("core", "full", "all")]
    [string]$Target = "core",

    [ValidateSet("release", "bench-fast")]
    [string]$Profile = "release",

    [string]$InstallPath = (Join-Path $env:USERPROFILE ".cargo\bin\agent-harness.exe")
)

$ErrorActionPreference = "Stop"

$cargoArgs = @("build", "--profile", $Profile)
if ($Target -eq "full" -or $Target -eq "all") {
    $cargoArgs += "--all-features"
}

Write-Host "Running: cargo $($cargoArgs -join ' ')"
$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$stopwatch.Stop()

$exePath = Join-Path $PSScriptRoot "target\$Profile\agent-harness.exe"
if (Test-Path $exePath) {
    $sizeMb = [Math]::Round((Get-Item $exePath).Length / 1MB, 2)
    Write-Host "Build completed in $([Math]::Round($stopwatch.Elapsed.TotalSeconds, 2))s | $exePath ($sizeMb MB)"

    if ($InstallPath) {
        Stop-Process -Name "agent-harness" -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 300
        $destDir = Split-Path -Parent $InstallPath
        if ($destDir -and -not (Test-Path $destDir)) {
            New-Item -ItemType Directory -Path $destDir -Force | Out-Null
        }
        Copy-Item -Path $exePath -Destination $InstallPath -Force
        Write-Host "Installed to $InstallPath"
    }
} else {
    Write-Host "Build completed in $([Math]::Round($stopwatch.Elapsed.TotalSeconds, 2))s | Binary not found at $exePath"
}
