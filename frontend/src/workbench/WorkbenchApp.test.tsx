import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../locales'
import WorkbenchApp from './WorkbenchApp'
import type { Draft } from './api'

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: async () => null }))
vi.mock('../hooks/useTheme', () => ({ useTheme: () => ({ toggleTheme: vi.fn() }) }))
vi.mock('../main-window/layout/TitleBar', () => ({ TitleBar: () => <header>Workbench</header> }))
vi.mock('../main-window/workflow-canvas/CanvasPage', () => ({
  CanvasPage: () => <div>Canvas editor</div>,
}))
vi.mock('./AuthoringPanel', () => ({ AuthoringPanel: () => <div>Internal composer</div> }))
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
    await screen.findByRole('button', { name: /Flow one/ })
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
          }
        : fallback(command, payload),
    )
    render(
      <LangProvider>
        <WorkbenchApp />
      </LangProvider>,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Connections' }))
    await screen.findByText('Service running · Full access')
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
    fireEvent.click(await screen.findByRole('button', { name: /Flow one/ }))
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
    fireEvent.click(await screen.findByRole('button', { name: /Flow one/ }))
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
    await waitFor(() =>
      expect(screen.getByRole('combobox', { name: 'Current project' })).toHaveValue('one'),
    )
    fireEvent.change(screen.getByRole('combobox', { name: 'Current project' }), {
      target: { value: 'two' },
    })
    await screen.findByRole('button', { name: /Flow two/ })
    await act(async () => {
      completeOld([draft('one')])
      await oldRequest
    })
    expect(screen.getByRole('button', { name: /Flow two/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Flow one/ })).not.toBeInTheDocument()
  })
})
