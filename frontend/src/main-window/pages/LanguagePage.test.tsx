/**
 * LanguagePage（设置中心分区 `language`）契约测试。
 *
 * 2026-09-28 语言区从外观浮窗整体迁入设置中心，这条链路一行未改 —— 本测试把原
 * AppearancePanel 的语言覆盖（「切换即写后端」+「后端优先、localStorage 兜底」）
 * 平移到新落点，保证迁移不丢行为：
 * ① 挂载时优先读后端语言（getLanguage），失败回落 localStorage；
 * ② 点 segmented 即写 localStorage `nuphus_language` 并通知后端（zh-CN/en-US），
 *    aria-pressed 随动。
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LanguagePage } from './LanguagePage'
import { LangProvider } from '../../locales'

let backendLang: Promise<string> = Promise.resolve('zh-CN')
const setLanguageMock = vi.fn(async (_lang: string) => undefined)

vi.mock('../lib/api', () => ({
  getLanguage: () => backendLang,
  setLanguage: (lang: string) => setLanguageMock(lang),
}))

function renderPage() {
  return render(
    <LangProvider>
      <LanguagePage />
    </LangProvider>,
  )
}

const segItems = () => Array.from(document.querySelectorAll('.segmented-item'))

describe('LanguagePage 界面语言', () => {
  beforeEach(() => {
    localStorage.clear()
    backendLang = Promise.resolve('zh-CN')
    setLanguageMock.mockClear()
  })
  afterEach(() => {
    vi.clearAllMocks()
  })

  it('分区标题 + zh/en 两项，默认选中跟随后端设置', async () => {
    renderPage()

    // Section 标题与表单 label 同源（app.language）
    expect(screen.getAllByText('语言').length).toBeGreaterThan(0)
    await waitFor(() => expect(segItems().map(b => b.textContent)).toEqual(['中文', 'English']))
    expect(segItems()[0].getAttribute('aria-pressed')).toBe('true')
    expect(segItems()[1].getAttribute('aria-pressed')).toBe('false')
  })

  it('切到 English：写 localStorage + 通知后端 en-US，aria-pressed 随动', async () => {
    renderPage()
    await waitFor(() => expect(segItems()[0].getAttribute('aria-pressed')).toBe('true'))

    fireEvent.click(segItems()[1])

    expect(segItems()[1].getAttribute('aria-pressed')).toBe('true')
    expect(segItems()[0].getAttribute('aria-pressed')).toBe('false')
    expect(localStorage.getItem('nuphus_language')).toBe('en')
    expect(setLanguageMock).toHaveBeenCalledWith('en-US')
  })

  it('后端读取失败：回落 localStorage 里已存的语言', async () => {
    localStorage.setItem('nuphus_language', 'en')
    backendLang = Promise.reject(new Error('backend down'))
    renderPage()

    await waitFor(() => expect(segItems()[1].getAttribute('aria-pressed')).toBe('true'))
  })
})
