[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$BinaryPath
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    if ($env:OS -ne 'Windows_NT' -or -not [Environment]::Is64BitOperatingSystem) {
        throw 'This package targets Windows x86-64 only.'
    }
    if ($SkipBuild -and -not $BinaryPath) { throw '-SkipBuild requires an explicit -BinaryPath.' }
    if (-not $SkipBuild) {
        & cargo build --release --locked --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
        $BinaryPath = Join-Path $repo 'target/x86_64-pc-windows-msvc/release/agent-harness.exe'
    }
    $binary = (Resolve-Path -LiteralPath $BinaryPath).Path
    $manifest = Get-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Raw
    $version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $version -or $version -notmatch '^[0-9A-Za-z.+-]+$') { throw 'Invalid package version.' }
    $reported = & $binary --version
    if ($LASTEXITCODE -ne 0 -or ($reported -join ' ') -notmatch [regex]::Escape($version)) {
        throw 'Binary version does not match Cargo.toml.'
    }
    $name = "agent-harness-$version-windows-x86_64"
    $dist = Join-Path $repo 'target/dist'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $zip = Join-Path $dist "$name.zip"
    $sums = Join-Path $dist "$name-SHA256SUMS.txt"
    if ((Test-Path -LiteralPath $zip) -or (Test-Path -LiteralPath $sums)) {
        throw 'Release artifacts already exist. Preserve them or choose a new version before packaging.'
    }
    $stage = Join-Path $dist ("stage-" + [guid]::NewGuid().ToString('N'))
    $package = Join-Path $stage $name
    New-Item -ItemType Directory -Path $package -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $package 'agent-harness.exe')
    Copy-Item -LiteralPath (Join-Path $repo 'release/README.md') -Destination (Join-Path $package 'README.md')
    Copy-Item -LiteralPath (Join-Path $repo 'vendor/codegraph-native/LICENSE') -Destination (Join-Path $package 'CodeGraph-LICENSE.txt')
    Copy-Item -LiteralPath (Join-Path $repo 'ROLLDOWN-LICENSE.txt') -Destination $package
    Copy-Item -LiteralPath (Join-Path $repo 'Cargo.lock') -Destination $package
    $config = @{ mcpServers = @{ harness = @{ command = 'C:\tools\agent-harness\agent-harness.exe'; args = @('--root', 'C:\projects\your-project') } } }
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText((Join-Path $package 'mcp.example.json'), ($config | ConvertTo-Json -Depth 6), $utf8)
    $metadataText = & cargo metadata --locked --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Dependency inventory failed.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    $inventory = @($metadata.packages | Sort-Object name, version | ForEach-Object {
        [ordered]@{ name = $_.name; version = $_.version; license = $_.license; source = $_.source }
    })
    [IO.File]::WriteAllText((Join-Path $package 'dependencies.json'), (ConvertTo-Json -InputObject $inventory -Depth 5), $utf8)
    Compress-Archive -LiteralPath $package -DestinationPath $zip -CompressionLevel Optimal
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($sums, "$hash  $name.zip`n", $utf8)
    Write-Output $zip
    Write-Output $sums
    Write-Output "Staging retained for inspection: $package"
} finally { Pop-Location }
