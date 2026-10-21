param([Parameter(Mandatory=$true)][string]$Binary)
$ErrorActionPreference = 'Stop'
$resolvedBinary = (Resolve-Path -LiteralPath $Binary).Path
$observations = @()
foreach ($size in @(16, 128, 512)) {
    & $resolvedBinary check $size
    if ($LASTEXITCODE -ne 0) { throw "Symbol equality gate failed for $size" }
    foreach ($round in 1..3) {
        $engines = if ($round % 2 -eq 1) { @('syn', 'ra') } else { @('ra', 'syn') }
        foreach ($engine in $engines) {
            $start = [System.Diagnostics.ProcessStartInfo]::new()
            $start.FileName = $resolvedBinary
            $start.Arguments = "$engine $size"
            $start.UseShellExecute = $false
            $start.CreateNoWindow = $true
            $start.RedirectStandardOutput = $true
            $start.RedirectStandardError = $true
            $process = [System.Diagnostics.Process]::Start($start)
            $outputTask = $process.StandardOutput.ReadToEndAsync()
            $errorTask = $process.StandardError.ReadToEndAsync()
            $peak = 0L
            $deadline = [DateTime]::UtcNow.AddSeconds(60)
            while (-not $process.HasExited) {
                if ([DateTime]::UtcNow -gt $deadline) {
                    $process.Kill()
                    throw "Own benchmark process exceeded 60 seconds"
                }
                try { $process.Refresh(); $peak = [Math]::Max($peak, $process.PeakWorkingSet64) } catch {}
                Start-Sleep -Milliseconds 5
            }
            $process.WaitForExit()
            if ($process.ExitCode -ne 0) { throw $errorTask.Result }
            $row = $outputTask.Result | ConvertFrom-Json
            $row | Add-Member NoteProperty round $round
            $row | Add-Member NoteProperty observedProcessPeakWorkingSetBytes $peak
            $observations += $row
            $process.Dispose()
        }
    }
}
$observations | ConvertTo-Json -Depth 8
