import { useEffect, useMemo, useRef, useState } from 'react'
import { useLanguage } from '../locales'
import { IconClock3 } from '../ui/Icons'
import { WorkflowScheduleDialog } from '../main-window/workflow/WorkflowScheduleDialog'
import { call, canvasBackend, type Draft } from './api'
import type { ScheduleConfig } from '../core/types'
import type { WorkflowRunTrace } from '../main-window/lib/api'

export interface ScheduleSummary {
  workflow_id: string
  config: ScheduleConfig
  next_at: number | null
}
interface Attempt {
  id: string
  workflow_id: string
  workflow_title: string
  due_at: number
  status: string
  reason: string | null
  run_id: string | null
}

export function ScheduleControl({
  draft,
  summary,
  onChanged,
}: {
  draft: Draft
  summary?: ScheduleSummary
  onChanged: () => void
}) {
  const { lang } = useLanguage()
  const [editing, setEditing] = useState<Draft | null>(null)
  const changed = useRef(onChanged)
  changed.current = onChanged
  const backend = useMemo(
    () =>
      editing
        ? canvasBackend(
            editing,
            () => changed.current(),
            () => {},
          ).schedule
        : undefined,
    [editing],
  )
  const text = !summary
    ? lang === 'zh'
      ? '设置定时'
      : 'Set schedule'
    : summary.config.enabled
      ? lang === 'zh'
        ? '定时已启用'
        : 'Schedule enabled'
      : lang === 'zh'
        ? '定时已暂停'
        : 'Schedule disabled'
  return (
    <div className="wb-schedule-control">
      <button onClick={() => setEditing(draft)}>
        <IconClock3 size={14} />
        {text}
      </button>
      {summary?.next_at && (
        <small>
          {lang === 'zh' ? '下次：' : 'Next: '}
          {new Date(summary.next_at).toLocaleString()}
        </small>
      )}
      {editing && (
        <WorkflowScheduleDialog
          open
          workflow={{
            id: editing.workflow_id,
            title: editing.document.name,
            inputs: editing.document.inputs,
            schedule: summary?.config,
          }}
          backend={backend}
          notice={
            lang === 'zh'
              ? '执行最新已保存内容。驻留托盘时继续，退出应用后停止；桌面 RPA 需要可用的登录会话。'
              : 'Runs the latest saved workflow while the app is running, including in the tray. Desktop RPA needs an available signed-in session.'
          }
          onClose={() => setEditing(null)}
          onChanged={() => {
            setEditing(null)
            changed.current()
          }}
        />
      )}
    </div>
  )
}

