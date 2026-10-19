# Nuphus 工具脚本

## 图标生成

图标真相源：`src-tauri/icons/icon-source.svg` + `render-icons.mjs`（headless Chrome 渲染，用法见该文件头注释）。

## 开发辅助

| 脚本 | 说明 |
|------|------|
| `dev-restart.bat` | 开发重启 — 重启前端 dev server |

## 使用注意

- 所有 `.ps1` 脚本为 Windows PowerShell
- ~~涉及 Nuphus 进程管理的脚本（`restart_nuphus`、`build_restart`、`candle_rebuild`）必须由外部守护进程调用，不可由 Leader 直接执行~~ **2026-07-11 移除**：`restart_nuphus.ps1` 与 `build_restart.ps1` 因存在 `Stop-Process` 自杀风险且无自动调用方（已 grep 全项目无引用）已删除。Nuphus 重启请直接通过开发环境（`cd frontend && npx tauri dev`）或人工触发 cargo build 完成。
- 可执行权限：PowerShell 需 `-ExecutionPolicy Bypass`