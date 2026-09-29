import { ReactNode, ReactElement, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { IconX, IconGrip } from '../../ui/Icons'
import { IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'
import { usePanelDrag } from '../../hooks/usePanelDrag'
import '../../styles/panel-drag.css'

interface CompactModalProps {
  open: boolean
  onClose: () => void
  title: string
  icon?: ReactElement
  size?: 'sm' | 'md' | 'lg' | 'xl' | 'auto'
  /** 弹层所在宿主；settings 需覆盖设置中心自身的高层遮罩 */
  layer?: 'default' | 'settings'
  /** 追加到 modal 卡片的类（如 compact-modal--fit 高度自适应） */
  className?: string
  /**
   * 传入 localStorage key 即启用「应用内拖拽」：标题左侧出现共享把手（.panel-grip），
   * 拖过一次后卡片脱离 flex 居中、改按 inline left/top 的 position:fixed 定位
   * （.compact-overlay 是 inset:0 的全屏遮罩，其 padding box 与视口重合，
   * fixed 的包含块即视口，与 DesktopToolbar 同一套机制）。缺省 → 与现在完全一致。
   */
  dragKey?: string
  /** 固定底部操作区 — 渲染在滚动区之外，长内容时主操作始终可见 */
  footer?: ReactNode
  children: ReactNode
}

/** 出场动画时长（与 components.css 中 compactModalOut 一致） */
const EXIT_MS = 150

export function CompactModal({
  open,
  onClose,
  title,
  icon,
  size = 'auto',
  layer = 'default',
  className,
  dragKey,
  footer,
  children,
}: CompactModalProps) {
  const { t } = useLanguage()
  const [closing, setClosing] = useState(false)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // 应用内拖拽（可选）：dragKey 缺省 → enabled=false，hook 空转、无监听、无样式，
  // 其余全部 CompactModal 调用方行为零变化
  const drag = usePanelDrag(dragKey ?? '', {
    enabled: Boolean(dragKey) && open,
    clampOnResize: true,
  })

  // 父组件把 open 置 false 时重置 closing 状态
  useEffect(() => {
    if (!open) setClosing(false)
  }, [open])

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current)
    },
    [],
  )

  if (!open) return null

  const requestClose = () => {
    if (closing) return
    setClosing(true)
    timerRef.current = setTimeout(onClose, EXIT_MS)
  }

  const sizeClass = size === 'auto' ? 'compact-modal--auto' : `compact-modal--${size}`
  const overlayClassName = [
    'compact-overlay',
    layer === 'settings' ? 'compact-overlay--above-settings' : '',
    closing ? 'compact-overlay--closing' : '',
  ]
    .filter(Boolean)
    .join(' ')

  // Portal 到 body：就地渲染时，任何带 transform/filter 的祖辈会成为 fixed 后代的
  // 包含块（CSS 规范），导致弹窗按局部盒子定位而非视口——如 .chat-input-area 的
  // translate(-50%) 曾把终止确认弹窗错位到输入框区域内。与项目弹层 portal 惯例一致。
  return createPortal(
    <div className={overlayClassName} onClick={requestClose}>
      <div
        ref={drag.panelRef}
        className={`compact-modal ${sizeClass}${className ? ` ${className}` : ''}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={e => e.stopPropagation()}
        // 拖过后才脱离 flex 居中、改按 left/top 固定；未拖拽过不写 inline（观感零回归）。
        // 进场/退场动画跑在 transform 上，与 left/top 定位互不抵消
        style={
          dragKey && drag.pos ? { position: 'fixed', left: drag.pos.x, top: drag.pos.y } : undefined
        }
      >
        <div className="compact-header">
          {dragKey && (
            <div className="panel-grip" onMouseDown={drag.handleMouseDown} title="拖拽移动">
              <IconGrip size={14} />
            </div>
          )}
          {icon && <div className="compact-header-icon">{icon}</div>}
          <span className="compact-header-title">{title}</span>
          <IconButton
            variant="compact-header-close"
            label={t('common.close')}
            onClick={requestClose}
          >
            <IconX size={14} />
          </IconButton>
        </div>
        <div className="compact-divider" />
        <div className="compact-body">{children}</div>
        {footer && (
          <>
            <div className="compact-divider" />
            <div className="compact-footer">{footer}</div>
          </>
        )}
      </div>
    </div>,
    document.body,
  )
}