export function ScheduleHistory({ projectId }: { projectId: string }) {
  const { lang } = useLanguage()
  const ui = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const [rows, setRows] = useState<Attempt[]>([])
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  useEffect(() => {
    let alive = true
    let timer: ReturnType<typeof setTimeout>
    const poll = async () => {
      try {
        const result = await call<Attempt[]>('workflow.schedule.history', { project_id: projectId })
        if (alive) {
          setRows(result)
          setError('')
        }
      } catch (error) {
        if (alive) setError(String(error))
      }
      if (alive) timer = setTimeout(() => void poll(), 2000)
    }
    void poll()
    return () => {
      alive = false
      clearTimeout(timer)
    }
  }, [projectId, refresh])
  const reasons: Record<string, string> = {
    previous_run_active: ui(
      '上次运行仍在执行或等待确认，本次跳过',
      'Previous run is still active or awaiting confirmation',
    ),
    automation_busy: ui(
      '执行资源被占用，本次跳过',
      'Execution resources are busy; occurrence skipped',
    ),
    missed_while_unavailable: ui(
      '应用或电脑暂不可用，错过的任务不补跑',
      'Missed while unavailable; not replayed',
    ),
    validation_failed: ui(
      '最新工作流校验失败，请在画布检查问题',
      'Latest workflow is invalid; review canvas problems',
    ),
    invalid_inputs: ui(
      '运行输入不符合最新定义，请修改定时输入',
      'Fixed inputs no longer match the workflow; edit the schedule',
    ),
    host_restarted: ui(
      '应用重启中断了触发准备，未自动重放',
      'Dispatch interrupted by restart; not replayed',
    ),
    schedule_secret_unavailable: ui(
      '无法读取本机保存的输入，请重新填写',
      'Cannot read saved inputs on this machine; re-enter them',
    ),
  }
  const statuses: Record<string, string> = {
    starting: ui('准备中', 'Starting'),
    running: ui('运行中', 'Running'),
    completed: ui('已完成', 'Completed'),
    failed: ui('失败', 'Failed'),
    skipped: ui('已跳过', 'Skipped'),
    paused: ui('已暂停', 'Paused'),
    awaiting_human: ui('等待确认', 'Awaiting confirmation'),
    cancelled: ui('已取消', 'Cancelled'),
    interrupted: ui('已中断', 'Interrupted'),
  }
  return (
    <section className="wb-schedule-history">
      <div className="wb-row">
        <h2>{ui('定时触发记录', 'Scheduled occurrences')}</h2>
        <button
          disabled={!rows.length}
          onClick={() => {
            if (
              !window.confirm(
                ui(
                  '清理已结束的定时触发记录？工作流和运行详情将保留。',
                  'Clear finished schedule history? Workflows and run evidence will be retained.',
                ),
              )
            )
              return
            void call('workflow.schedule.history_delete', { project_id: projectId })
              .then(() => setRefresh(n => n + 1))
              .catch(e => setError(String(e)))
          }}
        >
          {ui('清理定时历史', 'Clear schedule history')}
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
      {!rows.length && !error && <p>{ui('暂无定时触发记录', 'No scheduled occurrences yet')}</p>}
      {rows.map(row => (
        <details className="wb-run" key={row.id}>
          <summary>
            {row.workflow_title} · {statuses[row.status] ?? row.status} ·{' '}
            {new Date(row.due_at).toLocaleString()}
          </summary>
          {row.reason && <p>{reasons[row.reason] ?? row.reason}</p>}
          {row.run_id && <RunEvidence projectId={projectId} runId={row.run_id} />}
        </details>
      ))}
    </section>
  )
}

function RunEvidence({ projectId, runId }: { projectId: string; runId: string }) {
  const { lang } = useLanguage()
  const [trace, setTrace] = useState<WorkflowRunTrace | null>(null)
  const [evidence, setEvidence] = useState<Record<number, unknown>>({})
  const [error, setError] = useState('')
  const [loaded, setLoaded] = useState(false)
  return (
    <div>
      <code>{runId}</code>
      <button
        onClick={() => {
          void call<WorkflowRunTrace | null>('run.steps', { project_id: projectId, run_id: runId })
            .then(t => {
              setTrace(t)
              setLoaded(true)
              setError('')
            })
            .catch(e => setError(String(e)))
        }}
      >
        {lang === 'zh' ? '查看步骤详情' : 'View step evidence'}
      </button>
      {error && <p role="alert">{error}</p>}
      {loaded && !trace && (
        <p>{lang === 'zh' ? '本次未产生步骤记录' : 'No step evidence was produced'}</p>
      )}
      {trace?.invocations.map(step => (
        <details
          key={step.id}
          onToggle={e => {
            if (e.currentTarget.open && !evidence[step.id])
              void call('run.steps', {
                project_id: projectId,
                run_id: runId,
                invocation_id: step.id,
              })
                .then(value => setEvidence(old => ({ ...old, [step.id]: value })))
                .catch(e => setError(String(e)))
          }}
        >
          <summary>
            {step.step_name || step.step_id} · {step.status}
          </summary>
          <pre>{JSON.stringify(evidence[step.id] ?? step, null, 2)}</pre>
        </details>
      ))}
    </div>
  )
}
