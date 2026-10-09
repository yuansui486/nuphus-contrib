# 灵雀独立自动更新

源码同步与稳定发布相互独立：每小时同步上游，在三平台 CI 通过后快进产品分支；不会自动发布安装包或 OSS 稳定清单。

## 产品边界

- 安装身份、主程序、数据目录保持不变：`io.github.yuansui486.nuphusworkbench` / `nuphus-workbench`。
- 产品版本只取 `src-tauri/tauri.workbench.conf.json`，不跟随上游 Cargo 或前端包版本。
- 标签使用 `lingque-v<版本>`，不触发上游 `v*` 发布。
- 更新地址：`https://tct12.oss-cn-beijing.aliyuncs.com/12box/lingque/updates/stable/latest.json`。
- 支持 Windows x64、Mac ARM64；其他平台不会回退到 Nuphus 或私匣更新源。
- 独立签名密钥，OSS 账号可复用，但权限应限制到 `12box/lingque/updates/*`。客户端不包含 AccessKey。

## 维护者发版

1. 调整 Workbench 版本并添加 `docs/lingque-releases/<版本>.md`，提交通过 CI。
2. 创建指向该提交的 `lingque-v<版本>` 标签。
3. 在 Workbench dispatcher 中选择 `build`，填写同一个提交和标签，保持 `publish=false`。
4. 等待前端、Windows、Ubuntu、Mac ARM64 检查，以及两平台安装包构建和原生安装验收全部成功。下载并人工检查最终包；Mac 还需真机验证触控板、Finder 和系统权限。
5. 再次运行 `build`，填写相同提交、标签以及上一步的 `reuse_build_run`，勾选 `publish=true` 才会发布。首次更新器接入版本需手动安装一次。

Secrets：`TAURI_SIGNING_PRIVATE_KEY`（灵雀独立私钥，本项目使用空密码）、`OSS_ACCESS_KEY_ID`、`OSS_ACCESS_KEY_SECRET`。曾在聊天中提供过的 OSS 凭据应先轮换，不能提交到仓库。OSS 需要对象 Get/Put 权限和公共读取，无需删除桶或模型目录权限。

构建回执记录实际产品 SHA、版本、平台、运行 ID 和产物哈希，不以 dispatcher 的默认分支 SHA 代替。复用仅接受同仓库发布流程中 13 天内完整成功的手动构建；Actions 产物保留 14 天。上传失败请复用原先成功的只构建运行，不重新编译后覆盖同一版本。

发布先验证签名，再上传不可变版本对象并用 HEAD 核对大小与 SHA-256 元数据，最后以 ETag 条件切换稳定清单。不同版本的发布串行执行，拒绝降级和同版本不同字节覆盖。完成后只读取小型公开清单，不从 OSS 下载整包或模型验收。

## 用户交互和安装保护

登录前和设置中均可进入“版本与更新”。启动延迟 5 秒、每天检查，可关闭；下载和安装分别由用户确认。支持进度、取消、后台下载以及完整缓存重启后重新验签复用，不支持不完整下载的断点续传。

安装前保存画布内容和布局队列，失败不安装。Rust 原子关闭任务准入，运行、暂停、生成、调度、外部请求、模型准备及配置写入未结束时拒绝更新。安装中建立短期 MCP 标记，避免客户端拉起旧主程序；安装前失败恢复任务准入并保留有效缓存。登录、项目、历史、模型与任务数据不会清理。

Windows 使用官方 Tauri updater 启动 NSIS `/P /UPDATE /R`，只清理当前用户、当前安装目录的 MCP，持续占用有界非零退出。Mac 从已安装的 `.app` 更新，拒绝 DMG 和 App Translocation；先完成 ad-hoc 系统签名，再生成归档，验收 DMG 与归档的所有文件一致。更新签名不等于 Apple 公证或 Windows Authenticode。

## 开发验证

- `npm test --prefix tools/lingque-release`：离线签名、两平台清单、OSS 模拟上传和构建来源核对。
- `cargo test -p nuphus-workbench --features gateway`：任务准入并发、真实 HTTP/IPC、调度、短期更新标记。
- `cargo test -p nuphus-desktop --features workbench --bin nuphus workbench::updates`：下载失败、取消、签名、缓存、偏好；Mac 额外实际替换微型应用并运行新版。
- Windows：先构建 `lingque-updater-probe` example 和 MCP；用 Windows PowerShell 5.1 运行 `src-tauri/installer/tests/run.ps1`，传入 `-McpExecutable` 和 `-NativeUpdaterProbe`。正式构建后自动使用 Tauri 生成的 NSIS 模板；测试不会覆盖用户安装。
- 临时目录可通过 `TEMP`、`TMP` 指定到 E 盘；`CARGO_TARGET_DIR` 也使用 E 盘。`NUPHUS_BUILD_SKIP_MODELS=1` 使 CI 不下载模型。灵雀编译不向已安装应用的数据目录写入模型。

私钥要离线备份，后续版本必须沿用。离线测试夹具的公钥与生产公钥不同；`generate-fixtures.mjs` 只生成微型测试包，不保存私钥，不用于生产签名。
