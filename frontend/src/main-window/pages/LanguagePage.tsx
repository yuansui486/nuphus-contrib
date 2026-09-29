/**
 * LanguagePage.tsx — 界面语言（设置中心分区 `language`）
 *
 * 2026-09-28 从外观浮窗（layout/AppearancePanel.tsx）整体迁出：语言是全局偏好，
 * 与「改完立刻看见主界面」的主题调整不是同一档价值——混在外观浮窗里只会把浮窗
 * 拉得更长。落库链路**逐行未改**（原样迁移）：
 *   - 选中即写 localStorage `nuphus_language`（与 LangProvider、移动端同键）；
 *   - 同步通知后端 `apiSetLanguage`（zh-CN / en-US）；
 *   - 挂载时优先读后端设置，兜底 localStorage（后端返回 zh-CN/en-US，归一化到 zh/en）。
 */
import { useEffect, useState } from 'react'
import { FormRow, Section } from '../../ui/PageLayout'
import { setLanguage as apiSetLanguage, getLanguage } from '../lib/api'
import { useLanguage } from '../../locales'

const LANG = [
  { id: 'zh', label: 'lang.zh' },
  { id: 'en', label: 'lang.en' },
]

const LS_LANG = 'nuphus_language'

export function LanguagePage() {
  const { t, setLang } = useLanguage()
  const [language, setLanguage] = useState('zh')

  useEffect(() => {
    // 优先读取后端语言设置，兜底 localStorage（后端返回 zh-CN/en-US，归一化到 zh/en）
    getLanguage()
      .then(backendLang => {
        const raw = backendLang || localStorage.getItem(LS_LANG) || 'zh'
        setLanguage(raw.startsWith('zh') ? 'zh' : 'en')
      })
      .catch(() => {
        const raw = localStorage.getItem(LS_LANG) || 'zh'
        setLanguage(raw.startsWith('zh') ? 'zh' : 'en')
      })
  }, [])

  const handleLang = (id: string) => {
    setLanguage(id)
    setLang(id)
    localStorage.setItem(LS_LANG, id)
    apiSetLanguage(id === 'zh' ? 'zh-CN' : 'en-US')
  }

  return (
    <Section title={t('app.language')}>
      <FormRow
        label={t('app.language')}
        control={
          <div className="segmented" role="group">
            {LANG.map(l => (
              <button
                key={l.id}
                type="button"
                aria-pressed={language === l.id}
                className={`segmented-item ${language === l.id ? 'active' : ''}`}
                onClick={() => handleLang(l.id)}
              >
                {t(l.label)}
              </button>
            ))}
          </div>
        }
      />
    </Section>
  )
}
