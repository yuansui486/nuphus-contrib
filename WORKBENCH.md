# Nuphus Workbench

独立、工作流优先的 Nuphus 版本。产品分支为 `edition/workflow-workbench`，
上游是 `mrpulor-gh/nuphus/main`。保留上游原版入口、编译器、画布和执行器，
不维护另一套执行语义，不要求外部 Agent 复用内置 Agent。

## 使用方式

- 首页是项目中的工作流列表。「画布新建」直接创建未命名的空白工作流并打开上游画布，
  无需预先填写名称；之后双击画布标题即可改名。默认在左侧工作流助手底部直接描述任务并发送，
  不必先选择生成方式。「引导填写」可将分阶段描述追加到输入框，检查、补充后再发送。
  使用本版本「设置 → 模型配置」配置的工作流模型，生成和修改草稿，校验通过后发布，不自动运行。
- 顶部保留工作流、运行记录和外部接入入口；项目目录、打开项目、主题和编排模式收进设置。
  自动记住上次项目；目录不可用时回到可用项目并提示。收起助手不会切换编排模式，也不会停止生成或丢失输入；
  窄窗口下助手覆盖在画布左侧，可随时收起以使用完整画布。输入支持 Ctrl / Cmd + Enter 发送。
- 「外部 Agent / 手工」模式隐藏内置生成面板。外部新建的工作流默认使用此模式。
  显式切换模式不删除画布内容；确定性编排和执行不要求配置模型。
- 保存草稿允许未完成的内容；运行先校验和发布固定版本。单步调试可指定测试值，
  沿用原生调试器的单节点／运行到节点行为，执行会产生真实副作用。
- 外部编辑与应用编辑共享版本。内容和布局各自有修订号；干净画布自动刷新，
  有未保存编辑时提示冲突，不悄悄覆盖。画布连线由原生嵌套 IR 推导，
  修改执行顺序、容器和条件分支即修改编排，不额外保存可任意连接的第二套 DAG。
- 工作台不展示语言切换按钮，保留上游语言基础设施和已保存语言，避免分叉公共组件。
- 运行记录展示真实运行编号、状态、输入、结果和人工等待请求；画布调试面板查看步骤证据。
  最小化或关闭主窗口保留托盘服务，退出托盘才退出应用。重启后未完成运行标记中断，不自动重放。

### 定时任务

在工作流卡片点击「设置定时」，或进入画布「更多 → 定时」，使用原版规则编辑器配置
分钟间隔、每天/每周等规则、五字段 Cron、IANA 时区及固定输入；预览接下来三次触发时间。
定时会执行**最新已保存**的工作流，触发时校验并固定本次运行快照；未保存的画布编辑不生效，
最新内容无效时记录失败，不回退执行旧版本。子工作流仍沿用工作台的已发布版本快照规则。

工作台首次启动默认开启工具类别权限；用户显式保存的权限设置仍被保留，原版默认值不变。
桌面/浏览器 RPA 可定时执行，不要求启用内置 AI。应用需要保持运行（可驻留托盘），
桌面操作需要可用的登录会话；退出、休眠或执行资源占用期间不积压任务、不自动补跑。
上次运行仍在执行、暂停或等待人工确认时，本次跳过。恢复后超过 30 秒的逾期触发视为错过，
继续下一计划时间。停用/删除定时不取消已经启动的运行，后者在运行记录中单独取消。

「运行记录 → 定时触发」包含跳过原因、失败原因和运行步骤证据；清理定时历史不删除运行记录。
调度配置及触发记录在各项目数据库中；版本 1 数据库幂等升级到版本 2，旧工作台程序不能再打开升级后的项目。
导入文件中的 schedule 字段不会自动启用任务，需显式保存定时配置。
固定输入加密保存：Windows 使用 DPAPI，macOS/Linux 使用 AES-GCM 和工作台配置目录内权限为 0600 的本机密钥。
这不是对同一系统用户的进程隔离；跨机器复制项目后需重新填写固定输入，不会明文导出或自动转移密钥。

Windows 本机定时验收（不调用模型）：编译后设置 `WORKBENCH_TEST_ROOT` 为专用临时目录，运行
`node scripts/workbench-schedule-smoke.mjs`。脚本新建隔离项目，等待真实一分钟触发，检查等待节点、
只读桌面目标查询、最新保存版本、重启去重和停用/移除。可事先打开标题包含 `schedule-smoke-notepad`
的临时记事本文档；脚本不点击、输入或关闭用户应用，测试记录保留在打印的临时目录。

## 启动与构建

需要与上游相同的 Node、Rust、平台原生依赖及 Windows WebView2。

```powershell
npm ci
npm --prefix frontend ci
npx tauri dev --config src-tauri/tauri.workbench.conf.json --features workbench
```

```powershell
# Windows x64 安装包；macOS Apple Silicon 将 nsis 换成 app 或 dmg
npx tauri build --config src-tauri/tauri.workbench.conf.json --features workbench --bundles nsis
cargo build --release -p nuphus-workbench --features gateway --bin nuphus-workbench-mcp
```

