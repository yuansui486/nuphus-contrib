import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkflowIR } from './types'
import { LangProvider } from '../../locales'

// ── 可变远端模型：同一 mock 同时供 wfGetRaw 与 wfSave 消费 ──
const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  validate: vi.fn(),
  refresh: vi.fn(),
  rf: {
    fitView: vi.fn(),
    getViewport: () => ({ x: 0, y: 0, zoom: 1 }),
    getNodes: () => [],
    getZoom: () => 1,
    setCenter: vi.fn(),
    setViewport: vi.fn(),
    screenToFlowPosition: (p: unknown) => p,
  },
}))

let remote: Record<string, unknown> | null
let wfGetRawImpl: () => Promise<Record<string, unknown> | null>

const workflow: WorkflowIR = {
  id: 'wf',
  name: '一致性测试',
  status: 'Draft',
  updated_at: '2026-01-01T00:00:00Z',
  steps: [{ id: 'first', name: '第一步', do: { script: { runtime: 'python', code: 'print(1)' } } }],
}

vi.mock('@xyflow/react', () => ({
  ReactFlowProvider: ({ children }: { children: React.ReactNode }) => children,
  ReactFlow: ({
    nodes,
    onNodeDoubleClick,
    onNodeClick,
    children,
  }: {
    nodes: { id: string }[]
    onNodeDoubleClick: (e: unknown, n: unknown) => void
    onNodeClick: (e: unknown, n: unknown) => void
    children: React.ReactNode
  }) => (
    <div>
      {nodes.map(node => (
        <span key={node.id}>
          <button onClick={e => onNodeDoubleClick(e, node)}>打开 {node.id}</button>
          <button onClick={e => onNodeClick(e, node)}>Select {node.id}</button>
        </span>
      ))}
      {children}
    </div>
  ),
  Background: () => null,
  Controls: () => null,
  BackgroundVariant: { Dots: 'dots' },
  MarkerType: { ArrowClosed: 'arrow' },
  useReactFlow: () => mocks.rf,
  applyNodeChanges: (_: unknown, nodes: unknown) => nodes,
}))

vi.mock('../lib/api', () => ({
  getLanguage: async () => '',
  wfGetRaw: () => wfGetRawImpl(),
  wfLayoutGet: async () => null,
  wfLayoutSave: async () => null,
  wfSave: (...args: unknown[]) => mocks.save(...args),
  wfValidate: (...args: unknown[]) => mocks.validate(...args),
  wfTools: async () => [],
  listModels: async () => [],
  wfScheduleHistoryGet: async () => null,
}))
vi.mock('../lib/useWorkflowGate', () => ({
  useWorkflowGate: () => ({ locked: false, refresh: mocks.refresh }),
}))
vi.mock('./runStatus', () => ({
  subscribeRunStatus: () => () => {},
  aggregateContainerBadges: () => new Map(),
}))
vi.mock('./ToolPalette', () => ({ ToolPalette: () => null, TOOL_DRAG_MIME: 'tool' }))
vi.mock('./ProblemsPanel', () => ({
  ProblemsPanel: ({ backendReport }: { backendReport: unknown }) => (
    <output data-testid="backend-check">{JSON.stringify(backendReport)}</output>
  ),
}))
vi.mock('./OutlinePanel', () => ({ OutlinePanel: () => null }))
vi.mock('./IntentFormPanel', () => ({ IntentFormPanel: () => null }))
vi.mock('./WorkflowSwitcher', () => ({ WorkflowSwitcher: () => null }))
vi.mock('./WorkflowInputsEditor', () => ({
  WorkflowInputsEditor: ({
    open,
    onApply,
  }: {
    open: boolean
    onApply: (inputs: unknown[]) => void
  }) =>
    open ? (
      <button onClick={() => onApply([{ name: 'new_input', type: 'string' }])}>应用测试输入</button>
    ) : null,
}))
vi.mock('../workflow/WorkflowScheduleDialog', () => ({ WorkflowScheduleDialog: () => null }))
vi.mock('../workflow/WorkflowInputsForm', () => ({
  WorkflowInputsDialog: () => null,
  NO_INPUT_SPECS: [],
}))
vi.mock('./ScopedEditDialog', () => ({ ScopedEditDialog: () => null }))
import { CanvasPage } from './CanvasPage'

const panel = (id: string) =>
  within(document.querySelector(`[data-inspector-node="${id}"]`) as HTMLElement)

const CONFLICT_MSG =
  '该工作流在画布外被修改过（如定时设置、重命名或运行记录）。继续保存将覆盖这些修改，仍要继续？'

