/**
 * WorkflowRunModal 应用内拖拽 —— 工作流运行面板（CompactModal 卡片 + dragKey）
 *
 * 钉住的不变量（对应任务验收项）：
 *  ① 把手渲染在面板头部（.compact-header）首位，走共享类 .panel-grip
 *  ② 未拖拽过 → 卡片保持 flex 居中（无 inline 定位，观感零回归）
 *  ③ 拖拽 → 卡片脱离居中、按 inline position:fixed + left/top；松手落盘 workflow_run_pos
 *  ④ 重开（重挂载）后从 localStorage 恢复
 *  ⑤ 边界钳制：拖不出视口；窗口变小后不飞出
 *  ⑥ 拖拽不干扰既有交互：遮罩点击关闭、内部「启动」按钮照旧；点把手不误关面板
 *  ⑦ key 独立：task_track_pos 不串到本面板
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WorkflowRunModal } from './WorkflowRunModal'
import type { WorkflowInputSpec, WorkflowItem } from '../../core/types'

const MODAL_KEY = 'workflow_run_pos'

function workflowWith(inputs?: WorkflowInputSpec[]): WorkflowItem {
  return {
    id: 'wf-1',
    title: '测试工作流',
    steps: [{ id: 's1', name: '读文件', do: { tool: 'Read', with: { path: 'a.txt' } } }],
    tags: [],
    created_at: 0,
    updated_at: 0,
    run_count: 0,
    status: 'draft',
    inputs,
  }
}

const card = () => document.querySelector('.compact-modal') as HTMLElement
const grip = () => document.querySelector('.compact-header .panel-grip') as HTMLElement

/** jsdom 无布局：hook 的面板尺寸走 200×40 兜底，innerWidth/Height 换可写属性后再还原 */
function withViewport(w: number, h: number, run_: () => void) {
  const wDesc = Object.getOwnPropertyDescriptor(window, 'innerWidth')!
  const hDesc = Object.getOwnPropertyDescriptor(window, 'innerHeight')!
  try {
    Object.defineProperty(window, 'innerWidth', { value: w, configurable: true, writable: true })
    Object.defineProperty(window, 'innerHeight', { value: h, configurable: true, writable: true })
    run_()
  } finally {
    Object.defineProperty(window, 'innerWidth', wDesc)
    Object.defineProperty(window, 'innerHeight', hDesc)
  }
}

beforeEach(() => {
  localStorage.clear()
})
afterEach(cleanup)

describe('WorkflowRunModal 应用内拖拽', () => {
  it('把手渲染在面板头部（.compact-header）首位', () => {
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={vi.fn()} />)
    const header = document.querySelector('.compact-header') as HTMLElement
    expect(header.firstElementChild).toBe(grip())
    expect(grip().getAttribute('title')).toBe('拖拽移动')
  })

  it('未拖拽过 → 卡片保持 flex 居中（无 inline 定位）', () => {
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={vi.fn()} />)
    expect(card().getAttribute('style')).toBeNull()
  })

  it('拖拽 → 卡片脱离居中、按 inline fixed left/top 定位，并落盘', () => {
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={vi.fn()} />)
    fireEvent.mouseDown(grip(), { clientX: 20, clientY: 30 })
    fireEvent.mouseMove(window, { clientX: 400, clientY: 300 })
    fireEvent.mouseUp(window)
    expect(card().style.position).toBe('fixed')
    expect(card().style.left).toBe('380px') // 400 - 20
    expect(card().style.top).toBe('270px') // 300 - 30
    expect(localStorage.getItem(MODAL_KEY)).toBe(JSON.stringify({ x: 380, y: 270 }))
  })

  it('重开（重挂载）后从 localStorage 恢复拖拽位置', () => {
    localStorage.setItem(MODAL_KEY, JSON.stringify({ x: 111, y: 222 }))
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={vi.fn()} />)
    expect(card().style.position).toBe('fixed')
    expect(card().style.left).toBe('111px')
    expect(card().style.top).toBe('222px')
  })

  it('边界钳制：拖不出视口', () => {
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={vi.fn()} />)
    fireEvent.mouseDown(grip(), { clientX: 0, clientY: 0 })
    fireEvent.mouseMove(window, { clientX: 9999, clientY: 9999 })
    expect(card().style.left).toBe(`${window.innerWidth - 200}px`)
    expect(card().style.top).toBe(`${window.innerHeight - 40}px`)
  })

  it('窗口变小 → 已拖拽卡片被钳回视口（不飞出）', () => {
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={vi.fn()} />)
    fireEvent.mouseDown(grip(), { clientX: 0, clientY: 0 })
    fireEvent.mouseMove(window, { clientX: 700, clientY: 600 })

    withViewport(300, 200, () => fireEvent(window, new Event('resize')))
    expect(card().style.left).toBe('100px') // min(700, 300-200)
    expect(card().style.top).toBe('160px') // min(600, 200-40)
  })

  it('遮罩点击关闭回归：拖拽接入后照旧可关', async () => {
    const onCancel = vi.fn()
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={onCancel} />)
    fireEvent.click(document.querySelector('.compact-overlay') as HTMLElement)
    await waitFor(() => expect(onCancel).toHaveBeenCalledTimes(1))
  })

  it('点把手不误关面板（卡片 stopPropagation，冒泡不到遮罩）', () => {
    const onCancel = vi.fn()
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={onCancel} />)
    fireEvent.mouseDown(grip(), { clientX: 10, clientY: 10 })
    fireEvent.mouseUp(grip(), { clientX: 10, clientY: 10 })
    fireEvent.click(grip())
    expect(onCancel).not.toHaveBeenCalled()
  })

  it('拖拽后内部「启动」交互回归（onRun 照旧触发）', () => {
    const onRun = vi.fn()
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={onRun} onCancel={vi.fn()} />)
    fireEvent.mouseDown(grip(), { clientX: 0, clientY: 0 })
    fireEvent.mouseMove(window, { clientX: 200, clientY: 150 })
    fireEvent.click(screen.getByRole('button', { name: '启动' }))
    expect(onRun).toHaveBeenCalledWith('wf-1', undefined)
  })

  it('key 独立：task_track_pos 不串到本面板', () => {
    localStorage.setItem('task_track_pos', JSON.stringify({ x: 400, y: 300 }))
    render(<WorkflowRunModal open workflow={workflowWith()} onRun={vi.fn()} onCancel={vi.fn()} />)
    expect(card().getAttribute('style')).toBeNull()
  })
})
