import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DesktopToolbar } from './DesktopToolbar'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

describe('user-selected desktop application registration', () => {
  beforeEach(() => {
    invoke.mockReset()
    localStorage.clear()
  })
  afterEach(cleanup)

  it('opens the host picker without a model-provided path and reports the registered name', async () => {
    invoke.mockResolvedValue({ name: 'Portable Editor', app_ref: 'app:local' })
    render(<DesktopToolbar visible onClose={() => {}} />)
    fireEvent.click(screen.getByRole('button', { name: '登记应用' }))
    expect(await screen.findByText('应用已登记')).toBeTruthy()
    expect(screen.getByText(/Portable Editor/)).toBeTruthy()
    expect(invoke).toHaveBeenCalledExactlyOnceWith('desktop_register_application', undefined)
  })

  it('cancelling the native picker neither launches an application nor reports success', async () => {
    invoke.mockResolvedValue(null)
    render(<DesktopToolbar visible onClose={() => {}} />)
    fireEvent.click(screen.getByRole('button', { name: '登记应用' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '登记应用' })).not.toBeDisabled())
    expect(screen.queryByText('应用已登记')).toBeNull()
    expect(invoke).toHaveBeenCalledTimes(1)
  })
})

describe('DesktopToolbar 应用内拖拽（抽取回归：行为零变化）', () => {
  const TOOLBAR_KEY = 'desktop_toolbar_pos'

  beforeEach(() => {
    invoke.mockReset()
    localStorage.clear()
  })
  afterEach(cleanup)

  const toolbar = () => document.querySelector('.desktop-toolbar') as HTMLElement
  const grip = () => toolbar().querySelector('.panel-grip') as HTMLElement

  it('无存储 → 默认站位 {x:669,y:60}（inline left/top 逐字不变）', () => {
    render(<DesktopToolbar visible onClose={() => {}} />)
    expect(toolbar().style.left).toBe('669px')
    expect(toolbar().style.top).toBe('60px')
  })

  it('把手是共享类 panel-grip、位于工具条首位、title 拖拽移动', () => {
    render(<DesktopToolbar visible onClose={() => {}} />)
    expect(toolbar().firstElementChild).toBe(grip())
    expect(grip().getAttribute('title')).toBe('拖拽移动')
  })

  it('拖拽跟手 + mouseup 落盘 desktop_toolbar_pos', () => {
    render(<DesktopToolbar visible onClose={() => {}} />)
    fireEvent.mouseDown(grip(), { clientX: 50, clientY: 70 })
    fireEvent.mouseMove(window, { clientX: 300, clientY: 200 })
    fireEvent.mouseUp(window)
    expect(toolbar().style.left).toBe('250px')
    expect(toolbar().style.top).toBe('130px')
    expect(localStorage.getItem(TOOLBAR_KEY)).toBe(JSON.stringify({ x: 250, y: 130 }))
  })

  it('刷新（重挂载）后从 localStorage 恢复', () => {
    localStorage.setItem(TOOLBAR_KEY, JSON.stringify({ x: 333, y: 444 }))
    render(<DesktopToolbar visible onClose={() => {}} />)
    expect(toolbar().style.left).toBe('333px')
    expect(toolbar().style.top).toBe('444px')
  })

  it('边界钳制：拖不出视口', () => {
    render(<DesktopToolbar visible onClose={() => {}} />)
    fireEvent.mouseDown(grip(), { clientX: 0, clientY: 0 })
    fireEvent.mouseMove(window, { clientX: 9999, clientY: 9999 })
    expect(toolbar().style.left).toBe(`${window.innerWidth - 200}px`)
    expect(toolbar().style.top).toBe(`${window.innerHeight - 40}px`)
  })

  it('零变化钉死：窗口变小后位置不自动钳制（未开启 clampOnResize）', () => {
    render(<DesktopToolbar visible onClose={() => {}} />)
    fireEvent.mouseDown(grip(), { clientX: 0, clientY: 0 })
    fireEvent.mouseMove(window, { clientX: 700, clientY: 600 })

    const wDesc = Object.getOwnPropertyDescriptor(window, 'innerWidth')!
    const hDesc = Object.getOwnPropertyDescriptor(window, 'innerHeight')!
    try {
      Object.defineProperty(window, 'innerWidth', {
        value: 300,
        configurable: true,
        writable: true,
      })
      Object.defineProperty(window, 'innerHeight', {
        value: 200,
        configurable: true,
        writable: true,
      })
      fireEvent(window, new Event('resize'))
    } finally {
      Object.defineProperty(window, 'innerWidth', wDesc)
      Object.defineProperty(window, 'innerHeight', hDesc)
    }
    expect(toolbar().style.left).toBe('700px')
    expect(toolbar().style.top).toBe('600px')
  })
})
