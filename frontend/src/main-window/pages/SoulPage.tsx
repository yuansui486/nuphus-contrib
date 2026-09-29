/**
 * SoulPage.tsx — 灵魂（身份：头像 + 称呼，单模块）
 *
 * 大王 2026-09-28 定稿：头像与关系本来就是一件事（「谁在说话、长什么样、叫什么」），
 * 不拆两个模块照搬照抄 —— 一个模块内：
 *   ① 「在消息中显示头像」总开关；
 *   ② 用户 / Nuphus 两条身份行：**点头像即上传**（pickAndImportImage 入库只存路径），
 *      「称呼」即关系字段 —— 保存时 saveRelation 落 localStorage + setRelation
 *      同步后端 relation.json（手机端 /identity 经 relation_cache 下发显示名）。
 *
 * 头像落库语义自 2026-09-28 并入起逐行未变：三个 localStorage 键、
 * toAssetUrl 渲染 asset://、空态退回字母头像（用户侧 U / 智能体侧 A）。
 *
 * 主题相关的外观调整不在这里：见 layout/AppearancePanel.tsx。
 */
import { useEffect, useState } from 'react'
import { loadRelation, saveRelation, type RelationConfig } from '../lib/relation'
import { setRelation } from '../lib/api'
import { Button } from '../../ui/Button'
import { LetterAvatar } from '../../ui/LetterAvatar'
import { toAssetUrl } from '../../ui/assetUrl'
import { Section, FormRow } from '../../ui/PageLayout'
import { pickAndImportImage } from '../lib/localImage'
import { useLanguage } from '../../locales'
/* 头像位样式（.avatar-preview / .avatar-preview--upload / .soul-name-input）
   定义在 themes.css（与主题面板同一份样式文件的头像小节） */
import '../../styles/themes.css'

const LS_SHOW_AVATAR = 'nuphus_show_avatar'
const LS_USER_AVATAR = 'nuphus_user_avatar'
const LS_NUPHUS_AVATAR = 'nuphus_nuphus_avatar'

/** 两条身份行同构（头像位 + 称呼字段成对），仅数据不同 —— 配置化避免照搬。
   侧别不配可见文字（大王定稿）：由头像本身（U / A 字母或上传图）与固定次序区分 */
const IDENTITY_SIDES = [
  { side: 'user', nameKey: 'userLabel', placeholder: 'USER' },
  { side: 'nuphus', nameKey: 'assistantName', placeholder: 'Nuphus' },
] as const satisfies readonly {
  side: 'user' | 'nuphus'
  nameKey: keyof RelationConfig
  placeholder: string
}[]

export function SoulPage({ onClose }: { onClose: () => void }) {
  const { t } = useLanguage()
  const [rel, setRel] = useState(() => loadRelation())
  const [saved, setSaved] = useState(false)
  const [showAvatar, setShowAvatar] = useState(false)
  const [userAvatar, setUserAvatar] = useState('')
  const [nuphusAvatar, setNuphusAvatar] = useState('')

  useEffect(() => {
    setShowAvatar(localStorage.getItem(LS_SHOW_AVATAR) === 'true')
    setUserAvatar(localStorage.getItem(LS_USER_AVATAR) || '')
    setNuphusAvatar(localStorage.getItem(LS_NUPHUS_AVATAR) || '')
  }, [])

  const handleSave = () => {
    saveRelation(rel)
    // 同步持久化到后端 relation.json（手机端 /identity 经 relation_cache 下发显示名）
    void setRelation(rel).catch(() => {})
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  const update = (key: keyof RelationConfig, val: string) => {
    setRel(prev => ({ ...prev, [key]: val }))
    setSaved(false)
  }

  const handleAvatarSelect = async (type: 'user' | 'nuphus') => {
    // 与皮肤背景同一套本地架构：入库到磁盘、只存路径（见 lib/localImage.ts）
    try {
      const path = await pickAndImportImage()
      if (!path) return
      const key = type === 'user' ? LS_USER_AVATAR : LS_NUPHUS_AVATAR
      const setter = type === 'user' ? setUserAvatar : setNuphusAvatar
      setter(path)
      localStorage.setItem(key, path)
    } catch (e) {
      console.error('头像入库失败:', e)
      alert(t('themes.avatarImportFailed'))
    }
  }

  const clearAvatar = (type: 'user' | 'nuphus') => {
    const key = type === 'user' ? LS_USER_AVATAR : LS_NUPHUS_AVATAR
    ;(type === 'user' ? setUserAvatar : setNuphusAvatar)('')
    localStorage.removeItem(key)
  }

  const handleToggleAvatar = () => {
    const next = !showAvatar
    setShowAvatar(next)
    localStorage.setItem(LS_SHOW_AVATAR, String(next))
  }

  // 默认头像 = 字母头像：用户侧 U、智能体侧 A（LetterAvatar 尺寸即容器尺寸，铺满）。
  // 自定义头像存的是本地方径，经 toAssetUrl 转 asset:// 渲染（存量 dataURL 亦兼容）。
  // 容器 = 可点的 .avatar-preview 按钮本身（不再套一层同名 span）。
  const renderAvatar = (src: string, fallback: 'user' | 'nuphus') =>
    src ? (
      <img src={toAssetUrl(src) ?? undefined} alt="" />
    ) : (
      <LetterAvatar letter={fallback === 'user' ? 'U' : 'A'} size={34} />
    )

  return (
    <Section title={t('app.soul')}>
      {/* ① 总开关：消息里是否显示头像 */}
      <FormRow
        label={t('themes.showAvatar')}
        control={
          <button
            type="button"
            role="switch"
            aria-checked={showAvatar}
            className="switch"
            onClick={handleToggleAvatar}
          />
        }
      />

      {/* ② 两条身份行：头像（点击即上传）+ 称呼输入，无解释性文字 */}
      {IDENTITY_SIDES.map(({ side, nameKey, placeholder }) => {
        const avatar = side === 'user' ? userAvatar : nuphusAvatar
        return (
          <FormRow
            key={side}
            label={
              <button
                type="button"
                className="avatar-preview avatar-preview--upload"
                title={t('themes.upload')}
                aria-label={t('themes.upload')}
                onClick={() => handleAvatarSelect(side)}
              >
                {renderAvatar(avatar, side)}
              </button>
            }
            control={
              <>
                <input
                  className="input soul-name-input"
                  value={rel[nameKey]}
                  onChange={e => update(nameKey, e.target.value)}
                  placeholder={placeholder}
                  aria-label={t('soul.nameField')}
                />
                {avatar && (
                  <Button variant="danger" size="sm" onClick={() => clearAvatar(side)}>
                    {t('themes.clearBg')}
                  </Button>
                )}
              </>
            }
          />
        )
      })}

      <div className="form-footer">
        {saved && <span className="badge badge-success">{t('common.saved')}</span>}
        <Button variant="primary" onClick={handleSave}>
          {t('common.save')}
        </Button>
      </div>
    </Section>
  )
}
