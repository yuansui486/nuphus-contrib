import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../locales'
import { AuthoringPanel } from './AuthoringPanel'
import type { Draft } from './api'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  stopListening: vi.fn(),
  listener: null as null | ((event: { payload: unknown }) => void),
  history: { busy: false, session: { messages: [] } } as {
    busy: boolean
    session: { messages: Array<{ role: string; content: Array<{ type: string; text: string }> }> }
  },
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: mocks.listen,
}))
vi.mock('../main-window/workflow-canvas/IntentFormPanel', () => ({
  IntentFormPanel: ({
    onSubmit,
    draftScope,
    submitLabel,
  }: {
    onSubmit: (form: unknown) => void
    draftScope: string
    submitLabel: string
  }) => (
    <div role="dialog" aria-label="Guided form" data-scope={draftScope}>
      <button
        onClick={() =>
          onSubmit({
            workflowName: 'Flow',
            stages: [
              { id: 'one', name: 'Prepare', steps: [{ id: 'step', intent: 'Wait briefly' }] },
            ],
          })
        }
      >
        {submitLabel}
      </button>
    </div>
  ),
}))
const draft: Draft = {
  project_id: 'project-one',
  workflow_id: 'flow-one',
  revision: 1,
  layout_revision: 0,
  authoring_mode: 'internal',
  document: { id: 'flow-one', name: 'Flow', steps: [], status: 'Draft' },
  layout: {},
  updated_at: 0,
}
const baseProps = () => ({
  draft,
  intent: '',
  onIntentConsumed: vi.fn(),
  onBusyChange: vi.fn(),
  onChanged: vi.fn(),
  beforeGenerate: vi.fn(async () => true),
  onCollapse: vi.fn(),
})
function setup(extra: Partial<Parameters<typeof AuthoringPanel>[0]> = {}) {
  const props = { ...baseProps(), ...extra }
  const view = render(
    <LangProvider>
      <AuthoringPanel {...props} />
    </LangProvider>,
  )
  return {
    ...view,
    props,
    rerenderPanel: (changes: Partial<typeof props>) =>
      view.rerender(
        <LangProvider>
          <AuthoringPanel {...props} {...changes} />
        </LangProvider>,
      ),
  }
}
const startCalls = () => mocks.invoke.mock.calls.filter(([, args]) => args.action === 'start')
async function ready() {
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      'workbench_generate',
      expect.objectContaining({ action: 'history' }),
    ),
  )
  await act(async () => {})
}
async function emit(type: string, extra: Record<string, unknown> = {}) {
  await act(async () =>
    mocks.listener?.({
      payload: {
        project_id: draft.project_id,
        workflow_id: draft.workflow_id,
        turn_id: 'turn-one',
        event: { type, ...extra },
      },
    }),
  )
}
beforeEach(() => {
  localStorage.setItem('nuphus_language', 'en')
  sessionStorage.clear()
  mocks.listener = null
  mocks.history = { busy: false, session: { messages: [] } }
  mocks.stopListening.mockReset()
  mocks.listen.mockReset().mockImplementation(async (_name, handler) => {
    mocks.listener = handler
    return mocks.stopListening
  })
  mocks.invoke
    .mockReset()
    .mockImplementation(async (_command, args) => (args.action === 'history' ? mocks.history : {}))
})

