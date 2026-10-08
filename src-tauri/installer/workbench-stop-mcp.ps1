param([Parameter(Mandatory = $true)][string]$InstallDir)
$ErrorActionPreference = 'Stop'
try {
  if (-not [IO.Path]::IsPathRooted($InstallDir)) { exit 4 }
  $directory = [IO.Path]::GetFullPath($InstallDir)
  if (-not [IO.Path]::IsPathRooted($directory)) { exit 4 }
  $executable = [IO.Path]::Combine($directory, 'nuphus-workbench-mcp.exe')
  $ownerSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
public static class WorkbenchMcpCleanup {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern bool QueryFullProcessImageName(IntPtr h, uint flags, StringBuilder s, ref int size);
  [DllImport("advapi32.dll", SetLastError=true)]
  static extern bool OpenProcessToken(IntPtr h, uint access, out IntPtr token);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateProcess(IntPtr h, uint code);
  public static void Stop(string path, string sid) {
    foreach (var process in Process.GetProcessesByName("nuphus-workbench-mcp")) {
      using (process) {
        try {
          // Query and terminate through the SAME handle, never an unverified PID.
          var handle = process.Handle;
          var buffer = new StringBuilder(32768); int size = buffer.Capacity;
          if (!QueryFullProcessImageName(handle, 0, buffer, ref size)) continue;
          if (!String.Equals(Path.GetFullPath(buffer.ToString()), path, StringComparison.OrdinalIgnoreCase)) continue;
          IntPtr token;
          if (!OpenProcessToken(handle, 8, out token)) throw new IOException("Cannot verify MCP process owner");
          bool matches;
          try { using (var user = new WindowsIdentity(token)) { matches = user.User.Value == sid; } }
          finally { CloseHandle(token); }
          if (!matches) continue;
          if (!TerminateProcess(handle, 0)) throw new IOException("Cannot stop MCP process");
          process.WaitForExit(1500);
        } catch (InvalidOperationException) { /* process already exited */ }
          catch (System.ComponentModel.Win32Exception) { /* inaccessible other-user process; file check still fails if our binary is locked */ }
      }
    }
  }
}
'@
  $deadline = [DateTime]::UtcNow.AddSeconds(8)
  do {
    [WorkbenchMcpCleanup]::Stop($executable, $ownerSid)
    Start-Sleep -Milliseconds 200
    if (-not (Test-Path -LiteralPath $executable)) { exit 0 }
    try {
      $stream = [IO.File]::Open($executable, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
      $stream.Dispose()
      exit 0
    } catch [IO.IOException] { }
  } while ([DateTime]::UtcNow -lt $deadline)
  exit 2
} catch {
  [Console]::Error.WriteLine($_.Exception.Message)
  exit 3
}
