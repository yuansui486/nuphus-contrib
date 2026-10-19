; installer-hooks.nsh — Nuphus 安装器钩子
;
; 作用：安装/升级完成后广播一次 SHCNE_ASSOCCHANGED，强制 Explorer 重建图标缓存。
;
; 为什么需要：Windows 的图标缓存以「文件路径 + 时间戳/大小」为键。安装包替换
; nuphus.exe 的瞬间若 Explorer 恰好去读图标，会读到不完整的 PE 资源并回退成
; 通用白纸图标，然后一直沿用。Tauri 的 NSIS 模板创建快捷方式后不做任何刷新，
; 是否重读完全依赖 Explorer 自身，这里补上确定性动作。
;
; 覆盖范围：首次安装、手动升级、Tauri updater 自动更新（同样走 NSIS）。
; 常量：SHCNE_ASSOCCHANGED = 0x08000000，SHCNF_IDLIST = 0x0000。
; 该 API 为用户级，不需要管理员权限。

!macro NSIS_HOOK_POSTINSTALL
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x0000, i 0, i 0)'
!macroend