beforeEach(() => {
  vi.clearAllMocks()
  remote = structuredClone(workflow) as unknown as Record<string, unknown>
  wfGetRawImpl = async () => remote
  mocks.refresh.mockResolvedValue({ locked: false })
  mocks.save.mockImplementation(async (wf: Record<string, unknown>) => {
    // 对齐后端 wf_save 行为：校验通过 → 强制刷新 updated_at → 覆盖写
    remote = { ...wf, updated_at: new Date().toISOString() }
    return { saved: true, report: { passed: true, issues: [] } }
  })
  mocks.validate.mockResolvedValue({ passed: true, issues: [] })
})

/** Flush the mocked API promises and cascading effects before querying canvas nodes. */
async function renderCanvas() {
  let view!: ReturnType<typeof render>
  await act(async () => {
    view = render(
      <LangProvider>
        <CanvasPage workflowId="wf" onClose={() => {}} />
      </LangProvider>,
    )
  })
  return view
}

async function openNode(id: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: `打开 ${id}` }))
  })
}

function shortcut() {
  fireEvent.keyDown(window, { key: 's', ctrlKey: true })
}

describe('I-4 load failure fallback', () => {
  it('shows error state with retry instead of hanging on Loading when wfGetRaw rejects', async () => {
    wfGetRawImpl = async () => {
      throw new Error('bridge invoke failed')
    }
    await renderCanvas()
    expect(screen.getByRole('alert')).toHaveTextContent('加载画布失败')
    expect(screen.getByRole('alert')).toHaveTextContent('bridge invoke failed')
    expect(screen.queryByText('加载画布中…')).not.toBeInTheDocument()

    // 重试成功后正常进入画布
    wfGetRawImpl = async () => remote
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '重试' }))
    })
    await waitFor(() => expect(document.querySelector('.wfc-title')).not.toBeNull())
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('loads exactly once per workflowId despite re-renders (draft-safety via ref)', async () => {
    const wfGetRawSpy = vi.fn(wfGetRawImpl)
    wfGetRawImpl = wfGetRawSpy
    await renderCanvas()
    await waitFor(() => expect(document.querySelector('.wfc-title')).not.toBeNull())
    const loadsAfterMount = wfGetRawSpy.mock.calls.length
    expect(loadsAfterMount).toBeGreaterThan(0)
    // 触发一轮重渲染（未提交草稿 + 通知清除等），加载 effect 不得重跑：
    // 重载会 setSteps 覆盖未提交草稿，等价于一次静默数据丢失
    await openNode('first')
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '草稿' } })
    await act(async () => {})
    expect(wfGetRawSpy.mock.calls.length).toBe(loadsAfterMount)
  })
})

