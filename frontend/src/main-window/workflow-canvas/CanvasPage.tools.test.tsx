import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import { CanvasPage } from './CanvasPage'
import { CanvasBackendContext } from './CanvasBackend'
import { canvasBackend, type Draft } from '../../workbench/api'

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('../../core/bridge', () => ({
  invoke: mocks.invoke,
  listen: async () => () => {},
}))
// Only replace the layout engine: the actual canvas, palette, inspector, gate,
// native API wrappers and workbench persistence adapter participate in this test.
vi.mock('@xyflow/react', () => ({
  ReactFlowProvider: ({ children }: { children: ReactNode }) => children,
  ReactFlow: ({
    nodes,
    onNodeDoubleClick,
    children,
  }: {
    nodes: { id: string }[]
    onNodeDoubleClick: (event: unknown, node: unknown) => void
    children: ReactNode
  }) => (
    <div>
      {nodes.map(node => (
        <button key={node.id} onClick={event => onNodeDoubleClick(event, node)}>
          打开 {node.id}
        </button>
      ))}
      {children}
    </div>
  ),
  Background: () => null,
  Controls: () => null,
  BackgroundVariant: { Dots: 'dots' },
  MarkerType: { ArrowClosed: 'arrow' },
  useReactFlow: () => ({
    fitView: vi.fn(),
    getViewport: () => ({ x: 0, y: 0, zoom: 1 }),
    getNodes: () => [],
    getZoom: () => 1,
    setCenter: vi.fn(),
    setViewport: vi.fn(),
    screenToFlowPosition: (point: unknown) => point,
  }),
  applyNodeChanges: (_: unknown, nodes: unknown) => nodes,
}))

it('recovers tools visibly and synchronizes the real palette, inspector and project save', async () => {
  localStorage.setItem('nuphus_language', 'zh')
  let draft: Draft = {
    project_id: 'shared-project',
    workflow_id: 'wf',
    revision: 1,
    layout_revision: 0,
    layout: {},
    updated_at: 0,
    authoring_mode: 'internal',
    document: {
      id: 'wf',
      name: '共享画布',
      status: 'Draft',
      steps: [{ id: 'first', name: '第一步', do: { tool: 'file_read', with: {} } }],
    },
  }
  let rejectTools!: (reason: Error) => void
  const unavailable = new Promise((_resolve, reject) => {
    rejectTools = reject
  })
  let toolCalls = 0
  mocks.invoke.mockImplementation(async (command, payload) => {
    if (command === 'wf_tools') {
      if (++toolCalls === 1) return unavailable
      return [
        {
          name: 'file_read',
          group: 'file',
          description: '读取文件',
          input_schema: {
            type: 'object',
            required: ['path'],
            properties: { path: { type: 'string', description: '文件路径' } },
          },
        },
      ]
    }
    if (command === 'wf_gate_status') return { locked: false, reason: 'idle' }
    if (command === 'list_models') return []
    if (command === 'workbench_call' && payload.operation === 'canvas.update') {
      draft = {
        ...draft,
        revision: draft.revision + 1,
        document: payload.args.operations[0].document,
      }
      return { draft, diagnostics: { passed: true, warnings: [], errors: [] } }
    }
    if (command === 'workbench_call' && payload.operation === 'workflow.validate')
      return { passed: true, warnings: [], errors: [] }
    if (command === 'workbench_call' && payload.operation === 'canvas.layout_update')
      return { ...draft, layout: payload.args.layout, layout_revision: draft.layout_revision + 1 }
    return null
  })
  render(
    <CanvasBackendContext.Provider value={canvasBackend(draft, vi.fn(), vi.fn())}>
      <CanvasPage workflowId="wf" onClose={vi.fn()} />
    </CanvasBackendContext.Provider>,
  )
  fireEvent.click(await screen.findByRole('button', { name: '打开 first' }))
  await act(async () => {
    rejectTools(new Error('temporarily unavailable'))
  })
  expect(await screen.findByText('工具加载失败')).toBeVisible()
  expect(await screen.findByText('工具信息加载失败，仍可手动编辑。')).toBeVisible()
  expect(toolCalls).toBe(1)
  // Retry from the inspector also restores the already mounted palette.
  fireEvent.click(screen.getAllByRole('button', { name: '重试加载' })[1])
  const files = await screen.findByRole('button', { name: '文件操作' })
  expect(screen.queryByText('工具信息加载失败，仍可手动编辑。')).not.toBeInTheDocument()
  fireEvent.mouseEnter(files.parentElement!)
  expect(screen.getByRole('textbox', { name: 'path' })).toBeVisible()
  fireEvent.change(screen.getByRole('textbox', { name: 'path' }), {
    target: { value: 'E:/existing.txt' },
  })
  fireEvent.blur(screen.getByRole('textbox', { name: 'path' }))
  fireEvent.click(screen.getByRole('button', { name: /file_readfile_read/ }))
  await waitFor(() => expect(document.querySelectorAll('[data-inspector-node]')).toHaveLength(1))
  fireEvent.keyDown(window, { key: 's', ctrlKey: true })
  await waitFor(() =>
    expect(
      mocks.invoke.mock.calls.some(
        ([command, payload]) =>
          command === 'workbench_call' && payload.operation === 'canvas.update',
      ),
    ).toBe(true),
  )
  expect(draft.document.steps).toHaveLength(2)
  expect(draft.document.steps[0].do).toEqual({
    tool: 'file_read',
    with: { path: 'E:/existing.txt' },
  })
  expect(draft.document.steps[1].do).toEqual({ tool: 'file_read', with: {} })
  expect(mocks.invoke).toHaveBeenCalledWith('wf_gate_status')
  expect(toolCalls).toBe(2)
})
