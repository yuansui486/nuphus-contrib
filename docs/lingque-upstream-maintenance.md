# 灵雀上游同步维护手册

## 目标与边界

灵雀以 `edition/lingque` 为唯一产品分支，持续接收 `mrpulor-gh/nuphus/main`
更新；保留鉴权、租户项目隔离、工作流工作台、MCP、定时任务和产品品牌。
继续复用上游画布、编译器和执行器，不维护第二套执行语义。

- 产品代码和 CI 位于 `yuansui486/nuphus-contrib`。
- 贡献仓库默认 `main` 只增加工作台 dispatcher，不合入灵雀产品代码。
- 不修改私有 Synapse 主分支，不推送上游仓库。
- 源码自动同步不等于自动发布安装包；原版更新渠道仍禁用。
- 保留安装标识、凭据存储标识、MCP 命令和数据目录，不自动复制原版密钥。
- 所有维护操作使用 E 盘；不关闭用户应用、不清除用户数据、不提交本地 `design/`。

## 2026-09-27 故障基线

| 对象 | 排查时提交或状态 |
| --- | --- |
| 本地 `edition/lingque` | `e522169`，包含鉴权、浅色默认主题和逐流图标 |
| 远程 `edition/workflow-workbench` | `d95678ae3b5a91ed6950cf9a20006b156283fdb5` |
| 本轮固定上游 | `8b65dcffbaaea8d7e7e107cde70f941bf4627078` |
| 默认分支 dispatcher | `9f9035e`，仍指向旧产品分支 |

