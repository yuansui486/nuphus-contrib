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
