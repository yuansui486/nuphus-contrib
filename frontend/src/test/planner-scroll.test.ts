/**
 * planner 弹窗**高度链**契约 —— jsdom 不跑样式级联，computed style 拿不到真实
 * 滚动行为，故用声明级契约钉住「能被滚动」的三个必要条件，实机观感由 devtools 复核。
 *
 * 背景（2026-09-28）：任务详情弹窗（TaskDetailModal，复用 planner.css 的 pm-* 类）
 * 展示长 markdown 摘要时「只能看到开头、无法下拉、无滚动条」。根因是高度链断在
 * `.planner-body`：flex column 子项的 min-height 默认为 auto，内容把 body 撑破
 * `.pm-content` 的 max-height，超出被 content 的 overflow:hidden 裁掉，而 body 自身
 * 的 overflow-y:auto 因 flex:1 无法收缩而没有滚动空间。
 *
 * 本契约钉三件事：
 *   ① 上游 `.pm-content`：flex column + max-height + overflow:hidden（裁切边界）
 *   ② 下游 `.planner-body`：flex:1 + min-height:0 + overflow-y:auto（滚动容器）
 *   ③ 两个消费方（TaskDetailModal / PlannerModal）都经 planner.css 拿到同一形态，
 *      没有谁另起一套弹窗样式
 */
import { describe, expect, it } from 'vitest'

type FsLike = { readFileSync: (path: string, encoding: string) => string }

/** CSS 源码读取（声明级契约用）：vitest 默认不加载样式，直接经 node:fs 读原文 */
async function readStyle(relPath: string): Promise<string> {
  const spec = 'node:fs'
  const fs = (await import(/* @vite-ignore */ spec)) as unknown as FsLike
  const cwd = (globalThis as unknown as { process: { cwd(): string } }).process.cwd()
  return fs.readFileSync(`${cwd}/${relPath}`, 'utf8')
}

// ── 极简 CSS 解析（只认顶层规则；@media/@keyframes 先整体摘掉）──

function stripComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '')
}

/** 摘掉 @at-rule 块（含嵌套大括号）与 @import 行，只留顶层普通规则 */
function stripAtBlocks(css: string): string {
  let out = ''
  for (let i = 0; i < css.length; i++) {
    if (css[i] !== '@') {
      out += css[i]
      continue
    }
    const braceStart = css.indexOf('{', i)
    const semicolon = css.indexOf(';', i)
    if (braceStart === -1 || (semicolon !== -1 && semicolon < braceStart)) {
      i = semicolon === -1 ? css.length : semicolon
      continue
    }
    let depth = 0
    let j = braceStart
    for (; j < css.length; j++) {
      if (css[j] === '{') depth++
      else if (css[j] === '}' && --depth === 0) break
    }
    i = j
  }
  return out
}

/** 某选择器（精确匹配选择器列表中的一项）合并后的声明表 */
function declarations(css: string, selector: string): Record<string, string> {
  const merged: Record<string, string> = {}
  const body = stripAtBlocks(stripComments(css))
  const RULE = /([^{}]+)\{([^{}]*)\}/g
  let match: RegExpExecArray | null
  while ((match = RULE.exec(body))) {
    if (!match[1].split(',').some(sel => sel.trim() === selector)) continue
    for (const decl of match[2].split(';')) {
      const colon = decl.indexOf(':')
      if (colon < 0) continue
      merged[decl.slice(0, colon).trim()] = decl
        .slice(colon + 1)
        .trim()
        .replace(/\s+/g, ' ')
    }
  }
  return merged
}

const PLANNER_CSS = 'src/styles/planner.css'

describe('planner 弹窗高度链（滚动可行性的声明级契约）', () => {
  it('上游 .pm-content 提供裁切边界：flex column + max-height + overflow:hidden', async () => {
    const css = await readStyle(PLANNER_CSS)
    const d = declarations(css, '.pm-content')

    expect(d['display']).toBe('flex')
    expect(d['flex-direction']).toBe('column')
    // max-height 是「弹窗不超过视口」的上界；overflow:hidden 决定超出部分被裁
    expect(d['max-height']).toBe('75vh')
    expect(d.overflow).toBe('hidden')
  })

  it('下游 .planner-body 是可滚动容器：flex:1 + min-height:0 + overflow-y:auto', async () => {
    const css = await readStyle(PLANNER_CSS)
    const d = declarations(css, '.planner-body')

    expect(d.flex).toBe('1')
    // 关键回归点：缺 min-height:0 时 flex:1 收缩不了，长内容被裁且无滚动条
    expect(d['min-height']).toBe('0')
    expect(d['overflow-y']).toBe('auto')
  })

  it('中游 .pm-header 不让位：flex-shrink:0（极矮视口下标题不被压缩）', async () => {
    const css = await readStyle(PLANNER_CSS)
    const d = declarations(css, '.pm-header')

    expect(d['flex-shrink']).toBe('0')
  })

  it('两个消费方共用 planner.css 的 .planner-body，无人另起弹窗样式', async () => {
    const taskDetail = await readStyle('src/main-window/chat/TaskDetailModal.tsx')
    const planner = await readStyle('src/main-window/components/PlannerModal.tsx')

    // 各自 import 同一份样式表（同一 chunk，不重复打包）
    expect(taskDetail).toContain("import '../../styles/planner.css'")
    expect(planner).toContain("import '../../styles/planner.css'")
    // 滚动容器用的都是共享类，而非自建的 body 容器
    expect(taskDetail).toContain('className="planner-body"')
    expect(planner).toContain('className="planner-body"')
    expect(taskDetail).not.toMatch(/className="[^"]*\bt[dm]-body\b/)
  })
})
