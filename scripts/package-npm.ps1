[CmdletBinding()]
param([switch]$SkipBuild, [switch]$PrepareOnly)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    if (-not $SkipBuild) {
        & cargo build --release --locked -p agent-harness -j 2
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $manifest = Get-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Raw
    $version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    $package = Join-Path $repo 'npm-dist'
    $npm = Get-Content -LiteralPath (Join-Path $package 'package.json') -Raw | ConvertFrom-Json
    if ($version -ne $npm.version) { throw 'Cargo/npm version mismatch.' }
    $binary = Join-Path $repo 'target/release/agent-harness.exe'
    $reported = & $binary --version
    if ($LASTEXITCODE -ne 0 -or $reported -ne "agent-harness $version") { throw 'Release binary version mismatch.' }
    Copy-Item -LiteralPath $binary -Destination (Join-Path $package 'bin/agent-harness.exe')
    Copy-Item -LiteralPath (Join-Path $repo 'vendor/codegraph-native/LICENSE') -Destination (Join-Path $package 'CodeGraph-LICENSE.txt')
    Copy-Item -LiteralPath (Join-Path $repo 'ROLLDOWN-LICENSE.txt') -Destination (Join-Path $package 'ROLLDOWN-LICENSE.txt')
    Push-Location $package
    try {
        & node scripts/verify-package.cjs
        if ($LASTEXITCODE -ne 0) { throw 'npm package verification failed.' }
        if ($PrepareOnly) { return }
        $dist = Join-Path $repo 'target/dist'
        New-Item -ItemType Directory -Force -Path $dist | Out-Null
        & npm pack --pack-destination $dist
        if ($LASTEXITCODE -ne 0) { throw 'npm pack failed.' }
    } finally { Pop-Location }
} finally { Pop-Location }
