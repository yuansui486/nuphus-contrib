/**
 * 自定义主题 / 皮肤背景的**启动恢复**契约。
 *
 * 背景（2026-09-28，用户报「自定义主题刷新后回默认系统主题、背景图不显示」，
 * 且此前已复现过一次）：App 层的启动恢复 effect 只读 LS_SKIN（系统预设态背景），
 * 而「激活自定义主题时按该主题 skin 快照写回全局背景」只发生在
 * `useTheme.activateCustom`（用户点击那一瞬）—— **没有任何启动路径重放它**。
 * 于是激活自定义主题的用户一刷新，背景就被 LS_SKIN 覆盖（LS_SKIN 为空 → 整个消失）。
 *
 * 本契约钉两件事，防第三次复现：
 *   ① 恢复源选择：`skinRestoreSource` 按激活态二选一（纯函数断言）
 *   ② App 层确实消费它：源码级断言，防止有人把 effect 改回只读 LS_SKIN
 */
import { describe, expect, it } from 'vitest'
import { skinRestoreSource } from '../hooks/useTheme'
import type { CustomTheme } from '../hooks/customTheme'

type FsLike = { readFileSync: (path: string, encoding: string) => string }

async function readSource(relPath: string): Promise<string> {
  const spec = 'node:fs'
  const fs = (await import(/* @vite-ignore */ spec)) as unknown as FsLike
  const cwd = (globalThis as unknown as { process: { cwd(): string } }).process.cwd()
  return fs.readFileSync(`${cwd}/${relPath}`, 'utf8')
}

const themed = (skin?: string): CustomTheme => ({
  id: 'ct-test',
  name: '测试主题',
  base: 'dark',
  overrides: { '--accent': '#3b82f6' },
  skin,
})

describe('皮肤背景启动恢复源（skinRestoreSource）', () => {
  it('激活自定义主题且带 skin 快照 → 用快照（不用 LS_SKIN 覆盖）', () => {
    expect(skinRestoreSource(themed('C:/img/a.png'), 'C:/img/preset.png')).toBe('C:/img/a.png')
  })

  it('激活自定义主题但无皮肤 → 清空语义（空串），不继承系统预设图', () => {
    expect(skinRestoreSource(themed(undefined), 'C:/img/preset.png')).toBe('')
  })

  it('未激活任何自定义主题 → 系统预设态 LS_SKIN', () => {
    expect(skinRestoreSource(null, 'C:/img/preset.png')).toBe('C:/img/preset.png')
  })
})

describe('App 层启动恢复的消费方式（源码级契约）', () => {
  it('恢复 effect 走 skinRestoreSource(激活态, LS_SKIN)，而不是只读 LS_SKIN', async () => {
    const app = await readSource('src/main-window/App.tsx')
    const theme = await readSource('src/hooks/useTheme.tsx')

    // 恢复调用点：两个参数都在（激活主题 + 预设背景）
    expect(app).toContain('skinRestoreSource(customTheme, readSkinBg())')
    // 不得退回旧形态：只读 LS_SKIN 就 apply（那条路径就是本 bug 的根因）
    expect(app).not.toContain('applySkinBg(readSkinBg())\n')
    // 激活态来自 useTheme（App 在 ThemeProvider 内，见 main.tsx）
    expect(app).toContain('useTheme()')
    // 纯函数本体在 useTheme 内导出，供测试与 App 共用同一语义
    expect(theme).toContain('export function skinRestoreSource(')
  })
})
