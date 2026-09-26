# Workbench MCP packaging

Only `tauri.workbench.conf.json` enables these hooks. Upstream installers are unchanged.
The build hook compiles the bridge before Tauri packaging, stages the target-suffixed
sidecar, and bundles it beside the application (macOS: `Contents/MacOS`).

Windows pre-install/uninstall holds a writer handle to `.workbench-upgrading` before
the normal Tauri application-close check. The MCP launcher checks that handle before
auto-starting a host. Closing/cancelling the installer releases it; a stale marker
does not prevent launch. Post-install/uninstall removes the marker.

Cleanup only terminates MCP processes matching the current user and exact executable
path in this installation. It verifies ownership/path and terminates via the same
process handle, then checks that the binary can be opened exclusively without writing.
Failure offers Retry/Cancel; silent/passive installation exits with code 10. No model
data, project files, credentials or Agent configuration are removed.

Run `test-workbench-mcp.ps1 -McpExecutable <absolute executable> -TestRoot <scratch directory>`
to test real stdio process cleanup, Unicode/spaced paths and preservation of another
installation. The fixture does not register or install the app. Full interactive
installer prompts and macOS drag-to-Applications replacement still need manual review.
Pause Agent MCP connections during an upgrade, then reconnect using the same path.