产品名 `Nuphus Workbench`，应用标识 `io.github.yuansui486.nuphusworkbench`。
发行程序名为 `nuphus-workbench`（Windows 带 `.exe`），安装器不会将原版 `nuphus.exe`
当作工作台进程。Cargo 目标仍保留上游名称，由 Tauri 打包时重命名，减少核心合并冲突。
配置、数据库、浏览器用户目录、模型缓存与原版隔离，不自动复制原版模型密钥。
默认服务数据根目录为系统用户数据目录下 `nuphus-workbench`；
`NUPHUS_WORKBENCH_DATA_DIR` 仅覆盖工作台注册表/默认项目，不重定向所有模型配置。
每个项目的数据放在 `<项目>/.nuphus-workbench`。项目 ID 随项目数据库保存。
服务端口默认 47731，可用 `NUPHUS_WORKBENCH_PORT` 设置；开发前端独立使用 5176。
`--background` 隐藏主窗口、保持托盘服务。不启动原版门铃、官方中继与移动端服务。
物理桌面自动化锁仍共享，防止两个版本同时抢占真实鼠标键盘。

## 外部接入

默认使用「外部接入 → 本地应用接入」：复制页面生成的 MCP 配置到 Agent。
安装包包含 `nuphus-workbench-mcp`，无需另装 Python/Node 或设置 PATH。
Agent 通过 stdio 启动它，再经 Windows 当前用户专属命名管道或 macOS/Linux Unix socket 连接工作台，
不依赖 HTTP 端口。工作台未运行时自动启动同安装目录/同 App 包内的主程序到托盘，最长等待 30 秒。
并发客户端共用一个宿主，断开 MCP 不退出工作台、不取消运行；后续调用可重新连接重启后的应用。
无需创建客户端、配置令牌或逐项选择权限。IPC、HTTP、MCP 和应用内画布调用同一个服务层。
本机外部调用默认具有全部项目及全部公共能力（包括注册项目、运行、自动化和人工响应）。
这不是项目权限隔离或 OS 沙箱：信任本机调用者，运行可产生真实桌面和文件副作用。
仅绑定 `127.0.0.1`，拒绝网页 Origin/Fetch-Metadata 和非本机 Host；退出托盘后端口关闭。

- HTTP：`POST http://127.0.0.1:47731/api/v1/{operation}`，JSON 参数，无需认证请求头。
- 发现：`GET /api/v1/discover` 返回当前客户端可用操作及 JSON Schema。
- MCP Streamable HTTP：`http://127.0.0.1:47731/mcp`，无需令牌；工具名将点换为下划线。
- stdio（推荐）：运行 `nuphus-workbench-mcp serve`，旧的无参数启动也兼容。
  「测试连接」会启动实际桥接程序，验证 MCP 初始化、工具清单和宿主状态，不执行工作流。
  显式设置 `NUPHUS_WORKBENCH_URL` 才走旧 HTTP 桥接，仍需手工维护 URL；未设置时走本地 IPC。
  桥接进程不启动 Agent、不单独执行工作流。标准输出仅用于 MCP，诊断使用标准错误。
- 持久事件：`run.events` 或 `/api/v1/events?project_id=…&after=…`；
  断线后带最后处理的游标继续读取，不以传输会话 ID 代替运行 ID。
- MCP resources：项目索引，以及项目、画布、版本和运行记录模板。

历史令牌调用保留兼容：显式发送 Bearer 时仍检查原有范围和撤销状态，不会将失效令牌
悄悄降级成免令牌访问；但免令牌本机入口本身具有全权限，因此令牌不再构成隔离边界。

配置示例（以页面生成的实际安装路径为准）：

```json
{"mcpServers":{"nuphus-workbench":{"command":"C:\\完整安装路径\\nuphus-workbench-mcp.exe","args":["serve"]}}}
```

macOS 程序位于 `Nuphus Workbench.app/Contents/MacOS/nuphus-workbench-mcp`，移动 App 后需重新复制路径。
同一路径更新不需要修改配置；更新期间请暂停 Agent 的 MCP，完成后重新连接。
Windows 安装器协调关闭当前安装的 MCP 程序，并在升级期间阻止自动启动主程序；
文件无法释放时停止安装并提示重试，不跳过文件。取消安装后不会留下永久启动禁用标记。
HTTP 服务失败不会关闭 IPC。本机 IPC 以用户和数据目录区分实例，协议不匹配时提示升级，
请求发送后丢失回复不自动重放。`NUPHUS_WORKBENCH_DATA_DIR` 指定的开发配置会写入页面生成的 MCP 配置，
避免误连普通工作台；默认配置不包含端口或密钥。

