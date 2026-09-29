/**
 * usePanelDrag 单测 —— 应用内面板拖拽的共享机制（从 DesktopToolbar 抽出的单一实现点）
 *
 * 钉住的不变量（对应任务验收项）：
 *  ① 初始站位：无存储 → fallback（有）/ null（无 = 未拖拽过，调用方不写 inline 坐标）
 *  ② mousedown 记抓取偏移 → mousemove 位置 = 鼠标坐标 - 偏移（把手按在哪儿都不跳面板）
 *  ③ 视口边界钳制：拖不出视口（x/y 各钳在 [0, innerWidth-offsetWidth]）
 *  ④ mouseup 落盘 localStorage；点了把手但没移动 → 不落盘（「未拖拽过」语义保持）
 *  ⑤ 重新挂载（模拟刷新）从 localStorage 恢复
 *  ⑥ clampOnResize：窗口变小后已拖拽面板被钳回视口；未开启 → 位置不动（DesktopToolbar 零变化）
 *  ⑦ enabled=false → mousedown 空转、无落盘
 *  ⑧ localStorage 值损坏（手改/旧版残留）→ 退回 fallback，渲染期不崩
 *
 * jsdom 局限（盲区）：getBoundingClientRect 恒为 0、offsetWidth/Height 为 0（走 hook 内
 * 200×40 兜底），真实鼠标事件流的按钮态/拖动手感由真机验收，这里断言坐标与钳制逻辑。
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { usePanelDrag, type UsePanelDragOptions } from './usePanelDrag'

const KEY = 'test_panel_pos'

/** 最小消费方：与三个真实面板同构（ref 挂根节点、把手接 onMouseDown、pos → inline left/top） */
function Harness({ options }: { options?: UsePanelDragOptions }) {
  const { pos, panelRef, handleMouseDown } = usePanelDrag(KEY, options)
  return (
    <div ref={panelRef} data-testid="panel" style={pos ? { left: pos.x, top: pos.y } : undefined}>
      <div data-testid="grip" className="panel-grip" onMouseDown={handleMouseDown} />
    </div>
  )
}

const panel = () => screen.getByTestId('panel')
const grip = () => screen.getByTestId('grip')

/** 在把手处按下并拖到指定视口坐标 */
function dragTo(x: number, y: number, from = { x: 0, y: 0 }) {
  fireEvent.mouseDown(grip(), { clientX: from.x, clientY: from.y })
  fireEvent.mouseMove(window, { clientX: x, clientY: y })
}

/** jsdom 的 innerWidth/innerHeight 是只读 getter：换可写属性模拟窗口尺寸变化，用完还原 */
function withViewport(w: number, h: number, run: () => void) {
  const wDesc = Object.getOwnPropertyDescriptor(window, 'innerWidth')!
  const hDesc = Object.getOwnPropertyDescriptor(window, 'innerHeight')!
  try {
    Object.defineProperty(window, 'innerWidth', { value: w, configurable: true, writable: true })
    Object.defineProperty(window, 'innerHeight', { value: h, configurable: true, writable: true })
    run()
  } finally {
    Object.defineProperty(window, 'innerWidth', wDesc)
    Object.defineProperty(window, 'innerHeight', hDesc)
  }
}

beforeEach(() => {
  localStorage.clear()
})
afterEach(cleanup)

describe('usePanelDrag 初始站位', () => {
  it('无存储、无 fallback → pos 为 null（未拖拽过，不写 inline 坐标）', () => {
    render(<Harness />)
    expect(panel().getAttribute('style')).toBeNull()
    expect(localStorage.getItem(KEY)).toBeNull()
  })

  it('无存储、有 fallback → 初值即 fallback（DesktopToolbar {x:669,y:60} 语义）', () => {
    render(<Harness options={{ fallback: { x: 669, y: 60 } }} />)
    expect(panel().style.left).toBe('669px')
    expect(panel().style.top).toBe('60px')
  })

  it('localStorage 有值 → 初值取存储（刷新后位置还在）', () => {
    localStorage.setItem(KEY, JSON.stringify({ x: 111, y: 222 }))
    render(<Harness options={{ fallback: { x: 669, y: 60 } }} />)
    expect(panel().style.left).toBe('111px')
    expect(panel().style.top).toBe('222px')
  })

  it('存储值损坏（not-json）→ 退回 fallback，渲染期不崩', () => {
    localStorage.setItem(KEY, 'not-json{{{')
    render(<Harness options={{ fallback: { x: 669, y: 60 } }} />)
    expect(panel().style.left).toBe('669px')
    expect(panel().style.top).toBe('60px')
  })
})

