# Housekeeping for the judge on the machine that has AutoCAD. -Root is the judge's scratch folder.
#   prep   empty the work folder (Root\lanes\judging)
#   forget delete the status and DXF of the given IDs before a RECOVER try
#   pack   gzip every DXF in the work folder so the copy back is small
#   clean  delete the work folder and stop any AutoCAD console left running from the scratch folder
#   stop   stop the judge's driver and its AutoCAD console (only the run that holds the judge lock calls this)
param(
  [Parameter(Mandatory = $true)][ValidateSet('prep', 'forget', 'pack', 'clean', 'stop')][string]$Action,
  [Parameter(Mandatory = $true)][string]$Root,
  [string]$Ids = ''
)
$work = Join-Path $Root 'lanes\judging'
function Ours($name) {
  Get-CimInstance Win32_Process -Filter "Name='$name'" |
    Where-Object { $_.CommandLine -and $_.CommandLine.IndexOf($Root, [StringComparison]::OrdinalIgnoreCase) -ge 0 }
}
switch ($Action) {
  'prep' {
    if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    'PREP'
  }
  'forget' {
    foreach ($id in ($Ids -split ',')) {
      if (-not $id) { continue }
      foreach ($ext in @('.status', '.dxf')) {
        $p = Join-Path $work ($id + $ext)
        if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force }
      }
    }
    'FORGOT'
  }
  'pack' {
    foreach ($f in (Get-ChildItem -LiteralPath $work -Filter '*.dxf' -File)) {
      $in = [System.IO.File]::OpenRead($f.FullName)
      $out = [System.IO.File]::Create($f.FullName + '.gz')
      $gz = New-Object System.IO.Compression.GZipStream($out, [System.IO.Compression.CompressionMode]::Compress)
      $in.CopyTo($gz); $gz.Close(); $in.Close()
      Remove-Item -LiteralPath $f.FullName -Force
    }
    'PACKED ' + @(Get-ChildItem -LiteralPath $work -Filter '*.dxf.gz' -File).Count
  }
  'clean' {
    if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
    $left = @(Ours 'accoreconsole.exe')
    foreach ($p in $left) { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue }
    'CLEANED leftover_consoles=' + $left.Count
  }
  'stop' {
    $driver = Join-Path $Root 'scripts\driver.ps1'
    $drivers = Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" |
      Where-Object { $_.CommandLine -and $_.CommandLine.IndexOf($driver, [StringComparison]::OrdinalIgnoreCase) -ge 0 }
    foreach ($d in $drivers) {
      'STOP_DRIVER ' + $d.ProcessId
      Stop-Process -Id $d.ProcessId -Force -ErrorAction SilentlyContinue
      Get-CimInstance Win32_Process -Filter ("ParentProcessId=" + $d.ProcessId) |
        ForEach-Object { 'STOP_CHILD ' + $_.ProcessId; Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    }
    Ours 'accoreconsole.exe' | ForEach-Object { 'STOP_CONSOLE ' + $_.ProcessId; Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    'STOP DONE'
  }
}