真实 MCP 验收：设置专用 `WORKBENCH_TEST_ROOT` 后运行 `node scripts/workbench-mcp-smoke.mjs`。
脚本隔离数据，验证端口占用、双客户端冷启动、画布读写、等待工作流、重启重连和运行去重，
不读取模型密钥、不调用模型、不操作业务文件。可用 `WORKBENCH_TEST_EXE` 和 `WORKBENCH_TEST_MCP`
指定已安装或打包后的程序。
Windows 可用 `./scripts/workbench-mcp-focus.ps1 -TestRoot <临时目录>` 包装同一验收，额外检查自动启动不抢前台焦点。

主要调用链：`project.list → canvas.create/get/update → workflow.validate →
workflow.save → workflow.run → run.get/events/steps`。
输入放在 `workflow.run.inputs`。每次有意启动使用新的 `request_id`，
网络错误后的重试复用原 ID 和参数，避免重复发送消息或写文件。
工具定义、调试、人工确认、并发及安全边界见
[公共接口说明](crates/nuphus-workbench/README.md)。
可运行 [HTTP 示例](examples/workbench-client.mjs)，仅创建并运行等待节点。
远程接入先使用自己管理的认证隧道；本版不直接暴露公网监听。

## 便于持续同步上游的结构

| 层 | 所在位置 | 职责 |
| --- | --- | --- |
| 工作台壳 | `frontend/src/workbench` | 工作流列表、模式选择、内置生成、本机接入状态与运行记录 |
| 画布适配接口 | `CanvasBackend.ts` | 复用上游画布，替换持久化/运行入口；原版默认行为不变 |
| 公共服务 | `crates/nuphus-workbench` | 项目、修订、发布、真实运行 ID、认证、HTTP/MCP |
| 原生宿主适配 | `src-tauri/src/workbench*` | 接上游编译器、执行器、模型与本地工具 |
| 小型核心扩展 | `profile`、`run_context`、`WorkflowAuthoring` | 目录隔离、运行快照/项目上下文、生成工具适配 |

不复制整个画布或执行器。新增能力优先落在独立目录，必须修改上游的地方保留默认实现。
这样不能保证永无冲突，但冲突集中在少数入口，而不是长期重写核心。

### GitHub Actions 配置

独立仓库 `yuansui486/nuphus-contrib`：

1. 推送产品分支及四个 `workbench-*.yml` 工作流。
2. **只**将 `workbench-dispatch.yml` 安装到该仓库默认 `main`；GitHub 定时任务只从默认分支执行。
   不把产品改动合并到默认主分支，也不修改私有 Synapse 的 main。
3. 仓库需允许 Actions 创建 PR，令牌具有 contents/pull-requests 写权限。
4. 每小时第 17 分钟检查上游，先建立带 DCO 签名的候选合并提交和 PR；
   显式调用工作台 CI（GITHUB_TOKEN 创建的 PR 不会自动触发另一个工作流）。
5. 普通版/工作台验证通过且产品分支仍等于测试前基线时，才快进到**同一个已测试提交**。
   冲突、失败或产品分支变化均不自动覆盖。无 force push、无自动选择冲突一侧。
6. 安装包独立手动触发 dispatcher 的 `build`，固定产品 SHA、先跑 CI，
   输出 Windows x64 安装器、macOS ARM64 DMG 和 stdio 桥接器。构建产物保留 14 天。

源码每小时同步不等于每小时发布安装包。自动更新渠道禁用，绝不拉取原版更新覆盖工作台。
正式自动升级需另行配置该产品独立的签名密钥、版本号和发布地址。
Mac 预览包采用 ad-hoc 签名，尚不是 Apple 公证发行包。

## 验证和边界

本地检查：前端 tsc、ESLint、Vitest、两个版本构建；Rust fmt、服务测试/Clippy、
工作流测试和两个版本原生 check。CI 对 Windows、Ubuntu、macOS ARM64 执行。
接口重试、修订冲突、外部模式不调用内置 Agent、项目数据分离、人工确认 ID、断线游标都需回归。
免配置 HTTP/MCP 的全能力发现、外部默认模式、网页访问拒绝和旧令牌兼容均有回归测试。

`scripts/workbench-smoke.mjs` 可对真实 Windows WebView2 和原生宿主执行无模型冒烟：
创建等待工作流、画布同步、发布运行、幂等重试、单节点调试、断点暂停、人工继续和取消。
仅使用独立测试项目，无需创建测试客户端。测试需要本地临时 Tauri 配置给各窗口设置
`additionalBrowserArgs: "--remote-debugging-port=9227"`；不要把调试端口加入产品配置。
启动时设置独立 `NUPHUS_WORKBENCH_DATA_DIR`、`NUPHUS_WORKBENCH_PORT`，然后执行
`node scripts/workbench-smoke.mjs`。测试仍会保留其工作流与运行证据，方便检查。

仍须在真实 Windows 和 macOS 15+ 上验收权限、窗口/浏览器控制、输入落点及安装并存，
使用本版本配置的模型验收自然语言生成质量。编译和模拟测试不代替这些验收。
不提供无桌面系统守护进程，也不承诺所有应用后台无焦点操作。