describe('usePanelDrag 拖拽与落盘', () => {
  it('mousedown 记抓取偏移 → mousemove 位置 = 鼠标坐标 - 偏移', () => {
    render(<Harness />)
    dragTo(300, 200, { x: 50, y: 70 })
    expect(panel().style.left).toBe('250px')
    expect(panel().style.top).toBe('130px')
  })

  it('mouseup 落盘（刷新后从 localStorage 恢复）', () => {
    render(<Harness />)
    dragTo(300, 200, { x: 50, y: 70 })
    fireEvent.mouseUp(window)
    expect(localStorage.getItem(KEY)).toBe(JSON.stringify({ x: 250, y: 130 }))
  })

  it('点了把手但没移动 → 不落盘，「未拖拽过」语义保持', () => {
    render(<Harness />)
    fireEvent.mouseDown(grip(), { clientX: 10, clientY: 10 })
    fireEvent.mouseUp(window)
    expect(localStorage.getItem(KEY)).toBeNull()
    expect(panel().getAttribute('style')).toBeNull()
  })

  it('没按把手时的 window mousemove 不产生位移', () => {
    render(<Harness options={{ fallback: { x: 669, y: 60 } }} />)
    fireEvent.mouseMove(window, { clientX: 10, clientY: 10 })
    expect(panel().style.left).toBe('669px')
    expect(panel().style.top).toBe('60px')
  })
})

describe('usePanelDrag 视口边界钳制', () => {
  it('拖过右/下边界 → 钳在视口内（jsdom 无布局，面板尺寸走 200×40 兜底）', () => {
    render(<Harness />)
    dragTo(9999, 9999, { x: 10, y: 10 })
    expect(panel().style.left).toBe(`${window.innerWidth - 200}px`)
    expect(panel().style.top).toBe(`${window.innerHeight - 40}px`)
  })

  it('拖过左/上边界 → 钳在 0', () => {
    render(<Harness />)
    dragTo(-500, -500, { x: 0, y: 0 })
    expect(panel().style.left).toBe('0px')
    expect(panel().style.top).toBe('0px')
  })
})

describe('usePanelDrag 窗口缩放', () => {
  it('clampOnResize → 窗口变小后已拖拽面板被钳回视口', () => {
    render(<Harness options={{ clampOnResize: true }} />)
    dragTo(700, 600, { x: 0, y: 0 })
    expect(panel().style.left).toBe('700px')

    withViewport(300, 200, () => fireEvent(window, new Event('resize')))
    expect(panel().style.left).toBe('100px') // min(700, 300-200)
    expect(panel().style.top).toBe('160px') // min(600, 200-40)
  })

  it('未开启 clampOnResize → 窗口变小后位置不动（DesktopToolbar 零变化语义）', () => {
    render(<Harness options={{ fallback: { x: 669, y: 60 } }} />)
    dragTo(700, 600, { x: 0, y: 0 })

    withViewport(300, 200, () => fireEvent(window, new Event('resize')))
    expect(panel().style.left).toBe('700px')
    expect(panel().style.top).toBe('600px')
  })
})

describe('usePanelDrag enabled 开关', () => {
  it('enabled=false → mousedown 空转、不落盘', () => {
    render(<Harness options={{ enabled: false }} />)
    dragTo(300, 200, { x: 50, y: 70 })
    fireEvent.mouseUp(window)
    expect(panel().getAttribute('style')).toBeNull()
    expect(localStorage.getItem(KEY)).toBeNull()
  })
})
