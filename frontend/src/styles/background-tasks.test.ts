/**
 * 后台任务面板的样式契约 —— 把新浮层钉进既有的不透明度体系。
 *
 * 为什么单独开一个文件而不是往 opacity-system.test.ts 里加：那份是全局体系
 * 的回归网（覆盖四个既有浮层 + token 体系），本次交付不该改动它（他人在途）。
 * 这里只锁**本次新增的浮层**遵守同一套约定：
 *   ① 面板底走 var(--panel-bg) —— 与 .session-rail-drawer / .wfst-panel 同键，
 *      受「主题设置 → 界面不透明度 → 控制面板」滑块管辖
 *   ② 不得退回 --glass-bg / --modal-bg 硬底，也不得用同族键兑透明绕过
 *   ③ 入口胶囊沿用 .chat-header-settings-btn 的磨砂配方（--panel-bg 派生 + blur）
 *   ④ 不写死 rgba 颜色、不新造颜色/阴影键（全部走既有 token）
 *
 * 读取方式沿用既有结论：`?raw` 在本项目 Vite 配置下对 .css 返回空串，
 * `node:fs` 又缺 @types/node —— 故模块名由变量拼出 + 运行时动态引入 + 类型收敛。
 */

import { describe, expect, it } from 'vitest'

const { readFileSync } = (await import('node:' + 'fs')) as {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  readFileSync: (p: any, enc: string) => string
}

/** 与 opacity-system.test.ts 同款读取：URL 对象入参，字面量路径由调用方拼出 */
const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')

const css = read('./background-tasks.css')

/** 去掉注释，避免注释里提到的选择器/属性名把断言骗过去 */
const stripComments = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, '')

const panelCss = stripComments(css)

/** 取一条顶层规则的声明块（首个匹配） */
const ruleBody = (selector: string): string => {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const re = new RegExp(`${escaped}\\s*\\{([^}]*)\\}`, 's')
  return re.exec(panelCss)?.[1] ?? ''
}

describe('后台任务面板样式契约（不透明度体系 + 无新 token）', () => {
  it('样式源确实读进来了（防「读空文件导致断言静默通过」）', () => {
    expect(panelCss.length).toBeGreaterThan(500)
    expect(panelCss).toContain('.bgt-panel')
    expect(panelCss).toContain('.bgt-entry')
  })

  it('.bgt-panel 底走 var(--panel-bg)', () => {
    expect(ruleBody('.bgt-panel')).toMatch(/background:\s*var\(--panel-bg\)/)
  })

  it('.bgt-panel 不得退回 --glass-bg / --modal-bg，也不得兑透明绕过', () => {
    const body = ruleBody('.bgt-panel')
    expect(body).not.toMatch(/background:\s*var\(--glass-bg\)/)
    expect(body).not.toMatch(/background:\s*var\(--modal-bg\)/)
    expect(body).not.toMatch(/color-mix\([^)]*--(glass-bg|modal-bg)/)
  })

  it('.bgt-entry 与聊天头部按钮同款磨砂配方（--panel-bg 派生 + blur）', () => {
    const body = ruleBody('.bgt-entry')
    expect(body).toMatch(/background:\s*color-mix\(in srgb, var\(--panel-bg\) \d+%, transparent\)/)
    expect(body).toMatch(/backdrop-filter:\s*blur\(/)
  })

  it('不写死 rgba 颜色、不 !important 绕过 token（边框/阴影/圆角全部走既有键）', () => {
    expect(panelCss).not.toMatch(/:\s*rgba\(/)
    expect(panelCss).not.toMatch(/!\s*important/)
    const panel = ruleBody('.bgt-panel')
    expect(panel).toMatch(/border:\s*var\(--border-divider\)/)
    expect(panel).toMatch(/box-shadow:\s*var\(--shadow-elevated\)/)
    expect(panel).toMatch(/border-radius:\s*\d+px/)
  })

  it('锚点压过工作流面板（z 52）；胶囊与面板同属锚点堆叠上下文', () => {
    expect(ruleBody('.bgt-anchor')).toMatch(/z-index:\s*52/)
    expect(ruleBody('.bgt-entry')).not.toMatch(/z-index/)
    expect(ruleBody('.bgt-panel')).toMatch(/z-index:\s*1\b/)
  })

  it('面板向左展开、右缘对齐胶囊左缘（避开桌面工具条命中区，用户钦定契约）', () => {
    // 锚点是唯一 fixed 元素（含工具条重叠事故背景的定位职责移交）
    const anchor = ruleBody('.bgt-anchor')
    expect(anchor).toMatch(/position:\s*fixed/)
    expect(anchor).toMatch(/width:\s*fit-content/)
    // 面板 absolute 于锚点内：右缘 = 胶囊左缘 - 10px 间距
    const panel = ruleBody('.bgt-panel')
    expect(panel).toMatch(/position:\s*absolute/)
    expect(panel).toMatch(/right:\s*calc\(100%\s*\+\s*10px\)/)
  })

  it('锚点 right=20px：入口胶囊不位移（大王钦定）；位移真源唯一', () => {
    // 2026-10-11 大王钦定：入口胶囊保持 right:20px 不动。面板向左展开、
    // 右缘 = 胶囊左缘 - 10px，整块天然避开 .desktop-toolbar-dock 命中列
    //（fixed right:20px / 30px 宽）的关闭按钮区域。锚点 right 是这块定位的
    // **唯一真源**（面板 absolute 于锚点内、随锚点移动），故禁止面板自己写 fixed。
    const anchor = ruleBody('.bgt-anchor')
    expect(anchor).toMatch(/right:\s*20px/)
    // 位移真源唯一：面板不得脱离锚点改写自己的 fixed/right
    const panel = ruleBody('.bgt-panel')
    expect(panel).not.toMatch(/position:\s*fixed/)
  })

  it('头部上下对齐：图标盒与标题行高同为 16px，header 上下留白对称', () => {
    // 2026-10-12 修复：图标包在裸 inline span 里按**文字基线**排布（不是垂直居中），
    // 与标题上下错位；且 header padding 顶部为 0，标题贴死面板上沿。
    // 三处钉死：图标必须 flex 居中 + 16px 方框、标题行高 16px、padding 上下对称。
    const header = ruleBody('.bgt-header')
    expect(header).toMatch(/padding:\s*10px\s+12px/)
    expect(header).not.toMatch(/padding:\s*0\s/)

    const icon = ruleBody('.bgt-header-icon')
    expect(icon).toMatch(/display:\s*flex/)
    expect(icon).toMatch(/align-items:\s*center/)
    expect(icon).toMatch(/height:\s*16px/)

    // 标题行高与图标方框同高 → align-items: center 对齐后两者中线重合
    const title = ruleBody('.bgt-title')
    expect(title).toMatch(/line-height:\s*16px/)
  })
})