[同步失败记录](https://github.com/yuansui486/nuphus-contrib/actions/runs/36321234669)
在合并阶段失败，涉及交接目录、桌面端知识库、工具端知识库和工具注册四处冲突，尚未进入编译。

[独立 PR 检查](https://github.com/yuansui486/nuphus-contrib/actions/runs/36263227455)
没有执行 job，GitHub 注释为审批过期，不是编译错误。同一候选的
[同步内置验证](https://github.com/yuansui486/nuphus-contrib/actions/runs/36263216729)
已通过 Windows、Ubuntu、macOS ARM64 和前端测试并完成推进。

本地已创建 `backup/lingque-before-sync-20260927` 和
`backup/workbench-remote-20260927` 备份引用。旧远程分支保留作历史参照，
不能将缺少新鉴权的旧产品直接当作新版运行回退目标。

## 首次迁移执行顺序

1. 检查 worktree、运行中的应用和远程状态，记录完整 SHA。保留本地未提交内容。
2. 从本地灵雀创建独立修复分支 `fix/lingque-upstream-sync`；先合并远程旧产品历史，
   再合并固定的上游 SHA。使用带 DCO 的中文合并提交，不重置、不整批覆盖文件。
3. 解决冲突并完成下述本地验证。公开推送前检查待发布内容，禁止包含令牌、密码、测试账号、
   本地运行日志和客户数据。UA2 的公开接口地址不属于客户端密钥。
4. 将修复结果快进到本地 `edition/lingque`，首次推送同名远程分支；push 触发产品 CI。
   新分支三平台验证全部通过前，不切换默认分支的定时入口。
5. 在默认 `main` 的独立维护分支，仅修改 dispatcher 的两个产品引用为 `edition/lingque`，
   用独立 `ci` 提交发布。发布前重新获取远程 `main`，保留其他人的提交，不 force push。
6. 手动运行 dispatcher 的 `sync`，再观察下一次计划调度。无更新应正确退出；有更新必须
   对固定候选验证后才能推进。旧分支、旧 PR 不自动删除。
7. 将最终 SHA、CI 链接和调度验收结果追加到本文执行记录，再对齐本地产品分支。

常用只读诊断（在正确工作目录执行）：

```powershell
git status --short --branch
git remote -v
git log -5 --oneline
gh run list -R yuansui486/nuphus-contrib --limit 10
gh run view <run-id> -R yuansui486/nuphus-contrib --log-failed
gh run view <run-id> -R yuansui486/nuphus-contrib --json jobs,conclusion,headSha,url
gh run list -R yuansui486/nuphus-contrib --workflow workbench-dispatch.yml --event schedule --limit 5
```

## 合并原则

### 路径和迁移

使用上游统一的 `plugin_root`、知识库文档根和索引目录入口；灵雀在底层保持独立数据根。
知识库两个调用端不得自行从文档目录反推索引路径。旧索引允许重建，原始文档不删除。

交接目录沿用上游按 Agent 目录合并的迁移方式，但来源名称通过产品 profile 注入：
普通版只使用 `.nuphus`，灵雀只使用 `.nuphus-workbench`。
覆盖程序目录、工作目录和数据目录下的旧嵌套布局。目标已有同名 Agent 时保留目标，
不覆盖新内容、不删除旧副本。新增回归测试同时放置两个版本的数据，确认只迁入灵雀数据。

### 工具和授权

保留上游新的工具注册名、专属超时桶以及“超时不等于取消”的说明；
保留灵雀执行前授权检查和传递到阻塞线程的工作流上下文。
不要为通过编译恢复旧的 `planner_complete` 注册名或丢弃 `planner_archive`。

检查自动合并的原生命令表仍由统一授权入口包裹，上游新增业务命令不能进入登录前白名单。
工作流、定时任务、生成、IPC、HTTP 和 MCP 继续调用现有授权及项目隔离层。
前端生成事件、画布适配接口、托盘与打包钩子也是每次同步的检查重点。

## 持续同步协议

GitHub 计划任务只从默认分支执行，所以保留轻量 dispatcher，调用产品分支内的
`workbench-sync.yml` 与 `workbench-release.yml`。计划表达式为每小时第 17 分钟；
GitHub 可能延迟执行，不能承诺准确到分钟或每次都准时启动。

1. 固定产品基线和本轮上游 SHA，建立候选合并分支及 PR。
2. 候选必须同时包含产品基线和上游 SHA。复用已有候选时也检查祖先关系。
3. 直接调用可复用 CI，每个 job 核对实际 checkout 的 SHA。机器人 PR 的独立检查可能需要审批，
   不将它作为同步内置验证的替代，也不绕过 GitHub 审批。
4. 前端和三个原生平台全部成功后，重新检查远程产品基线；变化则不推进，下一轮重建候选。
5. 仅快进到同一个已测试提交。禁止 force push、自动选择冲突一侧或用跳过测试冒充成功。
6. 无新提交时跳过验证和推进是正常结果；有候选时验证未成功就不能推进。

摘要必须展示基线、上游、候选及 prepare/verify/promote 状态。冲突 job 另外列出冲突文件。
人工 PR 的检查按正常审批流程处理；历史审批过期记录不会因修复代码自动变绿。

## 验证清单

- 前端：TypeScript、ESLint、Prettier、完整 Vitest；原版和工作台两个入口分别构建。
- 服务：`cargo test -p nuphus-workbench --features gateway`，以及同包全目标严格 Clippy。
- 原生：Rust fmt、工作流、工具缓存、路径工具、工具注册、交接迁移、工作台命令测试；
  分别检查有／无 `workbench` 特性的桌面入口。
- 平台：Windows、Ubuntu、macOS ARM64；保留 MCP 桥接构建、Windows 安装生命周期及 Mac 桌面契约测试。
- 授权：使用现有模拟测试验证未登录拒绝、过期／撤销、退出竞态和租户隔离，不在 CI 调用生产租户。
- 交互：后续需要新安装包时，验证登录、浅色默认主题、逐流图标、工作流列表、画布、调度和外部接入；
  Mac Accessibility 与真实应用操作仍需 macOS 15+ 真机，编译成功不替代此项。

磁盘空间不足时不复制大型 target，不删除用户文件或改用 G 盘。复用现有依赖缓存，
将完整平台编译交由 CI，并明确记录本地与 CI 分别执行了哪些检查。

## 故障分类与回退

| 现象 | 处理 |
| --- | --- |
| prepare 合并冲突 | 产品分支保持不变；人工修复候选并重新验证，重复点击重跑无效 |
| 类型／编译／测试失败 | 按第一个实际错误修复；不删测试、不关闭检查 |
| jobs 为空且提示审批过期 | 检查 GitHub 注释及内置 verify 结果；按正常权限审批，不当成代码错误 |
| 下载／runner 瞬时故障 | 确认候选 SHA 不变后重试失败任务 |
| 产品分支在验证期间变化 | 保留候选 PR，下一轮基于新产品版本重新验证 |
| dispatcher 分支不存在 | 先发布并验证产品分支，再修改入口，不能倒置顺序 |

需要暂停时只暂停贡献仓库的工作台 dispatcher，不停用户应用、不停其他工作流。
已合入代码有问题时创建正常 revert／修复提交，重新验证，禁止改写共享历史。
不自动切回缺少鉴权的旧产品，不逆向改写数据库，不因源码回退清空凭据或 device_id。

安装包通过 dispatcher 的 `build` 单独触发，绑定固定产品 SHA 并先验证。
Mac 预览包仍为 ad-hoc 签名，并非 Apple 公证发行包；本次同步不改变发布策略。

## 执行记录

- 2026-10-08：同步运行 37715661625 在准备阶段发生合并冲突，未进入编译。
  产品基线为 `15c04a1caafcd0fab4d668ff8232c4692cae7a45`，固定上游为
  `8bb370fa430c5a1258bf28c30ea50685703aac9c`（47 个待同步提交）。
  在 E 盘独立候选工作树解决两处 Cargo 配置与桌面入口冲突：保留灵雀 feature、
  加密依赖和初始化，接收上游 asset protocol、截图调整及旧录制接口清理。
  上游 task CLI 在灵雀中明确拒绝，避免绕过鉴权和租户工作台入口。
  同步工作流增加中文故障说明和复用修复候选的操作步骤，不改变验证与快进门禁。
  自动更新接入另行提交；本次源码同步不发布安装包。
- 2026-09-29：自动同步在 prepare 阶段遇到 `useTheme.tsx` 与 `CanvasPage.tsx` 冲突，
  并非编译失败；产品仍为 `79b78a1e132bec607bbe3d854f45d52f9d04ac16`。
  在 E 盘独立 worktree 合并固定上游 `8edefc42c6f329601010437620f04a51ec8447f9`，
  未移动原工作目录中未提交的生成文本、知识库、安装品牌等改动。
  保留上游皮肤恢复及意图表单的工具栏位置，同时保留灵雀默认浅色和 `backend.generation` 限制。
  新增自定义主题与浅色默认共存、原版直接打开意图表单、外部模式禁用生成的回归。
  本地完整前端测试 142 文件 / 1,435 项通过；跨平台结果以候选同步 CI 为准。
  首轮候选 CI 随后发现上游置顶会话函数未从 `config` 门面导出，桌面端报 E0425；
  补齐 `normalize_pinned_sessions` 导出并在归一测试中覆盖桌面消费的公开路径，再验证新候选。
  第二轮 macOS ARM64、Ubuntu 和前端通过；Windows 的等待提示测试依赖 900ms 硬截断而偶发失败。
  将该测试改为事件驱动：收到实际暂停事件后续跑并检查成功，临时目录独立，保留超时防挂死。
  不修改生产等待逻辑、不删除断言，修正后重新执行三平台候选验证。
- 2026-09-27：在 E 盘建立隔离修复目录，完成远程旧产品历史与上游 `8b65dcf` 合并；
  保留灵雀授权和工作流上下文，增加交接迁移隔离测试。
- 本地验证：TypeScript、ESLint（无错误，保留既有警告）、Prettier、Rust fmt、
  工作台服务 Clippy 通过；前端 125 个测试文件、1,211 项测试通过；Rust 服务及桥接器 60 项测试通过。
  原版与工作台前端均成功构建。工作流 YAML 经 actionlint v1.7.12 检查通过。
- 产品提交 `42004e4a6e175bee8143c33b956db0bc7167afc6` 已首次发布到 `edition/lingque`；
  [首轮 CI](https://github.com/yuansui486/nuphus-contrib/actions/runs/36324210621)
  前端、Windows、Ubuntu、macOS ARM64 全部成功。
- 完成上述验证后，默认分支以 `8494fb00711df8cc35ba4d8fd5bbc1f53e574859` 切换 dispatcher；
  仅改两处产品引用。该提交的
  [默认分支 CI](https://github.com/yuansui486/nuphus-contrib/actions/runs/36325616992) 也已通过。
- 手动触发的
  [新同步链路验收](https://github.com/yuansui486/nuphus-contrib/actions/runs/36325617562)
  发现上游新提交 `1399ddf0237d532feb9cc0af39cbe868aae29133`，自动建立
  [候选 PR #4](https://github.com/yuansui486/nuphus-contrib/pull/4)，候选 SHA 为
  `d1c3b2eda78e99b47a06f3092e9c09de8c6a918f`。本次对照仓库、SHA 和差异后，
  按 GitHub 正常流程批准了这一条独立 PR 检查；未修改审批策略或配置自动批准。
- 新同步链路的 prepare、三平台 verify、promote 和 summary 全部成功；
  远程产品分支已快进到同一个候选 `d1c3b2eda78e99b47a06f3092e9c09de8c6a918f`，
  PR #4 自动标记合并。没有手工替代 promote，没有发布安装包。
- 随后的[无更新验收](https://github.com/yuansui486/nuphus-contrib/actions/runs/36327599391)
  成功：prepare 和 summary 正常结束，verify／promote／build 按预期跳过，不产生重复合并。
- [候选的独立 PR CI](https://github.com/yuansui486/nuphus-contrib/actions/runs/36325632626)
  经正常审批后前端及三个原生平台全部通过；同步自身的 verify 仍独立执行，不以此替代。
- 定时入口已启用并保持 `17 * * * *`。上述两次端到端验收均由手动 dispatch 触发；
  切换后的下一次自然 `schedule` 事件尚待观察，不将手动运行冒充定时触发。
