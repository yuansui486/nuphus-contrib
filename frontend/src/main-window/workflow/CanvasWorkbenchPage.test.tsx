import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { CanvasWorkbenchPage } from './CanvasWorkbenchPage'
import { CanvasWorkbenchLoading } from './CanvasWorkbenchLoading'

// 画布页 lazy 拆包：测试中用轻量桩替换真实模块，
// 避免把工作流画布的依赖（@xyflow/react 等）拖进 jsdom。
vi.mock('../lib/api', () => ({
  listWorkflows: vi.fn(async () => [{ id: 'wf-1', status: 'draft', updated_at: 1 }]),
  wfSave: vi.fn(async () => ({ saved: true })),
}))
vi.mock('../workflow-canvas/CanvasPage', () => ({
  // 透出 workflowId：用于断言「列表选中哪个，画布就打开哪个」
  CanvasPage: ({ workflowId }: { workflowId: string }) => (
    <div data-testid="canvas-page">{workflowId}</div>
  ),
}))

describe('CanvasWorkbenchPage 首帧反馈与单形态画布', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('懒加载外壳期间保留顶部原生拖动区', () => {
    const { container } = render(<CanvasWorkbenchLoading />)
    expect(screen.getByText('加载中...')).toBeInTheDocument()
    expect(container.querySelector('header [data-tauri-drag-region]')).not.toBeNull()
    expect(container.querySelector('.page-loading')).not.toHaveAttribute('data-tauri-drag-region')
  })

  it('加载期与画布就绪后始终保留独立拖动区，不覆盖按钮或页面内容', async () => {
    const onClose = vi.fn()
    const { container } = render(<CanvasWorkbenchPage onClose={onClose} />)
    const drag = container.querySelector('header [data-tauri-drag-region]')
    expect(drag).not.toBeNull()
    expect(drag?.childElementCount).toBe(0)
    await screen.findByTestId('canvas-page')
    expect(container.querySelector('header [data-tauri-drag-region]')).toBe(drag)
    expect(container.querySelector('[data-tauri-drag-region] button')).toBeNull()
    expect(screen.getByTestId('canvas-page').closest('[data-tauri-drag-region]')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: '关闭' }))
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('外壳与加载态在首帧同步可见，不等待 IPC / 子页面 chunk', async () => {
    render(<CanvasWorkbenchPage onClose={() => {}} />)

    // 首帧断言：无 Provider 时 useLanguage 回退到 zh，'common.loading' = '加载中...'
    expect(screen.getByText('加载中...')).toBeInTheDocument()

    // 收尾：等在 act 内落地的异步状态，避免 act 警告
    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
  })

  it('画布不再有工作类型切换条（工具页已移出为独立界面）', async () => {
    render(<CanvasWorkbenchPage onClose={() => {}} />)
    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
    // 工具入口属于桌面工具条，画布内不得残留任何工具 tab 按钮
    expect(screen.queryByRole('button', { name: '工具' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '工作流编辑' })).not.toBeInTheDocument()
  })

  it('传入 workflowId 时直接编辑该工作流，跳过自选草稿', async () => {
    render(<CanvasWorkbenchPage workflowId="wf-explicit" onClose={() => {}} />)

    // 指定了目标：不应出现自选阶段的加载态
    expect(screen.queryByText('加载中...')).not.toBeInTheDocument()
    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
    expect(screen.getByTestId('canvas-page').textContent).toBe('wf-explicit')

    // 也不应再去查询草稿列表
    const { listWorkflows } = await import('../lib/api')
    expect(listWorkflows).not.toHaveBeenCalled()
  })

  it('未传 workflowId 时回退为自选最近更新的草稿', async () => {
    render(<CanvasWorkbenchPage onClose={() => {}} />)

    await waitFor(() => expect(screen.getByTestId('canvas-page')).toBeInTheDocument())
    expect(screen.getByTestId('canvas-page').textContent).toBe('wf-1')
  })
})
