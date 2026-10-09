; Workbench-only hooks. Never change upstream install behaviour.
!ifndef WORKBENCH_MCP_HOOKS
!define WORKBENCH_MCP_HOOKS
!include FileFunc.nsh
!define WB_STOP_SCRIPT "${__FILEDIR__}\workbench-stop-mcp.ps1"
Var WorkbenchUpgradeHandle

!macro WorkbenchCleanupFunction PREFIX
Function ${PREFIX}WorkbenchStopMcp
  InitPluginsDir
  File /oname=$PLUGINSDIR\workbench-stop-mcp.ps1 "${WB_STOP_SCRIPT}"
  wb_retry:
    nsExec::ExecToStack /TIMEOUT=15000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\workbench-stop-mcp.ps1" -InstallDir "$INSTDIR\."'
    Pop $0
    Pop $1
    StrCmp $0 "0" wb_done
    IfSilent wb_abort 0
    ClearErrors
    ${GetOptions} $CMDLINE "/P" $1
    ${IfNot} ${Errors}
      Goto wb_abort
    ${EndIf}
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Please pause Nuphus Workbench MCP in your Agent, then retry. The MCP executable is still in use or cannot be accessed. No application data will be removed." IDRETRY wb_retry
  wb_abort:
    SetErrorLevel 10
    ; Passive updater has nobody to dismiss an Abort error page.
    Quit
  wb_done:
FunctionEnd
!macroend
!insertmacro WorkbenchCleanupFunction ""
!insertmacro WorkbenchCleanupFunction "un."

!macro WorkbenchBeginUpgrade
  CreateDirectory "$INSTDIR"
  ClearErrors
  ; FileOpen keeps a writer handle without write sharing. MCP probes that handle
  ; before auto-start. Cancelling closes it, so a stale marker never blocks launch.
  FileOpen $WorkbenchUpgradeHandle "$INSTDIR\.workbench-upgrading" w
  ${If} ${Errors}
    SetErrorLevel 10
    Quit
  ${EndIf}
  FileWrite $WorkbenchUpgradeHandle "Workbench upgrade in progress"
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
!macroend
!macro WorkbenchEndUpgrade
  FileClose $WorkbenchUpgradeHandle
  Delete "$INSTDIR\.workbench-upgrading"
!macroend
!macro NSIS_HOOK_PREINSTALL
  !insertmacro WorkbenchBeginUpgrade
  Call WorkbenchStopMcp
!macroend
!macro NSIS_HOOK_POSTINSTALL
  !insertmacro WorkbenchEndUpgrade
!macroend
!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro WorkbenchBeginUpgrade
  Call un.WorkbenchStopMcp
!macroend
!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro WorkbenchEndUpgrade
!macroend
!endif
