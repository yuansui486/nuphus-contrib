import React from 'react'
import type { TaskRun, TaskRunState } from '../../core/types'
import { useLanguage } from '../../locales'
import { MorphIcon } from 'morphicons/react'
// morphicon 的 icon 入参只认 `lucide` 包的图标组件（与 ChatInputBar / WorkflowRunCard 同源）；
// ui/Icons 是 lucide-react 的再导出，两者类型不兼容，勿混用。
import {
  Check as CheckIcon,
  CircleX as CircleXIcon,
  CircleAlert as AlertIcon,
  Loader2 as LoaderIcon,
} from 'lucide'
import { IconX } from '../../ui/Icons'
import '../../styles/task-bubble.css'

interface TaskBubbleProps {
  visible: boolean
  /** ExecAgent 执行生命周期快照（服务端台账、本轮的投影） */
  runs: TaskRun[]
  onClose: () => void
}

/** 台账状态 → 展示样式 class。interrupted 单独一档：绝不伪装成完成 */
const STATE_CLASS: Record<TaskRunState, string> = {
  running: 'running',
  completed: 'completed',
  failed: 'failed',
  interrupted: 'cancelled',
}

/** 状态 → 形变目标图标（切换 icon prop 即触发 morphicon 平滑形变） */
function stateIcon(state: TaskRunState) {
  switch (state) {
    case 'running':
      return LoaderIcon
    case 'completed':
      return CheckIcon
    case 'failed':
      return CircleXIcon
    case 'interrupted':
      return AlertIcon
  }
}

function fmtDuration(ms: number | null): string {
  if (ms == null) return ''
  if (ms < 1000) return `${ms}ms`
  const s = ms / 1000
  if (s < 60) return `${s.toFixed(1)}s`
  return `${Math.floor(s / 60)}m${Math.round(s % 60)}s`
}

/**
 * 任务面板 —— ExecAgent 执行生命周期的**只读投影**（一轮一面墙）。
 *
 * 数据只有一个来源：后端 `agent::task_run` 台账的本轮快照（NuphusEvent::TaskRuns）。
 * 不做 id 配对、不做事件对账、不维护状态机；新一轮开始即归零，新旧计划/派发不会混。
 *
 * Nuphus 是**串行执行**（铁律：无并行机制、禁止并行）——同一轮内一笔派发结清才开下一笔，
 * 因此面板里同时 `running` 的永远只有一行；其余行是本轮已结清的历史，用来回答
 * 「这一轮已经干了什么、还剩多少笔没结」。
 *
 * 刻意**不**展示 ExecAgent 的交付摘要：那是 Exec 给 Leader 的汇报，不是给用户的；
 * 过程与结论细节由执行面板（时间线 + task_dispatch 展开）承载，小面板只回答
 * 「这一轮在跑什么、跑到第几笔、快慢如何」。
 */
export const TaskBubble: React.FC<TaskBubbleProps> = ({ visible, runs, onClose }) => {
  const { t } = useLanguage()

  if (!visible || runs.length === 0) return null

  const running = runs.filter(r => r.state === 'running')
  const settled = runs.filter(r => r.state !== 'running')
  const failed = runs.filter(r => r.state === 'failed').length
  const interrupted = runs.filter(r => r.state === 'interrupted').length

  return (
    <div className="task-track">
      <div className="task-track-header">
        <div className="task-track-title">
          {running.length > 0 ? (
            <>
              <span className="task-track-shimmer" />
              <span className="task-track-active-name">
                {/* Nuphus 串行执行（铁律：无并行机制）——同一轮内一笔结清才开下一笔，
                    所以正常情况下这里永远只有一个 running；这里不做「并行」表述 */}
                {running.length === 1
                  ? running[0].title
                  : `${running[0].title} 等 ${running.length} 项`}
              </span>
            </>
          ) : (
            <span className="task-track-idle">{t('taskBubble.idle')}</span>
          )}
        </div>
        <button className="task-track-close" onClick={onClose}>
          <IconX size={12} />
        </button>
      </div>

      {/* 聚合口径：先给「这一轮一共多少笔、结了多少」，避免把逐项派发误读成一个小任务 */}
      <div className="task-track-progress">
        <div className="task-track-bar">
          <div
            className="task-track-fill"
            style={{ width: `${runs.length > 0 ? (settled.length / runs.length) * 100 : 0}%` }}
          />
        </div>
        <span className="task-track-count">
          {`${settled.length}/${runs.length}`}
          {failed > 0 ? t('taskBubble.failed', String(failed)) : ''}
          {interrupted > 0 ? t('taskBubble.interrupted', String(interrupted)) : ''}
        </span>
      </div>

      <div className="task-track-list">
        {runs.map(run => {
          const cls = STATE_CLASS[run.state]
          return (
            <div key={run.run_id} className={`task-track-item ${cls}`}>
              <div className="task-track-item-row">
                {/* 同一 MorphIcon 常驻、只切 icon prop —— 分支条件渲染会重挂导致形变失效 */}
                <span className={`task-track-dot ${cls}`}>
                  <MorphIcon
                    icon={stateIcon(run.state)}
                    size={11}
                    spring="snappy"
                    className={`task-track-glyph${run.state === 'running' ? ' spin' : ''}`}
                  />
                </span>
                <span className="task-track-name">{run.title}</span>
                {run.attempt > 1 && <span className="task-track-attempt">#{run.attempt}</span>}
                {run.origin?.task_no != null && (
                  <span className="task-track-origin">{`计划#${run.origin.task_no}`}</span>
                )}
                {fmtDuration(run.duration_ms) && (
                  <span className="task-track-duration">{fmtDuration(run.duration_ms)}</span>
                )}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
