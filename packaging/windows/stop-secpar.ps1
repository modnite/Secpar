# Stops the parts of this app that were started from the folder being installed
# over, so their files can be replaced. Nothing outside that folder is touched:
# another copy or another product is left running.
param([string]$Root)
if (-not $Root) { exit 3 }
$root = [IO.Path]::GetFullPath($Root)
$names = 'secpar', 'secpar-service', 'SecparStream'
foreach ($name in $names) {
    foreach ($process in @(Get-Process -Name $name -ErrorAction SilentlyContinue)) {
        try {
            if ($process.Path -and $process.Path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
                $process.Kill()
                $process.WaitForExit(5000) | Out-Null
            }
        } catch {
        }
    }
}
# The engine and the stream component take a moment to let go of their files.
for ($i = 0; $i -lt 40; $i++) {
    $busy = $false
    foreach ($name in $names) {
        foreach ($process in @(Get-Process -Name $name -ErrorAction SilentlyContinue)) {
            try {
                if ($process.Path -and $process.Path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { $busy = $true }
            } catch {
            }
        }
    }
    if (-not $busy) { break }
    Start-Sleep -Milliseconds 250
}
exit 0
