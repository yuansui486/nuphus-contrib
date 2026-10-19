import { describe, expect, it } from 'vitest'
import { HOT_MODELS, type HotModelChip } from './hotModels'

/**
 * 真实模型 id 池（含品牌主力与中转站常见形态）。
 * 这些 id 就是用户列表里实际会出现的东西——chip 片段必须在它们身上命中。
 */
const ID_POOL = [
  'deepseek-chat',
  'deepseek-reasoner',
  'deepseek-v4',
  'gpt-4o',
  'gpt-5',
  'claude-opus-4-20250514',
  'claude-sonnet-4-20250514',
  'gemini-2.5-pro',
  'qwen3-max',
  'grok-4',
] as const

/**
 * 每个品牌必须覆盖的「主力全家」。
 * 这是「匹配片段过窄 → 漏主力模型」的防回归核心：早先用 `gpt-5` / `deepseek-v4` /
 * `claude-opus` 这类窄片段时，下面的 id 会成片命中不到，本用例当场变红。
 */
const BRAND_MAINLINE: Record<string, readonly string[]> = {
  GPT: ['gpt-4o', 'gpt-5'],
  Claude: ['claude-opus-4-20250514', 'claude-sonnet-4-20250514'],
  Gemini: ['gemini-2.5-pro'],
  DeepSeek: ['deepseek-chat', 'deepseek-reasoner', 'deepseek-v4'],
  Qwen: ['qwen3-max'],
  Grok: ['grok-4'],
}

/** 与 ModelsPage 筛选同口径：小写、空格分词、子串、任一命中即算命中。 */
function chipHits(chip: HotModelChip, id: string): boolean {
  const lowered = id.toLowerCase()
  const tokens = chip.matches.join(' ').trim().toLowerCase().split(/\s+/).filter(Boolean)
  return tokens.some(tk => lowered.includes(tk))
}

describe('热门模型 chip 清单', () => {
  it('label 非空且互不重复', () => {
    const labels = HOT_MODELS.map(c => c.label)
    expect(labels.length).toBeGreaterThan(0)
    for (const label of labels) expect(label.trim()).not.toBe('')
    expect(new Set(labels).size).toBe(labels.length)
  })

  it('六个主力品牌齐备', () => {
    const labels = new Set(HOT_MODELS.map(c => c.label))
    for (const brand of ['GPT', 'Claude', 'Gemini', 'DeepSeek', 'Qwen', 'Grok']) {
      expect(labels.has(brand), `缺少品牌 chip：${brand}`).toBe(true)
    }
  })

  it('matches 非空、无空串 / 纯空白 / 内嵌空白片段', () => {
    for (const chip of HOT_MODELS) {
      expect(chip.matches.length, `chip「${chip.label}」的 matches 为空`).toBeGreaterThan(0)
      for (const fragment of chip.matches) {
        expect(fragment.trim(), `chip「${chip.label}」含空片段`).not.toBe('')
        // 片段会被 join(' ') 后按空格分词：片段自身带空白会静默裂成两个关键词
        expect(/\s/.test(fragment), `chip「${chip.label}」片段含空白：${fragment}`).toBe(false)
      }
    }
  })

  it('每个 chip 至少命中该品牌主力全家（防「片段过窄漏主力」回归）', () => {
    for (const chip of HOT_MODELS) {
      const mainline = BRAND_MAINLINE[chip.label] ?? []
      expect(mainline.length, `chip「${chip.label}」缺少主力 id 用例`).toBeGreaterThan(0)
      for (const id of mainline) {
        expect(ID_POOL as readonly string[]).toContain(id)
        expect(chipHits(chip, id), `chip「${chip.label}」未命中主力模型 ${id}`).toBe(true)
      }
    }
  })

  it('品牌片段不会误伤：跨品牌 id 不会被无关 chip 命中（抽样）', () => {
    const gpt = HOT_MODELS.find(c => c.label === 'GPT')!
    expect(chipHits(gpt, 'claude-opus-4-20250514')).toBe(false)
    expect(chipHits(gpt, 'deepseek-chat')).toBe(false)
  })
})
