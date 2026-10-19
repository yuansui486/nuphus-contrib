import React from 'react'
import ReactDOM from 'react-dom/client'
import { CaptureOverlay } from './CaptureOverlay'

/**
 * 主题同步（最小子集，与主窗 ThemeProvider 同源）：
 * - 基底：localStorage `nuphus_theme`（dark/light/tech），缺省 dark；
 * - 自定义主题：`nuphus_custom_themes` + `nuphus_custom_active` 激活项的 overrides
 *   逐键写进 <html> 内联样式（与 ThemeProvider.applyOverrides 同语义：内联优先级
 *   压过 [data-theme] 规则）。覆盖窗此前从不接主题——确认条永远显示暗色面板，
 *   亮色/自定义主题用户看到的弹窗与主题脱节。
 * - 键名与 useTheme.tsx 的 LS_* 逐字一致（跨窗口 localStorage 共享）。
 */
;(() => {
  const html = document.documentElement
  type ThemeId = 'dark' | 'light' | 'fresh'
  const readBase = (): ThemeId => {
    try {
      const saved = localStorage.getItem('nuphus_theme') as ThemeId | null
      if (saved === 'dark' || saved === 'light' || saved === 'fresh') return saved
    } catch {}
    return 'dark'
  }
  let theme: ThemeId = readBase()
  try {
    const raw = localStorage.getItem('nuphus_custom_themes')
    const activeId = localStorage.getItem('nuphus_custom_active')
    if (raw && activeId) {
      const parsed = JSON.parse(raw) as {
        id?: string
        base?: ThemeId
        overrides?: Record<string, string>
      }[]
      const active = Array.isArray(parsed) ? parsed.find(t => t.id === activeId) : undefined
      if (active) {
        if (active.base === 'dark' || active.base === 'light' || active.base === 'fresh') {
          theme = active.base
        }
        for (const [key, value] of Object.entries(active.overrides ?? {})) {
          html.style.setProperty(key, value)
        }
      }
    }
  } catch {}
  html.setAttribute('data-theme', theme)
})()

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <CaptureOverlay />
  </React.StrictMode>,
)
