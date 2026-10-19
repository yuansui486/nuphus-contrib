/**
 * 会话行「hover 交接」契约 —— 钉住一条不变式：**行尾弱信息必须整组淡出**。
 *
 * 为什么需要这组用例（2026-10-15 实测报的 bug）：
 * `.sr-actions`（置顶 / 重命名 / 归档）是 `position: absolute; right: 6px` 的浮层，
 * 直接压在行右端。行尾弱信息 —— 「当前」徽标 `.sr-current-badge`、置顶标签
 * `.sr-group-tag`、相对时间 `.sr-time` —— 在行 hover 时若不跟着淡出，就会被
 * 浮出的按钮图标盖住：现象就是「置顶」两个字与垃圾桶图标叠在一起。
 *
 * 设计上原本只有 `.sr-time` 做了交接（它是行尾唯一元素时就写好了），后来
 * 加进来的徽标与置顶标签漏了这一步。这组用例把「谁在行尾」这件事变成断言：
 * 今后再往行尾加弱信息，漏了交接会被直接挡住。
 *
 * 读法沿用 opacity-system.test.ts 的既有结论：本项目 Vite 配置下 `?raw` 对 .css
 * 返回空串（断言会静默落空），tsc 又未开 @types/node，故用运行时动态引入 node:fs。
 * 只在 CSS 文本层面断言（jsdom 不做布局/级联计算）。
 */

import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

/** 去掉注释，避免注释里提到的选择器把断言骗过去 */
const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '')

/** 剥掉 @media / @supports 等 at-rule 块（按大括号平衡），只留顶层规则 */
function stripAtRuleBlocks(css: string): string {
  let out = ''
  let i = 0
  for (;;) {
    const at = css.indexOf('@', i)
    if (at === -1) return out + css.slice(i)
    out += css.slice(i, at)
    let j = css.indexOf('{', at)
    if (j === -1) return out
    let depth = 0
    for (; j < css.length; j += 1) {
      if (css[j] === '{') depth += 1
      else if (css[j] === '}') {
        depth -= 1
        if (depth === 0) break
      }
    }
    i = j + 1
  }
}

const norm = (s: string) => s.trim().replace(/\s+/g, ' ')

/** 收集「选择器列表中包含 selector 的规则」的声明块（多选择器规则必须拆开比对） */
function declarationsFor(css: string, selector: string): string[] {
  const out: string[] = []
  const re = /([^{}]+)\{([^{}]*)\}/g
  let m: RegExpExecArray | null
  while ((m = re.exec(css)) !== null) {
    if (m[1].split(',').map(norm).includes(norm(selector))) out.push(m[2])
  }
  return out
}

/** 所有含某声明的规则里出现过的选择器（摊平） */
function selectorsWithDeclaration(css: string, declaration: RegExp): string[] {
  const out: string[] = []
  const re = /([^{}]+)\{([^{}]*)\}/g
  let m: RegExpExecArray | null
  while ((m = re.exec(css)) !== null) {
    if (declaration.test(m[2])) out.push(...m[1].split(',').map(norm))
  }
  return out
}

const rail = stripAtRuleBlocks(stripComments(read('./session-rail.css')))

/** 行尾弱信息：留在行右端、会被 .sr-actions 覆盖的元素 */
const TRAILING_WEAK = ['.sr-current-badge', '.sr-group-tag', '.sr-time'] as const

describe('会话行 hover 交接（行尾弱信息整组淡出）', () => {
  it('.sr-actions 是绝对定位浮层 —— 这正是「必须整组交接」的原因', () => {
    const body = declarationsFor(rail, '.sr-actions')[0] ?? ''
    expect(body).toContain('position: absolute')
    expect(body).toMatch(/opacity:\s*0/)
  })

  it('hover 时浮出（与淡出成对，缺一边交接就不成立）', () => {
    const bodies = declarationsFor(rail, '.sr-item:hover .sr-actions')
    expect(bodies.length).toBeGreaterThan(0)
    expect(bodies.join('\n')).toMatch(/opacity:\s*1/)
  })

  it.each(TRAILING_WEAK)('%s 在行 hover 时淡出', selector => {
    const bodies = declarationsFor(rail, `.sr-item:hover ${selector}`)
    expect(bodies.length, `${selector} 漏了 hover 交接，会被 .sr-actions 压住`).toBeGreaterThan(0)
    expect(bodies.join('\n')).toMatch(/opacity:\s*0/)
  })

  it.each(TRAILING_WEAK)('%s 在行键盘聚焦（focus-within）时同样淡出', selector => {
    const bodies = declarationsFor(rail, `.sr-item:focus-within ${selector}`)
    expect(
      bodies.length,
      `${selector} 漏了 focus-within 交接（键盘用户同样会被压住）`,
    ).toBeGreaterThan(0)
    expect(bodies.join('\n')).toMatch(/opacity:\s*0/)
  })

  it.each(TRAILING_WEAK)('%s 带透明度过渡（否则是瞬移，不是交接）', selector => {
    const selectors = selectorsWithDeclaration(rail, /transition:\s*opacity/)
    // `.sr-time` 是裸选择器（本就只服务行内），后加的两个挂 `.sr-item` 作用域下
    const ok = selectors.includes(selector) || selectors.includes(`.sr-item ${selector}`)
    expect(ok, `${selector} 缺 transition: opacity —— 需要对得上 .sr-time 的交叉淡化`).toBe(true)
  })

  it('组头的同名标签不被误伤（.sr-group-tag 是通用类）', () => {
    // 组头用的是 .sr-group-head 容器，交接规则必须限定在 .sr-item 作用域内
    const bodies = declarationsFor(rail, '.sr-group-head:hover .sr-group-tag')
    expect(bodies.join('\n')).not.toMatch(/opacity:\s*0/)
  })
})
