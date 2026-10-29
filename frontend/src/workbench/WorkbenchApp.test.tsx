import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../locales'
import WorkbenchApp from './WorkbenchApp'
import type { Draft } from './api'
import { useEffect } from 'react'

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), leave: vi.fn(async () => true) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: async () => null }))
vi.mock('../hooks/useTheme', () => ({ useTheme: () => ({ toggleTheme: vi.fn() }) }))
vi.mock('../main-window/layout/TitleBar', () => ({ TitleBar: () => <header>Workbench</header> }))
vi.mock('../main-window/workflow-canvas/CanvasPage', () => ({
  CanvasPage: ({
    onSwitchWorkflow,
    registerLeaveGuard,
  }: {
    onSwitchWorkflow: (id: string) => void
    registerLeaveGuard: (guard: null | (() => Promise<boolean>)) => void
  }) => {
    useEffect(() => {
      registerLeaveGuard(mocks.leave)
      return () => registerLeaveGuard(null)
    }, [registerLeaveGuard])
    return (
      <div>
        Canvas editor
        <button onClick={() => onSwitchWorkflow('flow-one')}>Canvas switch workflow</button>
      </div>
    )
  },
}))
vi.mock('./AuthoringPanel', () => ({
  AuthoringPanel: ({
    collapsed,
    onBusyChange,
  }: {
    collapsed: boolean
    onBusyChange: (busy: boolean) => void
  }) => (
    <div hidden={collapsed}>
      Internal composer
      <input aria-label="Assistant draft" />
      <button onClick={() => onBusyChange(true)}>Start generation</button>
      <button onClick={() => onBusyChange(false)}>Finish generation</button>
    </div>
  ),
}))
const draft = (project: string, mode: 'internal' | 'external' = 'internal'): Draft => ({
  project_id: project,
  workflow_id: `flow-${project}`,
  revision: 1,
  layout_revision: 0,
  document: { id: `flow-${project}`, name: `Flow ${project}`, steps: [], status: 'Draft' },
  authoring_mode: mode,
  layout: {},
  updated_at: 0,
})
beforeEach(() => {
  localStorage.clear()
  vi.spyOn(window, 'confirm').mockReturnValue(false)
  mocks.leave.mockReset().mockResolvedValue(true)
  localStorage.setItem('nuphus_language', 'en')
  mocks.invoke.mockReset()
  mocks.invoke.mockImplementation(async (command, payload) => {
    if (command !== 'workbench_call') return null
    if (payload.operation === 'project.list')
      return [
        { project_id: 'one', name: 'One', directory: '/one' },
        { project_id: 'two', name: 'Two', directory: '/two' },
      ]
    if (payload.operation === 'workflow.list') return [draft(payload.args.project_id)]
    if (payload.operation === 'run.list') return []
  })
})
describe('workflow-first Workbench shell', () => {
  it('cancels deletion without sending a request or opening the canvas', async () => {
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: 'Delete workflow: Flow one' }))
    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining('Flow one'))
    expect(
      mocks.invoke.mock.calls.filter(([, args]) => args?.operation === 'workflow.delete'),
    ).toHaveLength(0)
    expect(screen.getByRole('button', { name: /^Flow one/ })).toBeInTheDocument()
    expect(screen.queryByText('Canvas editor')).not.toBeInTheDocument()
  })
  it('deletes the selected revision once and refreshes the list while keeping history', async () => {
    vi.mocked(window.confirm).mockReturnValue(true)
    let finish!: (value: unknown) => void
    let removed = false
    const response = new Promise(resolve => {
      finish = resolve
    })
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) => {
      if (payload?.operation === 'workflow.delete') {
        await response
        removed = true
        return { deleted: true }
      }
      if (payload?.operation === 'workflow.list' && removed) return []
      return fallback(command, payload)
    })
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    const button = await screen.findByRole('button', { name: 'Delete workflow: Flow one' })
    fireEvent.click(button)
    fireEvent.click(button)
    expect(button).toBeDisabled()
    expect(screen.getByRole('button', { name: /^Flow one/ })).toBeDisabled()
    expect(
      mocks.invoke.mock.calls.filter(([, args]) => args?.operation === 'workflow.delete'),
    ).toHaveLength(1)
    expect(mocks.invoke).toHaveBeenCalledWith('workbench_call', {
      operation: 'workflow.delete',
      args: { project_id: 'one', workflow_id: 'flow-one', revision: 1 },
    })
    await act(async () => {
      finish({})
      await response
    })
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /^Flow one/ })).not.toBeInTheDocument(),
    )
    expect(screen.getByRole('status')).toHaveTextContent('versions and run history retained')
  })
  it('refreshes a stale deletion conflict without automatically retrying or removing the card', async () => {
    vi.mocked(window.confirm).mockReturnValue(true)
    let conflicted = false
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) => {
      if (payload?.operation === 'workflow.delete') {
        conflicted = true
        throw { code: 'revision_conflict', message: 'Reload the changed draft' }
      }
      if (payload?.operation === 'workflow.list' && conflicted)
        return [{ ...draft('one'), revision: 2 }]
      return fallback(command, payload)
    })
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: 'Delete workflow: Flow one' }))
    await screen.findByText(/revision_conflict/)
    expect(screen.getByRole('button', { name: /^Flow one/ })).toHaveTextContent('Revision 2')
    expect(screen.getByRole('button', { name: 'Delete workflow: Flow one' })).toBeEnabled()
    expect(
      mocks.invoke.mock.calls.filter(([, args]) => args?.operation === 'workflow.delete'),
    ).toHaveLength(1)
  })
  it('keeps the newly selected project intact when an old project deletion finishes', async () => {
    vi.mocked(window.confirm).mockReturnValue(true)
    let finish!: (value: unknown) => void
    const response = new Promise(resolve => {
      finish = resolve
    })
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) =>
      payload?.operation === 'workflow.delete' ? response : fallback(command, payload),
    )
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: 'Delete workflow: Flow one' }))
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    fireEvent.change(screen.getByRole('combobox', { name: 'Current project' }), {
      target: { value: 'two' },
    })
    await screen.findByRole('button', { name: /^Flow two/ })
    await act(async () => {
      finish({ deleted: true })
      await response
    })
    expect(screen.getByRole('button', { name: /^Flow two/ })).toBeInTheDocument()
    expect(screen.queryByText(/Workflow deleted/)).not.toBeInTheDocument()
  })
  it.each([
    ['two', 'one', 'one', 'two'],
    ['missing', 'two', 'one', 'two'],
    ['missing', 'missing', 'two', 'two'],
    ['missing', 'missing', 'missing', 'one'],
  ])(
    'restores a shared project with legacy fallback (%s, %s, %s)',
    async (shared, current, historical, expected) => {
      localStorage.setItem('workbench:last-project', shared)
      localStorage.setItem('workbench:current:last-project', current)
      localStorage.setItem('workbench:historical:last-project', historical)
      const view = render(
        <LangProvider>
          <WorkbenchApp storageScope="current" />
        </LangProvider>,
      )
      await screen.findByRole('button', { name: new RegExp(`^Flow ${expected}`) })
      await waitFor(() => expect(localStorage.getItem('workbench:last-project')).toBe(expected))
      expect(localStorage.getItem('workbench:historical:last-project')).toBe(historical)
      view.unmount()
      render(
        <LangProvider>
          <WorkbenchApp storageScope="another-account" />
        </LangProvider>,
      )
      await screen.findByRole('button', { name: new RegExp(`^Flow ${expected}`) })
    },
  )
  it('shows account controls only inside settings', async () => {
    render(
      <LangProvider>
        <WorkbenchApp accountSettings={<button>Sign out</button>} />
      </LangProvider>,
    )
    await screen.findByRole('button', { name: /^Flow one/ })
    expect(screen.queryByRole('button', { name: 'Sign out' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    const account = screen.getByRole('region', { name: 'Account and authorization' })
    expect(account).toContainElement(screen.getByRole('button', { name: 'Sign out' }))
    fireEvent.click(screen.getByRole('button', { name: 'Close settings' }))
    expect(screen.queryByRole('button', { name: 'Sign out' })).not.toBeInTheDocument()
  })
  it('does not reopen a slow create after the user navigates to runs', async () => {
    let finish!: (value: Draft) => void
    const response = new Promise<Draft>(resolve => {
      finish = resolve
    })
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) =>
      payload?.operation === 'canvas.create' ? response : fallback(command, payload),
    )
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    await screen.findByRole('button', { name: /^Flow one/ })
    fireEvent.click(screen.getByRole('button', { name: 'New canvas' }))
    fireEvent.click(screen.getByRole('button', { name: 'Runs' }))
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Runs' })).toHaveAttribute('aria-current', 'page'),
    )
    await act(async () => {
      finish(draft('one'))
      await response
    })
    expect(screen.queryByText('Canvas editor')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Runs' })).toHaveAttribute('aria-current', 'page')
  })
  it('does not reopen a slow mode update after the user leaves the editor', async () => {
    let finish!: (value: { draft: Draft }) => void
    const response = new Promise<{ draft: Draft }>(resolve => {
      finish = resolve
    })
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) => {
      if (payload?.operation === 'workflow.list') return [draft('one', 'external')]
      if (payload?.operation === 'canvas.update') return response
      return fallback(command, payload)
    })
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: /^Flow one/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Enable AI assistant' }))
    await waitFor(() =>
      expect(mocks.invoke.mock.calls.some(([, args]) => args?.operation === 'canvas.update')).toBe(
        true,
      ),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Runs' }))
    await waitFor(() => expect(screen.queryByText('Canvas editor')).not.toBeInTheDocument())
    await act(async () => {
      finish({ draft: draft('one') })
      await response
    })
    expect(screen.queryByText('Canvas editor')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Runs' })).toHaveAttribute('aria-current', 'page')
  })
  it('moves project selection into settings and restores the previous project', async () => {
    localStorage.setItem('workbench:last-project', 'two')
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    await screen.findByRole('button', { name: /^Flow two/ })
    expect(screen.queryByRole('combobox', { name: 'Current project' })).not.toBeInTheDocument()
    const settings = screen.getByRole('button', { name: 'Settings' })
    settings.focus()
    fireEvent.click(settings)
    expect(screen.getByRole('combobox', { name: 'Current project' })).toHaveValue('two')
    expect(screen.getByText('/two')).toBeInTheDocument()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(settings).toHaveFocus()
  })
  it('falls back visibly when the saved project no longer exists', async () => {
    localStorage.setItem('workbench:last-project', 'missing')
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    await screen.findByRole('button', { name: /^Flow one/ })
    expect(screen.getByText(/previous project is unavailable/)).toBeInTheDocument()
    expect(localStorage.getItem('workbench:last-project')).toBe('one')
  })
  it('collapses the assistant without discarding input or switching authoring mode', async () => {
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: /^Flow one/ }))
    const field = screen.getByRole('textbox', { name: 'Assistant draft' })
    fireEvent.change(field, { target: { value: 'Keep this request' } })
    fireEvent.click(screen.getByRole('button', { name: 'AI assistant' }))
    expect(field).not.toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'AI assistant' }))
    expect(field).toHaveValue('Keep this request')
    expect(field).toBeVisible()
    expect(mocks.invoke.mock.calls.some(([, args]) => args?.operation === 'canvas.update')).toBe(
      false,
    )
  })
  it('keeps generation locked even when the panel is collapsed and blocks canvas workflow switching', async () => {
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: /^Flow one/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Start generation' }))
    fireEvent.click(screen.getByRole('button', { name: 'AI assistant' }))
    expect(screen.getByRole('status', { name: 'Generating' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Canvas switch workflow' }))
    expect(screen.getByText(/Stop generation before switching canvases/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'AI assistant' })).toHaveAttribute(
      'aria-pressed',
      'false',
    )
    fireEvent.click(screen.getByRole('button', { name: 'Back to workflows' }))
    expect(screen.getByText('Canvas editor')).toBeInTheDocument()
  })
  it('keeps dirty editor navigation under the existing leave guard', async () => {
    mocks.leave.mockResolvedValue(false)
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: /^Flow one/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Runs' }))
    await waitFor(() => expect(mocks.leave).toHaveBeenCalled())
    expect(screen.getByText('Canvas editor')).toBeInTheDocument()
  })
  it('creates and opens a blank canvas immediately without asking for a name', async () => {
    const fallback = mocks.invoke.getMockImplementation()!
    let finish: (value: Draft) => void = () => {}
    mocks.invoke.mockImplementation(async (command, payload) =>
      payload?.operation === 'canvas.create'
        ? new Promise<Draft>(resolve => {
            finish = resolve
          })
        : fallback(command, payload),
    )
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    await screen.findByRole('button', { name: /^Flow one/ })
    expect(screen.queryByRole('button', { name: /English|中文/ })).not.toBeInTheDocument()
    const create = screen.getByRole('button', { name: 'New canvas' })
    fireEvent.click(create)
    fireEvent.click(create)
    expect(
      mocks.invoke.mock.calls.filter(([, args]) => args?.operation === 'canvas.create'),
    ).toHaveLength(1)
    expect(mocks.invoke).toHaveBeenCalledWith('workbench_call', {
      operation: 'canvas.create',
      args: { project_id: 'one', name: 'Untitled workflow' },
    })
    expect(screen.queryByLabelText('Workflow name')).not.toBeInTheDocument()
    await act(async () => finish(draft('one')))
    expect(screen.getByText('Canvas editor')).toBeInTheDocument()
  })
  it('shows ready-to-use endpoints without client registration or permission checkboxes', async () => {
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) =>
      command === 'workbench_clients'
        ? {
            status: 'listening',
            url: 'http://127.0.0.1:47731',
            mcp_url: 'http://127.0.0.1:47731/mcp',
            local: {
              status: 'listening',
              available: true,
              executable: 'E:/Apps/nuphus-workbench-mcp.exe',
              config: { mcpServers: {} },
            },
          }
        : fallback(command, payload),
    )
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Connections' }))
    await screen.findByText('Local service ready · Full access')
    expect(screen.getByText('http://127.0.0.1:47731/mcp')).toBeInTheDocument()
    expect(screen.queryByLabelText('Client name')).not.toBeInTheDocument()
    expect(screen.queryAllByRole('checkbox')).toHaveLength(0)
    expect(
      mocks.invoke.mock.calls
        .filter(([cmd]) => cmd === 'workbench_clients')
        .every(([, args]) => args.action === 'status'),
    ).toBe(true)
  })
  it('shows a port failure instead of claiming the service is ready', async () => {
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) =>
      command === 'workbench_clients'
        ? { status: 'failed', message: 'Port occupied' }
        : fallback(command, payload),
    )
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Connections' }))
    await screen.findByText('Port occupied')
    expect(screen.queryByText('Service running · Full access')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Copy address' })).not.toBeInTheDocument()
  })
  it('opens the workflow list in English and exposes internal generation only for internal canvases', async () => {
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: /^Flow one/ }))
    expect(screen.getByText('Canvas editor')).toBeInTheDocument()
    expect(screen.getByText('Internal composer')).toBeInTheDocument()
  })
  it('does not require a model or show an internal composer for external authoring', async () => {
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) =>
      payload?.operation === 'workflow.list'
        ? [draft('one', 'external')]
        : fallback(command, payload),
    )
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(await screen.findByRole('button', { name: /^Flow one/ }))
    expect(screen.getByText('Canvas editor')).toBeInTheDocument()
    expect(screen.queryByText('Internal composer')).not.toBeInTheDocument()
    expect(mocks.invoke.mock.calls.some(([command]) => command === 'workbench_generate')).toBe(
      false,
    )
  })
  it('ignores an old project response after the user switches projects', async () => {
    let completeOld: (value: Draft[]) => void = () => {}
    const oldRequest = new Promise<Draft[]>(resolve => {
      completeOld = resolve
    })
    const fallback = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation(async (command, payload) =>
      payload?.operation === 'workflow.list' && payload.args.project_id === 'one'
        ? oldRequest
        : fallback(command, payload),
    )
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    await waitFor(() =>
      expect(screen.getByRole('combobox', { name: 'Current project' })).toHaveValue('one'),
    )
    fireEvent.change(screen.getByRole('combobox', { name: 'Current project' }), {
      target: { value: 'two' },
    })
    await screen.findByRole('button', { name: /^Flow two/ })
    await act(async () => {
      completeOld([draft('one')])
      await oldRequest
    })
    expect(screen.getByRole('button', { name: /^Flow two/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /^Flow one/ })).not.toBeInTheDocument()
  })
})
