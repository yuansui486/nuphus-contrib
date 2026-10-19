/**
 * hotModels — 模型设置页「热门模型」快捷 chip 的内置清单。
 *
 * chip 存两段信息：展示名（label）+ 用于**子串匹配**的关键词片段（matches）。
 * 为什么不直接存完整模型 id：模型多经中转站接入，id 前缀会变
 * （如 `anthropic/claude-opus-4-20250514`），只有片段才能命中；
 * 点击 chip = `setFilterInput(matches.join(' '))`，复用列表既有的子串筛选。
 *
 * `matches` 是**数组**而非单个字符串：筛选引擎按空格分词做「任一命中」，
 * 一个 chip 因此可以带多个片段（多关键词 OR）。当前数据用宽品牌片段单条即覆盖
 * 该品牌主力全家（`gpt` 同时命中 gpt-4o / gpt-5；`deepseek` 同时命中
 * deepseek-chat / deepseek-reasoner / deepseek-v4），故暂时都只放一个片段；
 * 数组形态留着是为了「同一 chip 需要跨命名变体」时不必改引擎与调用方。
 *
 * 片段故意取**宽品牌名**而非具体型号：早先版本用 `gpt-5` / `deepseek-v4` /
 * `claude-opus` 这类窄片段，命中不到主力模型（deepseek-chat / deepseek-reasoner、
 * gpt-4 全家族），上游因此否掉了那一版。窄片段属于已知回归，不要写回来；
 * `hotModels.test.ts` 用真实 id 池守着这条线。
 *
 * 纯展示常量，不参与模型解析 / 切换 / 偏好读写。
 */
export type HotModelChip = {
  /** chip 上的展示名（品牌名，无需 i18n）。 */
  label: string
  /** 填进筛选框的关键词片段（空格分隔，子串匹配，任一命中即显示该模型）。 */
  matches: readonly string[]
}

export const HOT_MODELS: readonly HotModelChip[] = [
  { label: 'GPT', matches: ['gpt'] },
  { label: 'Claude', matches: ['claude'] },
  { label: 'Gemini', matches: ['gemini'] },
  { label: 'DeepSeek', matches: ['deepseek'] },
  { label: 'Qwen', matches: ['qwen'] },
  { label: 'Grok', matches: ['grok'] },
]
