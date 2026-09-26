import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../locales'
import { ScheduleControl, ScheduleHistory } from './ScheduleControls'
import {
  formatScheduleTime,
  WorkflowScheduleDialog,
} from '../main-window/workflow/WorkflowScheduleDialog'
import type { Draft } from './api'
const ipc = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: ipc }))
vi.mock('../core/bridge', () => ({ invoke: ipc }))
const draft: Draft = {
  project_id: 'p',
  workflow_id: 'wf',
  revision: 7,
  layout_revision: 0,
  authoring_mode: 'external',
  document: {
    id: 'wf',
    name: 'Test workflow',
    status: 'Draft',
    steps: [],
    inputs: [{ name: 'token', type: 'string', sensitive: true, required: true }],
  },
  layout: {},
  updated_at: 0,
}
const config = { cron: '* * * * *', timezone: 'UTC', enabled: true, interval_minutes: 5 }
beforeEach(() => {
  ipc.mockReset()
  localStorage.setItem('nuphus_language', 'en')
  ipc.mockImplementation(async (command, args) => {
    if (args?.operation === 'workflow.schedule.get' || command === 'wf_schedule_get')
      return { revision: 7, config, inputs: {}, sensitive_inputs: ['token'], eligible: true }
    if (args?.operation === 'workflow.schedule.preview') return [Date.now() + 60_000]
    if (command === 'wf_schedule_preview') return [new Date().toISOString()]
    if (
      args?.operation === 'workflow.schedule.set' ||
      args?.operation === 'workflow.schedule.remove'
    )
      return { draft: { ...draft, revision: 8 } }
  })
})
describe('Workbench scheduling UI', () => {
  it('previews in the selected timezone and tolerates incomplete timezone edits', () => {
    const value = '2026-09-26T00:00:00Z'
    expect(formatScheduleTime(value, 'Asia/Shanghai')).toBe(
      new Date(value).toLocaleString(undefined, {
        timeZone: 'Asia/Shanghai',
        timeZoneName: 'short',
      }),
    )
    expect(formatScheduleTime(value, 'Asia/')).toBe(new Date(value).toLocaleString())
  })
  it.each(['en', 'zh'])(
    'edits schedules in %s without calling the legacy workflow store',
    async lang => {
      localStorage.setItem('nuphus_language', lang)
      const changed = vi.fn()
      render(
        <LangProvider>
          <ScheduleControl draft={draft} onChanged={changed} />
        </LangProvider>,
      )
      fireEvent.click(
        screen.getByRole('button', { name: lang === 'en' ? 'Set schedule' : '设置定时' }),
      )
      const interval = await screen.findByLabelText(
        lang === 'en' ? 'Interval (minutes)' : '间隔（分钟）',
      )
      fireEvent.change(interval, { target: { value: '90' } })
      expect(
        screen.getByPlaceholderText(
          lang === 'en' ? 'Saved secret; leave blank to keep' : '已保存敏感值，留空保持',
        ),
      ).toHaveValue('')
      fireEvent.click(
        screen.getByRole('button', { name: lang === 'en' ? 'Save and apply' : '保存并应用' }),
      )
      await waitFor(() => expect(changed).toHaveBeenCalled())
      expect(ipc).toHaveBeenCalledWith(
        'workbench_call',
        expect.objectContaining({
          operation: 'workflow.schedule.set',
          args: expect.objectContaining({
            project_id: 'p',
            workflow_id: 'wf',
            revision: 7,
            config: expect.objectContaining({ interval_minutes: 90 }),
            inputs: {},
            preserve_sensitive: ['token'],
          }),
        }),
      )
      expect(ipc.mock.calls.some(([command]) => command.startsWith('wf_schedule'))).toBe(false)
    },
  )
  it('keeps the original editor default connected to legacy commands', async () => {
    render(
      <LangProvider>
        <WorkflowScheduleDialog
          open
          workflow={{ id: 'legacy', title: 'Legacy' }}
          onClose={() => {}}
          onChanged={() => {}}
        />
      </LangProvider>,
    )
    await screen.findByLabelText('Frequency')
    expect(ipc).toHaveBeenCalledWith('wf_schedule_get', { id: 'legacy' })
    expect(ipc.mock.calls.some(([command]) => command === 'workbench_call')).toBe(false)
  })
  it('shows skipped reasons and loads actual step evidence on demand', async () => {
    ipc.mockImplementation(async (_command, args) => {
      if (args.operation === 'workflow.schedule.history')
        return [
          {
            id: 'attempt',
            workflow_id: 'wf',
            workflow_title: 'Test workflow',
            due_at: Date.now(),
            status: 'skipped',
            reason: 'automation_busy',
            run_id: 'run-one',
          },
        ]
      if (args.operation === 'run.steps')
        return args.invocation_id
          ? { inputs: { text: 'test' }, output: 'done' }
          : { invocations: [{ id: 1, step_id: 'step', step_name: 'Wait', status: 'success' }] }
    })
    render(
      <LangProvider>
        <ScheduleHistory projectId="p" />
      </LangProvider>,
    )
    await screen.findByText(/Test workflow · Skipped/)
    const outer = screen.getByText(/Test workflow · Skipped/).closest('details')!
    outer.open = true
    fireEvent(outer, new Event('toggle'))
    expect(screen.getByText('Execution resources are busy; occurrence skipped')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'View step evidence' }))
    const step = await screen.findByText('Wait · success')
    const details = step.closest('details')!
    details.open = true
    fireEvent(details, new Event('toggle'))
    await waitFor(() =>
      expect(ipc).toHaveBeenCalledWith('workbench_call', {
        operation: 'run.steps',
        args: { project_id: 'p', run_id: 'run-one', invocation_id: 1 },
      }),
    )
  })
})
