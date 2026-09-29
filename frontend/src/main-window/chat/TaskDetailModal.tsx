import React, { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import type { TaskRun, TaskRunState } from '../../core/types'
import { useLanguage } from '../../locales'
import { IconButton } from '../../ui/Button'
import { IconX } from '../../ui/Icons'
import MarkdownContent from './MarkdownContent'
import { fmtDuration } from './taskRunFormat'
import '../../styles/planner.css'

interface TaskDetailModalProps {
  open: boolean
  /** 待展示的台账条目。退场动画期间仍要看到内容，关严后由父层清空 */
  run: TaskRun | null
  onClose: () => void
}

/** 状态 → 文案 key。四态齐备：interrupted 有独立文案，绝不混进 failed/completed */
const STATE_TEXT_KEY: Record<TaskRunState, string> = {
  running: 'taskBubble.detail.stateRunning',
  completed: 'taskBubble.detail.stateCompleted',
  failed: 'taskBubble.detail.stateFailed',
  interrupted: 'taskBubble.detail.stateInterrupted',
}

/** 状态 → pm-header-status 的配色档（沿用 planner.css 的 is-* 修饰名约定；
 *  具体取色在 task-bubble.css 补齐：planner.css 只为计划态定义了 active/archived） */
const STATUS_TONE: Record<TaskRunState, string> = {
  running: 'is-running',
  completed: 'is-completed',
  failed: 'is-failed',
  interrupted: 'is-interrupted',
}

/** 退场时长：与 PlannerModal 的延时卸载同参（先播完动画再摘 DOM） */
const EXIT_MS = 250

/**
 * 任务详情弹窗 —— 台账单条目的只读投影（标题 / 状态 / 元信息 / markdown 摘要）。
 *
 * 样式与动效**全部复用 planner 弹窗**（pm-wrapper / pm-backdrop / pm-content /
 * pm-header / planner-body / pm-req-card / pm-req-label / pm-req-text…
 * 见 TaskBubble 样式附录里的类名清单），不另起一套弹窗视觉；
 * 开合动画范式照 PlannerModal：visible + animating 双态，退场延时卸载。
 *
 * portal 到 body 是**必须**的：调用方 TaskBubble 的 .task-track 带
 * backdrop-filter，它会劫持 position:fixed 的定位基准，把弹窗压回
 * 230px 的面板框里（同 PreviewOverlay 的既有结论）。
 */
export const TaskDetailModal: React.FC<TaskDetailModalProps> = ({ open, run, onClose }) => {
  const { t } = useLanguage()
  const [visible, setVisible] = useState(false)
  const [animating, setAnimating] = useState(false)
  /** 退场动画期间要看到的内容：父层关窗即清 run，这里留住最后一条，
   *  否则 open 一置 false 内容瞬间消失，250ms 退场只剩空壳在播 */
  const [rendered, setRendered] = useState<TaskRun | null>(null)

  useEffect(() => {
    if (open && run) {
      setVisible(true)
      setRendered(run)
      const raf = requestAnimationFrame(() => setAnimating(true))
      return () => cancelAnimationFrame(raf)
    }
    setAnimating(false)
    const timer = setTimeout(() => {
      setVisible(false)
      setRendered(null)
    }, EXIT_MS)
    return () => clearTimeout(timer)
  }, [open, run])

  // Esc 关闭（浮层通用范式：window keydown，关严即摘监听）
  useEffect(() => {
    if (!open) return
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [open, onClose])

  if (!visible || !rendered) return null

  // 摘要兜底：null / 空串 / 全是空白都走同一句（后端只在结算时写 summary，
  // 故 running 态必然落在这里；正文区绝不空着）
  const summary = rendered.summary?.trim() ?? ''
  const duration = fmtDuration(rendered.duration_ms)
  // 派发正文：后端 open 时写入，可能为空（旧台账残留）
  const taskText = rendered.task?.trim() ?? ''

  return createPortal(
    <div className="pm-wrapper">
      <div className={`pm-backdrop ${animating ? '' : 'is-hidden'}`} onClick={onClose} />

      <div className={`pm-content ${animating ? '' : 'is-closing'}`}>
        <div className="pm-header">
          <div className="ptc-header">
            <div className="ptc-header-left">
              <span className="planner-header-title">{rendered.title}</span>
              <span className={`pm-header-status ${STATUS_TONE[rendered.state]}`}>
                {t(STATE_TEXT_KEY[rendered.state])}
              </span>
            </div>
            <div className="ptc-header-right">
              <IconButton variant="modal-close" label={t('common.close')} onClick={onClose}>
                <IconX size={14} />
              </IconButton>
            </div>
          </div>

          <div className="pm-header-meta">
            {rendered.attempt > 1 && (
              <span>{t('taskBubble.detail.attempt', String(rendered.attempt))}</span>
            )}
            {duration && <span>{duration}</span>}
            <span>{rendered.goal_type}</span>
            {rendered.origin?.task_no != null && (
              <span>{t('taskBubble.detail.plan', String(rendered.origin.task_no))}</span>
            )}
          </div>
        </div>

        <div className="planner-body">
          {/* ① 派发正文：点开就能看到这笔派的是什么（markdown 全文） */}
          <div className="pm-req-card">
            <div className="pm-req-label">{t('taskBubble.detail.task')}</div>
            <div className="pm-req-text">
              {taskText ? (
                <div className="task-detail-markdown task-detail-task">
                  <MarkdownContent content={rendered.task} />
                </div>
              ) : (
                t('taskBubble.detail.noTask')
              )}
            </div>
          </div>

          {/* ② 交付全文：ExecAgent 交给 Leader 的完整结果（markdown，不截断） */}
          <div className="pm-req-card">
            <div className="pm-req-label">{t('taskBubble.detail.summary')}</div>
            <div className="pm-req-text">
              {summary ? (
                <div className="task-detail-markdown task-detail-delivery">
                  <MarkdownContent content={rendered.summary ?? ''} />
                </div>
              ) : rendered.state === 'running' ? (
                // 执行中尚无结算是正常状态，不能与「终态却没有交付」混为一谈
                t('taskBubble.detail.pending')
              ) : (
                // 终态仍无交付：后端已保证至少留下兜底文案，走到这里说明该 run
                // 早于兜底逻辑存在（旧台账残留）——如实说明，不伪装成有内容
                t('taskBubble.detail.noSummary')
              )}
            </div>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  )
}