describe('Workbench assistant', () => {
  it('retries a failed history load without duplicating its listener or losing input', async () => {
    mocks.invoke.mockRejectedValueOnce(new Error('History temporarily unavailable'))
    setup()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Keep this request' } })
    expect(await screen.findByRole('alert')).toHaveTextContent('History temporarily unavailable')
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled())
    expect(mocks.listen).toHaveBeenCalledOnce()
    expect(mocks.invoke.mock.calls.filter(([, args]) => args.action === 'history')).toHaveLength(2)
    expect(screen.getByRole('textbox')).toHaveValue('Keep this request')
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Retry' })).not.toBeInTheDocument()
    expect(startCalls()).toHaveLength(0)
  })

  it('retries a failed event subscription and restores the actual busy state', async () => {
    mocks.listen.mockRejectedValueOnce(new Error('Event listener unavailable'))
    mocks.history.busy = true
    const { props } = setup()
    expect(await screen.findByRole('alert')).toHaveTextContent('Event listener unavailable')
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }))
    await screen.findByRole('button', { name: 'Stop' })
    expect(mocks.listen).toHaveBeenCalledTimes(2)
    expect(props.onBusyChange).toHaveBeenLastCalledWith(true)
    expect(screen.queryByRole('button', { name: 'Send' })).not.toBeInTheDocument()
  })

  it('handles a rejected subscription on unmount without a second rejected cleanup promise', async () => {
    mocks.listen.mockRejectedValueOnce(new Error('Cannot subscribe'))
    const { unmount } = setup()
    await screen.findByRole('alert')
    unmount()
    await act(async () => {})
    expect(mocks.stopListening).not.toHaveBeenCalled()
  })

  it('releases a subscription that resolves after the assistant is unmounted', async () => {
    let resolve: (unsubscribe: () => void) => void = () => {}
    mocks.listen.mockImplementationOnce(
      () =>
        new Promise<() => void>(done => {
          resolve = done
        }),
    )
    const { unmount } = setup()
    unmount()
    await act(async () => resolve(mocks.stopListening))
    expect(mocks.stopListening).toHaveBeenCalledOnce()
    expect(mocks.invoke).not.toHaveBeenCalled()
  })

  it('starts with a direct composer and only a guided entry; examples fill without sending', async () => {
    setup()
    await ready()
    expect(screen.getByRole('heading', { name: 'Workflow assistant' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Guided input' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Describe a task' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Process a spreadsheet' }))
    expect(screen.getByRole('textbox')).toHaveValue(
      'Read a project CSV, process each row, and summarize the results.',
    )
    expect(startCalls()).toHaveLength(0)
  })

  it('sends the description without an extra mode switch and clears its saved draft', async () => {
    const { props } = setup()
    await ready()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Wait two seconds' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(startCalls()).toHaveLength(1))
    expect(startCalls()[0][1]).toEqual({
      projectId: draft.project_id,
      workflowId: draft.workflow_id,
      action: 'start',
      input: 'Wait two seconds',
    })
    expect(screen.getByText('Wait two seconds')).toBeInTheDocument()
    expect(screen.getByRole('textbox')).toHaveValue('')
    expect(sessionStorage.getItem('workbench:authoring:project-one:flow-one')).toBe('')
    expect(props.onBusyChange).toHaveBeenLastCalledWith(true)
  })

  it.each([{ ctrlKey: true }, { metaKey: true }])(
    'supports a modifier+Enter shortcut (%j)',
    async modifier => {
      setup()
      await ready()
      fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Draft a workflow' } })
      fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter', ...modifier })
      await waitFor(() => expect(startCalls()).toHaveLength(1))
    },
  )

  it('does not send plain Enter or IME confirmation', async () => {
    setup()
    await ready()
    const field = screen.getByRole('textbox')
    fireEvent.change(field, { target: { value: '输入内容' } })
    fireEvent.keyDown(field, { key: 'Enter' })
    fireEvent.compositionStart(field)
    fireEvent.keyDown(field, { key: 'Enter', ctrlKey: true })
    fireEvent.compositionEnd(field)
    fireEvent.keyDown(field, { key: 'Enter', ctrlKey: true, isComposing: true })
    fireEvent.keyDown(field, { key: 'Enter', ctrlKey: true, keyCode: 229 })
    expect(startCalls()).toHaveLength(0)
  })

  it('appends guided details and canvas intents to existing text without auto-sending', async () => {
    const { rerenderPanel, props } = setup()
    await ready()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Keep my current notes.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Guided input' }))
    expect(screen.getByRole('dialog')).toHaveAttribute('data-scope', 'workbench:project-one')
    fireEvent.click(screen.getByRole('button', { name: 'Add to description' }))
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toContain(
      'Keep my current notes.\n\n',
    )
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toContain('Wait briefly')
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).not.toContain(
      'plugin/workflows/',
    )
    rerenderPanel({ intent: 'Canvas request' })
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toContain(
      '\n\nCanvas request',
    )
    expect(props.onIntentConsumed).toHaveBeenCalledOnce()
    expect(startCalls()).toHaveLength(0)
  })

  it('preserves the draft by project and workflow and does not clear it on collapse', async () => {
    sessionStorage.setItem('workbench:authoring:project-one:flow-one', 'Saved request')
    const { rerenderPanel, props } = setup()
    await ready()
    expect(screen.getByRole('textbox')).toHaveValue('Saved request')
    fireEvent.click(screen.getByRole('button', { name: 'Collapse assistant' }))
    expect(props.onCollapse).toHaveBeenCalledOnce()
    rerenderPanel({ collapsed: true })
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
    expect(mocks.stopListening).not.toHaveBeenCalled()
    rerenderPanel({ collapsed: false })
    expect(screen.getByRole('textbox')).toHaveValue('Saved request')
  })

  it('does not unlock generation when cancellation is only requested', async () => {
    mocks.history.busy = true
    const { props, rerenderPanel } = setup()
    await ready()
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        'workbench_generate',
        expect.objectContaining({ action: 'cancel' }),
      ),
    )
    expect(screen.getByRole('button', { name: 'Stopping…' })).toBeDisabled()
    expect(screen.queryByRole('button', { name: 'Send' })).not.toBeInTheDocument()
    expect(props.onBusyChange).toHaveBeenLastCalledWith(true)
    rerenderPanel({ collapsed: true })
    mocks.history.busy = false
    await emit('completed', { success: true })
    expect(props.onBusyChange).toHaveBeenLastCalledWith(false)
    expect(props.onChanged).toHaveBeenCalledOnce()
    rerenderPanel({ collapsed: false })
    expect(screen.getByRole('button', { name: 'Send' })).toBeInTheDocument()
  })

  it('keeps the input and displays errors when generation cannot start', async () => {
    setup()
    await ready()
    mocks.invoke.mockRejectedValueOnce(new Error('Configure a workflow model first'))
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'My request' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Configure a workflow model first')
    expect(screen.getByRole('textbox')).toHaveValue('My request')
    expect(screen.getByRole('button', { name: 'Send' })).toBeEnabled()
  })

  it('honors the save guard and prevents duplicate starts while it is pending', async () => {
    let release: (allowed: boolean) => void = () => {}
    const beforeGenerate = vi.fn(
      () =>
        new Promise<boolean>(resolve => {
          release = resolve
        }),
    )
    setup({ beforeGenerate })
    await ready()
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'My request' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    expect(beforeGenerate).toHaveBeenCalledOnce()
    await act(async () => release(false))
    expect(startCalls()).toHaveLength(0)
    expect(screen.getByRole('textbox')).toHaveValue('My request')
  })

  it('does not pull the conversation down while reading history and offers a latest button', async () => {
    mocks.history.busy = true
    setup()
    await ready()
    const log = screen.getByRole('log')
    Object.defineProperties(log, {
      scrollHeight: { configurable: true, value: 1000 },
      clientHeight: { configurable: true, value: 200 },
    })
    log.scrollTop = 300
    fireEvent.scroll(log)
    await emit('progress', { text: 'Added a wait node' })
    expect(log.scrollTop).toBe(300)
    expect(screen.getByText('Added a wait node')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Jump to latest' }))
    expect(log.scrollTop).toBe(1000)
    expect(screen.queryByRole('button', { name: 'Jump to latest' })).not.toBeInTheDocument()
  })
})
