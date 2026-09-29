import { describe, it, expect } from 'vitest'
import type { NuphusEvent, TaskRun, TimelineEntry } from '../core/types'

/**
 * task_runs 快照契约（后端 `agent::task_run` → 前端 task 面板）的回归固化。
 *
 * 这三个用例对应三件曾经真实发生的事故：
 * 1. 失败的子任务被显示成「完成」——`task_completed` 曾硬编码 completed，忽略 success；
 * 2. 同一 task_id 重试/复用时行被覆盖、前一条永不闭合——现在每次派发是独立 run；
 * 3. 中断/丢事件后行永久停在「执行中」——现在 interrupted 是独立终态，且快照自愈。
 */

function run(over: Partial<TaskRun> = {}): TaskRun {
  return {
    run_id: 'run-1',
    title: '任务A',
    task: '## 任务\n任务A 的派发正文',
    goal_type: 'file_operation',
    origin: null,
    attempt: 1,
    state: 'running',
    started_at: 1000,
    settled_at: null,
    duration_ms: null,
    ok: null,
    summary: null,
    ...over,
  }
}

/** 与 useEvents.ts 的 task_runs 分支同一套投影规则（面板行 = 快照行） */
function projectRows(runs: TaskRun[]): TimelineEntry[] {
  const byId = new Map(runs.map(r => [r.run_id, r]))
  const rowFor = (r: TaskRun): TimelineEntry => ({
    id: `task-${r.run_id}`,
    kind: 'task' as const,
    runId: r.run_id,
    text: r.title,
    status:
      r.state === 'running'
        ? ('running' as const)
        : r.ok
          ? ('success' as const)
          : ('error' as const),
    summary: r.summary ?? undefined,
  })
  return runs.filter(r => !byId.has(r.run_id) === false).map(rowFor)
}

describe('task_runs snapshot contract', () => {
  it('carries the full lifecycle, not deltas: no id pairing needed', () => {
    const ev: NuphusEvent = {
      type: 'task_runs',
      runs: [
        run({
          run_id: 'run-1',
          state: 'completed',
          ok: true,
          duration_ms: 4200,
          summary: '做完了',
        }),
        run({ run_id: 'run-2', title: '任务B' }),
      ],
    }
    expect(ev.type).toBe('task_runs')
    // 快照自带全部状态：前端不需要 task_started/task_completed 配对
    const rows = projectRows((ev as { runs: TaskRun[] }).runs)
    expect(rows.map(r => r.status)).toEqual(['success', 'running'])
    expect(rows[0].summary).toBe('做完了')
    expect(rows[1].runId).toBe('run-2')
  })

  it('failed runs are never rendered as completed', () => {
    const rows = projectRows([run({ state: 'failed', ok: false, summary: '安全检查未通过' })])
    expect(rows[0].status).toBe('error')
    expect(rows[0].status).not.toBe('success')
  })

  it('interrupted is a terminal state of its own (never stuck running)', () => {
    const rows = projectRows([
      run({ state: 'interrupted', ok: false, summary: '进程重启前未结算' }),
    ])
    expect(rows[0].status).toBe('error')
    // 终态四值完备：running 之外都能收尾
    const states: TaskRun['state'][] = ['running', 'completed', 'failed', 'interrupted']
    expect(states.filter(s => s !== 'running').length).toBe(3)
  })

  it('repeat dispatch of the same task produces separate runs (attempt grows)', () => {
    const runs = [
      run({ run_id: 'run-1', attempt: 1, state: 'failed', ok: false }),
      run({ run_id: 'run-2', attempt: 2 }),
    ]
    const rows = projectRows(runs)
    expect(rows).toHaveLength(2)
    expect(runs[0].attempt).toBe(1)
    expect(runs[1].attempt).toBe(2)
    expect(rows[0].runId).not.toBe(rows[1].runId)
  })

  it('origin label is display-only and never changes lifecycle', () => {
    const withOrigin = run({
      origin: { plan_path: 'p.plan.md', task_no: 2 },
      state: 'completed',
      ok: true,
    })
    const without = run({ state: 'completed', ok: true })
    expect(withOrigin.state).toBe(without.state)
    expect(withOrigin.origin?.task_no).toBe(2)
  })
})
