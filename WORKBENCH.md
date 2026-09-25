# Nuphus Workbench

独立、工作流优先的 Nuphus 版本。产品分支为 `edition/workflow-workbench`，
上游是 `mrpulor-gh/nuphus/main`。保留上游原版入口、编译器、画布和执行器，
不维护另一套执行语义，不要求外部 Agent 复用内置 Agent。

## 使用方式

- 首页是项目中的工作流列表。新建工作流默认显示左侧内置生成面板：引导式表单或一句话描述；
  使用本版本「模型设置」配置的工作流模型，生成和修改草稿，校验通过后发布，不自动运行。
- 「外部 Agent / 手工」模式隐藏内置生成面板。外部新建的工作流默认使用此模式。
  显式切换模式不删除画布内容；确定性编排和执行不要求配置模型。
- 保存草稿允许未完成的内容；运行先校验和发布固定版本。单步调试可指定测试值，
  沿用原生调试器的单节点／运行到节点行为，执行会产生真实副作用。
- 外部编辑与应用编辑共享版本。内容和布局各自有修订号；干净画布自动刷新，
  有未保存编辑时提示冲突，不悄悄覆盖。画布连线由原生嵌套 IR 推导，
  修改执行顺序、容器和条件分支即修改编排，不额外保存可任意连接的第二套 DAG。
- 运行记录展示真实运行编号、状态、输入、结果和人工等待请求；画布调试面板查看步骤证据。
  最小化或关闭主窗口保留托盘服务，退出托盘才退出应用。重启后未完成运行标记中断，不自动重放。

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

在「外部接入」为当前项目创建令牌。令牌只显示一次，可随时撤销；
它不同于模型 API Key。HTTP、MCP 和应用内画布调用同一个服务层。

- HTTP：`POST http://127.0.0.1:47731/api/v1/{operation}`，JSON 参数，Bearer 认证。
- 发现：`GET /api/v1/discover` 返回当前客户端可用操作及 JSON Schema。
- MCP Streamable HTTP：`http://127.0.0.1:47731/mcp`，相同 Bearer；工具名将点换为下划线。
- stdio：运行 `nuphus-workbench-mcp`，环境变量 `NUPHUS_WORKBENCH_TOKEN`；
  `NUPHUS_WORKBENCH_URL` 可覆盖本机服务地址。桥接进程不启动 Agent、不单独执行工作流。
- 持久事件：`run.events` 或 `/api/v1/events?project_id=…&after=…`；
  断线后带最后处理的游标继续读取，不以传输会话 ID 代替运行 ID。
- MCP resources：项目索引，以及项目、画布、版本和运行记录模板。资源读取同样校验项目权限。

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
| 工作台壳 | `frontend/src/workbench` | 工作流列表、模式选择、内置生成、外部客户端与运行记录 |
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
接口重试、修订冲突、外部模式不调用内置 Agent、项目隔离、人工确认 ID、断线游标都需回归。

`scripts/workbench-smoke.mjs` 可对真实 Windows WebView2 和原生宿主执行无模型冒烟：
创建等待工作流、画布同步、发布运行、幂等重试、单节点调试、断点暂停、人工继续和取消。
仅使用独立测试项目；令牌在结束时撤销。测试需要本地临时 Tauri 配置给各窗口设置
`additionalBrowserArgs: "--remote-debugging-port=9227"`；不要把调试端口加入产品配置。
启动时设置独立 `NUPHUS_WORKBENCH_DATA_DIR`、`NUPHUS_WORKBENCH_PORT`，然后执行
`node scripts/workbench-smoke.mjs`。测试仍会保留其工作流与运行证据，方便检查。

仍须在真实 Windows 和 macOS 15+ 上验收权限、窗口/浏览器控制、输入落点及安装并存，
使用本版本配置的模型验收自然语言生成质量。编译和模拟测试不代替这些验收。
当前不接入工作台的定时调度，不提供无桌面系统守护进程，也不承诺所有应用后台无焦点操作。
