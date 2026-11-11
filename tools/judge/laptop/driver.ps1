# Runs AutoCAD console jobs one at a time on the machine that has AutoCAD.
# Each job line: id<TAB>dwg (or -)<TAB>script<TAB>output prefix
# Before each start it waits until no AutoCAD console is running (another program may use AutoCAD too)
# and until at least 3 GB of memory is free. MaxWaitSec is what the run has left for that waiting, in
# all, not per job: once it is spent, the job waiting and every job after it are recorded as BUSY or
# LOWMEM at once and the driver ends. The last line gives the seconds waited, for the run's next call.
# Each AutoCAD is started and stopped by its process ID, with a time limit.
param(
  [Parameter(Mandatory = $true)][string]$Jobs,
  [Parameter(Mandatory = $true)][string]$Results,
  [Parameter(Mandatory = $true)][string]$Accore,
  [int]$TimeoutSec = 120,
  [int]$MaxWaitSec = 1200
)
$ErrorActionPreference = 'Stop'
$minFreeKB = 3 * 1024 * 1024
function FreeKB { [int64](Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory }
function ConsoleRunning { @(Get-Process -Name accoreconsole -ErrorAction SilentlyContinue).Count -gt 0 }
$script:waited = 0.0   # seconds spent waiting for a turn, over every job in this call
$script:gaveUp = ''    # BUSY or LOWMEM once the budget is spent
function WaitTurn {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($true) {
    $busy = ConsoleRunning
    $low = (-not $busy) -and ((FreeKB) -lt $minFreeKB)
    if (-not $busy -and -not $low) { $script:waited += $sw.Elapsed.TotalSeconds; return 'GO' }
    if ($script:waited + $sw.Elapsed.TotalSeconds -ge $MaxWaitSec) {
      $script:waited += $sw.Elapsed.TotalSeconds
      if ($busy) { return 'BUSY' } else { return 'LOWMEM' }
    }
    if ($low) { Start-Sleep -Seconds 5 } else { Start-Sleep -Milliseconds 500 }
  }
}
if (-not (Test-Path -LiteralPath $Accore)) { throw "AutoCAD console not found at the configured path" }
"id`tstatus`texit`tseconds`tfree_kb_before`tpid" | Set-Content -LiteralPath $Results -Encoding ASCII
foreach ($line in (Get-Content -LiteralPath $Jobs)) {
  if (-not $line.Trim()) { continue }
  $f = $line.Split("`t")
  $id = $f[0]; $dwg = $f[1]; $scr = $f[2]; $outp = $f[3]
  if ($script:gaveUp) { "$id`t$($script:gaveUp)`t`t0`t`t" | Add-Content -LiteralPath $Results -Encoding ASCII; continue }
  $turn = WaitTurn
  if ($turn -ne 'GO') {
    $script:gaveUp = $turn
    "$id`t$turn`t`t0`t$(FreeKB)`t" | Add-Content -LiteralPath $Results -Encoding ASCII
    continue
  }
  $free = FreeKB
  $sw = [Diagnostics.Stopwatch]::StartNew()
  if ($dwg -eq '-') { $argv = @('/s', ('"' + $scr + '"'), '/l', 'en-US') }
  else { $argv = @('/i', ('"' + $dwg + '"'), '/s', ('"' + $scr + '"'), '/l', 'en-US') }
  $p = Start-Process -FilePath $Accore -ArgumentList $argv -PassThru -NoNewWindow `
    -RedirectStandardOutput ($outp + '.out') -RedirectStandardError ($outp + '.err')
  $null = $p.Handle
  $status = 'OK'
  if (-not $p.WaitForExit($TimeoutSec * 1000)) {
    $status = 'TIMEOUT'
    try { Stop-Process -Id $p.Id -Force } catch {}
    try { $p.WaitForExit(10000) | Out-Null } catch {}
  }
  $sw.Stop()
  $code = ''
  try { $code = $p.ExitCode } catch {}
  "$id`t$status`t$code`t$([int]$sw.Elapsed.TotalSeconds)`t$free`t$($p.Id)" | Add-Content -LiteralPath $Results -Encoding ASCII
}
"DRIVER DONE`t$([int][Math]::Ceiling($script:waited))" | Add-Content -LiteralPath $Results -Encoding ASCII
