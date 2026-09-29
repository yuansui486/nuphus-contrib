/**
 * TaskBubble 应用内拖拽 —— 任务列表面板（.task-track）
 *
 * 钉住的不变量（对应任务验收项）：
 *  ① 把手在 .task-track-header 内、标题左侧，样式走共享类 .panel-grip（与 DesktopToolbar 同源）
 *  ② 未拖拽过 → 面板保持 planner.css 的 right/bottom 定位（无 inline 坐标，零回归）
 *  ③ 拖拽 → 切 inline left/top 并内联清掉 right/bottom；松手落盘 task_track_pos
 *  ④ 刷新（重挂载）后从 localStorage 恢复
 *  ⑤ 边界钳制：拖不出视口；窗口变小后不飞出（clampOnResize）
 *  ⑥ 拖拽不干扰既有交互：行点击弹详情、关闭钮、进度条照旧；点把手不弹详情
 *  ⑦ key 独立：desktop_toolbar_pos 不串到本面板
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TaskBubble } from './TaskBubble'
import type { TaskRun } from '../../core/types'

vi.mock('./PreviewOverlay', () => ({ PreviewOverlay: () => null }))

const TRACK_KEY = 'task_track_pos'

function run(over: Partial<TaskRun> = {}): TaskRun {
  return {
    run_id: 'run-1',
    title: '整理本周会议纪要',
    task: '## 任务\n整理本周会议纪要并输出 Markdown',
    goal_type: 'file_operation',
    origin: null,
    attempt: 1,
    state: 'completed',
    started_at: 1000,
    settled_at: 9000,
    duration_ms: 8200,
    ok: true,
    summary: null,
    ...over,
  }
}

const track = () => document.querySelector('.task-track') as HTMLElement
const grip = () => track().querySelector('.panel-grip') as HTMLElement

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

describe('TaskBubble 应用内拖拽', () => {
  it('未拖拽过 → 面板保持 CSS 定位（无 inline left/top）', () => {
    render(<TaskBubble visible runs={[run()]} onClose={() => {}} />)
    expect(track().getAttribute('style')).toBeNull()
  })

  it('把手在 header 内、标题左侧（共享类 .panel-grip + title 拖拽移动）', () => {
    render(<TaskBubble visible runs={[run()]} onClose={() => {}} />)
    const header = track().querySelector('.task-track-header') as HTMLElement
    const title = track().querySelector('.task-track-title') as HTMLElement
    expect(header.firstElementChild).toBe(grip())
    expect(grip().className).toContain('panel-grip')
    expect(grip().getAttribute('title')).toBe('拖拽移动')
    // DOM 序：把手在标题之前
    expect(grip().compareDocumentPosition(title) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('拖拽 → 面板切 inline left/top（并清掉 right/bottom），松手落盘', () => {
    render(<TaskBubble visible runs={[run()]} onClose={() => {}} />)
    fireEvent.mouseDown(grip(), { clientX: 50, clientY: 70 })
    fireEvent.mouseMove(window, { clientX: 300, clientY: 200 })
    fireEvent.mouseUp(window)
    expect(track().style.left).toBe('250px') // 300 - 50
    expect(track().style.top).toBe('130px') // 200 - 70
    expect(track().style.right).toBe('auto')
    expect(track().style.bottom).toBe('auto')
    expect(localStorage.getItem(TRACK_KEY)).toBe(JSON.stringify({ x: 250, y: 130 }))
  })

  it('刷新（重挂载）后从 localStorage 恢复拖拽位置', () => {
    localStorage.setItem(TRACK_KEY, JSON.stringify({ x: 111, y: 222 }))
    render(<TaskBubble visible runs={[run()]} onClose={() => {}} />)
    expect(track().style.left).toBe('111px')
    expect(track().style.top).toBe('222px')
  })

  it('边界钳制：拖不出视口', () => {
    render(<TaskBubble visible runs={[run()]} onClose={() => {}} />)
    fireEvent.mouseDown(grip(), { clientX: 0, clientY: 0 })
    fireEvent.mouseMove(window, { clientX: 9999, clientY: 9999 })
    expect(track().style.left).toBe(`${window.innerWidth - 200}px`)
    expect(track().style.top).toBe(`${window.innerHeight - 40}px`)
  })

  it('窗口变小 → 已拖拽面板被钳回视口（不飞出）', () => {
    render(<TaskBubble visible runs={[run()]} onClose={() => {}} />)
    fireEvent.mouseDown(grip(), { clientX: 0, clientY: 0 })
    fireEvent.mouseMove(window, { clientX: 700, clientY: 600 })
    expect(track().style.left).toBe('700px')

    withViewport(300, 200, () => fireEvent(window, new Event('resize')))
    expect(track().style.left).toBe('100px') // min(700, 300-200)
    expect(track().style.top).toBe('160px') // min(600, 200-40)
  })

  it('点把手不弹详情；行的点击入口照旧（拖拽不干扰内部交互）', () => {
    render(<TaskBubble visible runs={[run({ title: '甲任务' })]} onClose={() => {}} />)
    fireEvent.click(grip())
    expect(document.querySelector('.pm-content')).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: '甲任务' }))
    expect(document.querySelector('.pm-content')).not.toBeNull()
  })

  it('关闭钮回归：拖拽接入后 onClose 照旧触发', () => {
    const onClose = vi.fn()
    render(<TaskBubble visible runs={[run()]} onClose={onClose} />)
    fireEvent.click(track().querySelector('.task-track-close') as HTMLElement)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('key 独立：desktop_toolbar_pos 不串到本面板', () => {
    localStorage.setItem('desktop_toolbar_pos', JSON.stringify({ x: 400, y: 300 }))
    render(<TaskBubble visible runs={[run()]} onClose={() => {}} />)
    expect(track().getAttribute('style')).toBeNull()
  })
})
