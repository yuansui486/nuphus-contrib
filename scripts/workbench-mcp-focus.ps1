param([Parameter(Mandatory=$true)][string]$TestRoot)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class WorkbenchFocusProbe {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
}
'@
# Only observe the applications created inside THIS test scope, not user windows.
$scope = Join-Path ([IO.Path]::GetFullPath($TestRoot)) ('focus-' + [Guid]::NewGuid().ToString())
$info = [Diagnostics.ProcessStartInfo]::new()
$info.FileName = (Get-Command node).Source
$info.ArgumentList.Add((Join-Path $PSScriptRoot 'workbench-mcp-smoke.mjs'))
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
$info.Environment['WORKBENCH_TEST_ROOT'] = $scope
$child = [Diagnostics.Process]::Start($info)
$output = $child.StandardOutput.ReadToEndAsync()
$errors = $child.StandardError.ReadToEndAsync()
$violations = [Collections.Generic.HashSet[int]]::new()
try {
  while (-not $child.HasExited) {
    $window = [WorkbenchFocusProbe]::GetForegroundWindow()
    [uint32]$foregroundProcess = 0
    [WorkbenchFocusProbe]::GetWindowThreadProcessId($window, [ref]$foregroundProcess) | Out-Null
    $candidate = Get-Process -Id $foregroundProcess -ErrorAction SilentlyContinue
    if ($candidate -and $candidate.ProcessName -eq 'nuphus-workbench' -and
        $candidate.Path.StartsWith($scope + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
      $violations.Add([int]$foregroundProcess) | Out-Null
    }
    Start-Sleep -Milliseconds 30
  }
  $output.Result
  $errors.Result
  if ($child.ExitCode -ne 0) { throw 'Native MCP acceptance failed' }
  if ($violations.Count) { throw ('Background Workbench took focus: ' + ($violations -join ', ')) }
  'PASS: automatic tray hosts did not take foreground focus'
} finally {
  if (-not $child.HasExited) { $child.Kill() }
  $child.Dispose()
}
