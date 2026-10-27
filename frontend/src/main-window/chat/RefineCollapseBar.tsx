import { useCallback } from 'react'
import { IconSparkles, IconChevronDown } from '../../ui/Icons'
import { useLanguage } from '../../locales'
import '../../styles/refine-collapse-bar.css'

export interface RefineCollapseBarProps {
  /** 折叠段内的消息条数（refine 边界之前的历史消息） */
  count: number
  /** 当前是否展开 */
  expanded: boolean
  /** 点击切换展开/折叠 */
  onToggle: () => void
}

/**
 * Refine 折叠条 —— refine 前历史消息的紧凑占位。
 *
 * 设计约束（大王 10-21）：小巧精简、提示/按钮性质，**非通栏**。
 * 宽度自适应内容、居中显示，不铺满整行；点击展开/折叠其前的历史消息，
 * 解决多轮 refine 后前端历史气泡累积导致的卡顿。
 */
export function RefineCollapseBar({ count, expanded, onToggle }: RefineCollapseBarProps) {
  const { t } = useLanguage()
  const handleClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation()
      onToggle()
    },
    [onToggle],
  )

  return (
    <div className="refine-collapse-bar-row">
      <button
        type="button"
        className="refine-collapse-bar"
        onClick={handleClick}
        aria-expanded={expanded}
        title={expanded ? t('refineCollapse.collapse') : t('refineCollapse.expand')}
      >
        <IconSparkles size={12} />
        <span className="refine-collapse-bar-label">
          {t('refineCollapse.archived').replace('{n}', String(count))}
        </span>
        <IconChevronDown
          size={12}
          className={`refine-collapse-bar-chevron ${expanded ? 'expanded' : ''}`}
        />
      </button>
    </div>
  )
}
