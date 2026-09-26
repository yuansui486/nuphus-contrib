param([Parameter(Mandatory=$true)][string]$McpExecutable, [Parameter(Mandatory=$true)][string]$TestRoot)
$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetFullPath($TestRoot)) ('installer-' + [Guid]::NewGuid().ToString())
$install = Join-Path $root ('Install ' + [char]0x5de5 + [char]0x5177)
$other = Join-Path $root 'Other install'
$owned = @()
try {
  foreach ($directory in @($install, $other)) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $exe = Join-Path $directory 'nuphus-workbench-mcp.exe'
    Copy-Item -LiteralPath $McpExecutable -Destination $exe
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $exe
    $info.Arguments = 'serve'
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $owned += [Diagnostics.Process]::Start($info)
  }
  Start-Sleep -Milliseconds 400
  $script = Join-Path $PSScriptRoot 'workbench-stop-mcp.ps1'
  & "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $script -InstallDir $install
  if ($LASTEXITCODE -ne 0) { throw 'Cleanup script failed' }
  if (-not $owned[0].WaitForExit(2000)) { throw 'Owned MCP process was not stopped' }
  if ($owned[1].HasExited) { throw 'Another installation was incorrectly stopped' }
  $file = [IO.File]::Open((Join-Path $install 'nuphus-workbench-mcp.exe'), 'Open', 'ReadWrite', 'None')
  $file.Dispose()
  Write-Output "PASS: exact installation cleanup, Unicode/spaces path, other installation preserved. Evidence: $root"
} finally {
  foreach ($process in $owned) {
    if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit(2000) | Out-Null }
    $process.Dispose()
  }
}