describe('I-2 undo/redo invalidates dirty drafts', () => {
  /**
   * 2026-10-17：撤销/重做/保存/运行从 `.wfc-toolbar` 移到**画布 header**
   * （`.workflow-workbench-header` 右端挂载点，CanvasPage 经 portal 注入）。
   * 单测直接渲染 CanvasPage、没有 header 挂载点 → 走降级路径落回工具栏末尾。
   * 因此这里断言：主路径存在时不得留在工具栏；本用例覆盖的是降级契约。
   */
  it('有 header 挂载点时主操作组由 portal 注入；无挂载点时降级回工具栏', async () => {
    // ① 无挂载点（本测试环境）：四项必须仍可用（降级），不能被吞掉
    const plain = await renderCanvas()
    const fallback = document.querySelector('.wfc-toolbar .workflow-workbench-actions-group')
    expect(fallback).toBeTruthy()
    expect(
      within(fallback as HTMLElement).getByRole('button', { name: '撤销（Ctrl+Z）' }),
    ).toBeTruthy()
    expect(
      within(fallback as HTMLElement).getByRole('button', { name: '重做（Ctrl+Shift+Z）' }),
    ).toBeTruthy()
    plain.unmount()

    // ② 有挂载点：四项进 header，工具栏内不再有这一组
    const host = document.createElement('div')
    host.id = 'workflow-workbench-actions'
    document.body.appendChild(host)
    try {
      await renderCanvas()
      expect(within(host).getByRole('button', { name: '撤销（Ctrl+Z）' })).toBeTruthy()
      expect(within(host).getByRole('button', { name: '运行' })).toBeTruthy()
      expect(document.querySelector('.wfc-toolbar .workflow-workbench-actions-group')).toBeNull()
    } finally {
      host.remove()
    }
  })

  it('外部输入与定时常驻在工具栏，且「更多」排在最右端（尾部）', async () => {
    await renderCanvas()
    const toolbar = document.querySelector('.wfc-toolbar') as HTMLElement
    for (const name of [/^外部输入/, /^定时/]) {
      const btn = screen.getByRole('button', { name })
      expect(btn.closest('.wfc-more-menu')).toBeNull()
      expect(btn.closest('.wfc-toolbar')).toBe(toolbar)
    }
    // 「更多」必须是工具栏这一行里的最后一个可交互元素
    const summary = toolbar.querySelector('.wfc-more > summary')
    expect(summary).toBeTruthy()
    const tail = summary!.closest('.wfc-more')!
    const siblings = Array.from(tail.parentElement!.children)
    expect(siblings[siblings.length - 1]).toBe(tail)
  })

  it('discards an uncommitted name draft on undo so the flushed save keeps the undone value', async () => {
    await renderCanvas()
    await openNode('first')
    // 提交一次改名，构造可撤销历史
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '第二版' } })
    await waitFor(() => expect(panel('first').getByLabelText(/^名称/)).toHaveValue('第二版'))
    // 失焦提交，再输入新的未提交草稿
    fireEvent.blur(panel('first').getByLabelText(/^名称/))
    await act(async () => {})
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '未提交草稿' } })
    // undo：整体替换 steps，脏草稿必须失效
    await act(async () => {
      fireEvent.keyDown(window, { key: 'z', ctrlKey: true })
    })
    await waitFor(() => expect(panel('first').getByLabelText(/^名称/)).toHaveValue('第一步'))
    // 此时保存：撤销结果不被旧草稿覆盖
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(mocks.save.mock.calls[0][0].steps[0].name).toBe('第一步')
  })

  it('discards an uncommitted draft on redo as well', async () => {
    await renderCanvas()
    await openNode('first')
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '第二版' } })
    fireEvent.blur(panel('first').getByLabelText(/^名称/))
    await act(async () => {})
    // undo 回到初版 → redo 回到第二版
    await act(async () => {
      fireEvent.keyDown(window, { key: 'z', ctrlKey: true })
    })
    await waitFor(() => expect(panel('first').getByLabelText(/^名称/)).toHaveValue('第一步'))
    // redo 前留下未提交草稿
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '重做前草稿' } })
    await act(async () => {
      fireEvent.keyDown(window, { key: 'z', ctrlKey: true, shiftKey: true })
    })
    await waitFor(() => expect(panel('first').getByLabelText(/^名称/)).toHaveValue('第二版'))
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(mocks.save.mock.calls[0][0].steps[0].name).toBe('第二版')
  })
})

describe('I-1 third-party write guard', () => {
  it('warns and requires confirmation before overwriting a remote change', async () => {
    await renderCanvas()
    await openNode('first')
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '画布改名' } })
    // 第三方写者：保存前远端 updated_at 变了（如列表页重命名 / 调度器）
    await act(async () => {
      remote = { ...remote, name: '列表页改名', updated_at: '2026-02-02T00:00:00Z' }
    })
    shortcut()
    expect(await screen.findByText(CONFLICT_MSG)).toBeInTheDocument()
    expect(mocks.save).not.toHaveBeenCalled()

    // 取消 → 不保存；基线同步为远端，下次保存不再重复弹窗
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByText(CONFLICT_MSG)).not.toBeInTheDocument())
    expect(mocks.save).not.toHaveBeenCalled()
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(screen.queryByText(CONFLICT_MSG)).not.toBeInTheDocument()
  })

  it('proceeds without dialog when the remote is unchanged', async () => {
    await renderCanvas()
    await openNode('first')
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '正常保存' } })
    shortcut()
    await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(1))
    expect(screen.queryByText(CONFLICT_MSG)).not.toBeInTheDocument()
  })

  it('aborts save when the preflight wfGetRaw rejects', async () => {
    await renderCanvas()
    await openNode('first')
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '预检失败' } })
    wfGetRawImpl = async () => {
      throw new Error('preflight bridge failure')
    }
    shortcut()
    await waitFor(() =>
      expect(screen.getByText(/预检 bridge failure|preflight bridge failure/)).toBeInTheDocument(),
    )
    expect(mocks.save).not.toHaveBeenCalled()
  })

  it('aborts save when the workflow no longer exists', async () => {
    await renderCanvas()
    await openNode('first')
    fireEvent.change(panel('first').getByLabelText(/^名称/), { target: { value: '已被删除' } })
    await act(async () => {
      remote = null
    })
    shortcut()
    await waitFor(() =>
      expect(screen.getByText('保存失败：工作流不存在或已被删除')).toBeInTheDocument(),
    )
    expect(mocks.save).not.toHaveBeenCalled()
  })
})
