import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DesktopToolbar } from './DesktopToolbar'

const { invoke, showAppFeedback, listen, unlisten } = vi.hoisted(() => ({
  invoke: vi.fn(),
  showAppFeedback: vi.fn(),
  listen: vi.fn(),
  unlisten: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('../../ui/islandChannel', () => ({ showAppFeedback }))
// 热键通道走 core/bridge 的 listen（Tauri/WebSocket 双模式适配层）
vi.mock('../../core/bridge', () => ({
  invoke,
  listen: (...args: unknown[]) => listen(...args),
}))

describe('user-selected desktop application registration', () => {
  beforeEach(() => {
    invoke.mockReset()
    showAppFeedback.mockClear()
    listen.mockReset()
    unlisten.mockClear()
    listen.mockResolvedValue(unlisten)
    localStorage.clear()
  })
  afterEach(cleanup)

  it('opens the host picker without a model-provided path and reports the registered name', async () => {
    invoke.mockResolvedValue({ name: 'Portable Editor', app_ref: 'app:local' })
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '登记应用' }))
    // 登记成功走 island 轻提示（2026-10-11：信息类提示不再弹窗，免点关闭）
    await waitFor(() => expect(showAppFeedback).toHaveBeenCalled())
    expect(showAppFeedback).toHaveBeenCalledWith(
      expect.stringContaining('Portable Editor'),
      'success',
    )
    expect(invoke).toHaveBeenCalledExactlyOnceWith('desktop_register_application', undefined)
  })

  it('cancelling the native picker neither launches an application nor reports success', async () => {
    invoke.mockResolvedValue(null)
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '登记应用' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '登记应用' })).not.toBeDisabled())
    expect(showAppFeedback).not.toHaveBeenCalled()
    expect(invoke).toHaveBeenCalledTimes(1)
  })
})

/**
 * 大王 2026-10-09 定稿（两轮）：
 * ① 桌面工具条 = **右侧固定按钮列 + hover 从上到下逐次显形**，常驻渲染。
 *    Ctrl+U / visible / onClose 废弃；不可拖拽；**无置顶**；**无把手、无竖条包裹层**。
 * ③ 坐标读数 + 关闭是**常驻分组**（不受整列 hover 显隐约束），点关闭才收 ——
 *    钉死 2026-10-09 报障「点击后坐标位置获取不到」不得复发。
 * ② 反向钉死「被删掉的东西不得复活」——这不是「暂时没测」。
 */
describe('DesktopToolbar 常驻按钮列结构（废弃 Ctrl+U / 废弃拖拽 / 废弃置顶 / 废弃竖条）', () => {
  beforeEach(() => {
    invoke.mockReset()
    localStorage.clear()
  })
  afterEach(cleanup)

  const dock = () => document.querySelector('.desktop-toolbar-dock') as HTMLElement
  /** 按钮列 = dock 的直接子节点中带 .desktop-toolbar-btn 的那些（顺序 = 视觉从上到下）。
   *  不能按 tagName===BUTTON 取：坐标读数 .desktop-toolbar-pos 也是 BUTTON 且同为
   *  dock 的直接子节点，但它不是列成员且不吃错峰。 */
  const column = () =>
    Array.from(dock().children).filter(el =>
      el.classList.contains('desktop-toolbar-btn'),
    ) as HTMLElement[]

  it('无 props 即可渲染：按钮列直接挂在 dock 上，不再有把手 / 竖条两层', () => {
    render(<DesktopToolbar />)
    expect(dock()).toBeTruthy()
    expect(document.querySelector('.desktop-toolbar-handle')).toBeNull()
    expect(document.querySelector('.desktop-toolbar')).toBeNull()
    // 每一个按钮的父节点就是 dock 本身
    for (const btn of column()) expect(btn.parentElement).toBe(dock())
  })

  it('按钮列顺序：截图 → 选区 → 取色 → 鼠标 → 字典 → 登记应用（DOM 序 = 视觉从上到下）', () => {
    render(<DesktopToolbar />)
    expect(column().map(el => el.getAttribute('aria-label'))).toEqual([
      '截图',
      '选区',
      '取色',
      '鼠标',
      '字典',
      '登记应用',
    ])
  })

  it('传入 onOpenTools 时列末追加「内置工具」入口，点击即打开工具页', () => {
    const onOpenTools = vi.fn()
    render(<DesktopToolbar onOpenTools={onOpenTools} />)
    // 入口必须是列内最后一个按钮（大王要求：入口在工具条最后）
    expect(column().map(el => el.getAttribute('aria-label'))).toEqual([
      '截图',
      '选区',
      '取色',
      '鼠标',
      '字典',
      '登记应用',
      '内置工具',
    ])
    fireEvent.click(screen.getByRole('button', { name: '内置工具' }))
    expect(onOpenTools).toHaveBeenCalledOnce()
    // 打开整页工具界面不得触发任何遮罩抓屏
    expect(invoke).not.toHaveBeenCalledWith('start_overlay_mask', expect.anything())
  })

  it('未传 onOpenTools 时不渲染「内置工具」入口（保持无 props 可用的既有契约）', () => {
    render(<DesktopToolbar />)
    expect(screen.queryByRole('button', { name: '内置工具' })).toBeNull()
  })

  /**
   * 2026-10-17 大王定：浮层已足够承载提示，**不得再挂 title** ——
   * 原生灰框会和自定义浮层同时冒出来，同一个按钮两套提示。
   * 快捷键统一进浮层第二行。
   */
  it('列内按钮一律不挂 title；快捷键只出现在 hover 浮层里', () => {
    render(<DesktopToolbar onOpenTools={() => {}} />)
    for (const btn of column()) {
      expect(btn.getAttribute('title')).toBeNull()
      // 可访问名仍在（读屏不依赖浮层）
      expect(btn.getAttribute('aria-label')).toBeTruthy()
    }
    // 有快捷键的三个：浮层第二行必须是 Ctrl+N
    const hintOf = (name: string) =>
      screen.getByRole('button', { name }).querySelector('.desktop-toolbar-label-hint')?.textContent
    expect(hintOf('截图')).toBe('Ctrl+1')
    expect(hintOf('选区')).toBe('Ctrl+2')
    expect(hintOf('取色')).toBe('Ctrl+3')
    // 无快捷键的按钮：第二行给用途说明（不是空行）
    expect(hintOf('鼠标')).toBe('实时坐标')
    expect(hintOf('字典')).toBe('文字识别')
  })

  it('置顶已整体移除：无置顶按钮、不落盘 pinned 键', async () => {
    render(<DesktopToolbar />)
    expect(screen.queryByRole('button', { name: '固定窗口置顶' })).toBeNull()
    expect(screen.queryByRole('button', { name: '取消置顶' })).toBeNull()
    // 即使旧 localStorage 里还留着 pinned=true，也不应有任何置顶入口消费它
    localStorage.setItem('desktop_toolbar_pinned', 'true')
    fireEvent.click(screen.getByRole('button', { name: '截图' }))
    await waitFor(() => expect(invoke).toHaveBeenCalled())
    // 命令名已从后端整体删除（2026-10-09），这里改为「不得有任何置顶类窗口控制调用」
    for (const [cmd] of invoke.mock.calls) expect(String(cmd)).not.toMatch(/topmost/i)
    expect(localStorage.getItem('desktop_toolbar_pinned')).toBe('true')
  })

  it('分隔符已随置顶一并移除（列内不再有 divider）', () => {
    render(<DesktopToolbar />)
    expect(document.querySelector('.desktop-toolbar-divider')).toBeNull()
  })

  it('无 .panel-grip 拖拽把手（大王明令不可拖动）', () => {
    render(<DesktopToolbar />)
    expect(document.querySelector('.panel-grip')).toBeNull()
  })

  it('dock 无 inline 定位：left / top 均为空串（定位全部交给 CSS）', () => {
    render(<DesktopToolbar />)
    expect(dock().style.left).toBe('')
    expect(dock().style.top).toBe('')
  })

  it('旧存储键 desktop_toolbar_pos 被忽略（预置坐标也不产生 inline 定位）', () => {
    localStorage.setItem('desktop_toolbar_pos', JSON.stringify({ x: 333, y: 444 }))
    render(<DesktopToolbar />)
    expect(dock().style.left).toBe('')
    expect(dock().style.top).toBe('')
  })

  it('在按钮列上按下并拖动，既不改位置也不落盘', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseDown(dock(), { clientX: 50, clientY: 70 })
    fireEvent.mouseMove(window, { clientX: 900, clientY: 900 })
    fireEvent.mouseUp(window)
    expect(dock().style.left).toBe('')
    expect(dock().style.top).toBe('')
    expect(localStorage.getItem('desktop_toolbar_pos')).toBeNull()
  })

  it('全文档 [title] 不含 Ctrl+U（快捷键确已废弃）', () => {
    render(<DesktopToolbar />)
    for (const node of Array.from(document.querySelectorAll('[title]'))) {
      expect(node.getAttribute('title')).not.toMatch(/Ctrl\s*\+\s*U/i)
    }
  })

  it('卸载重挂后列结构不变（无 visible 开关导致的条件渲染）', () => {
    const first = render(<DesktopToolbar />)
    expect(column()).toHaveLength(6)
    first.unmount()
    render(<DesktopToolbar />)
    expect(column()).toHaveLength(6)
    expect(screen.getByRole('button', { name: '截图' })).toBeTruthy()
  })
})

describe('DesktopToolbar 显隐：边缘 hover 防抖 + 渐显后常驻 3s（无 CSS :hover 显隐）', () => {
  beforeEach(() => {
    invoke.mockReset()
    localStorage.clear()
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
    cleanup()
  })

  const dock = () => document.querySelector('.desktop-toolbar-dock') as HTMLElement
  const shown = () => dock().classList.contains('is-shown')
  /** 推进假定时器并把由此产生的 React 状态更新包进 act */
  const tick = (ms: number) => act(() => void vi.advanceTimersByTime(ms))

  it('初始未显形（渐显前不占视觉，但仍占布局盒）', () => {
    render(<DesktopToolbar />)
    expect(shown()).toBe(false)
  })

  it('进入即显形 —— 不做任何延迟（悬停必须跟手）', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseEnter(dock())
    // 不推进定时器：mouseEnter 同步生效
    expect(shown()).toBe(true)
  })

  it('快速划过也会显形（没有防抖窗口可以“滑过不触发”）', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseEnter(dock())
    fireEvent.mouseLeave(dock())
    expect(shown()).toBe(true)

    // 离开后仍按常驻规则：满 3s 才渐隐（不因“划过去”而提前消失）
    tick(2900)
    expect(shown()).toBe(true)
    tick(100)
    expect(shown()).toBe(false)
  })

  it('渐显后鼠标离开，仍常驻满 3s 才渐隐', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseEnter(dock())
    expect(shown()).toBe(true)

    fireEvent.mouseLeave(dock())
    // 2.9s 时仍常驻
    tick(2900)
    expect(shown()).toBe(true)
    // 到 3s 才渐隐
    tick(100)
    expect(shown()).toBe(false)
  })

  it('常驻期内再次进入 → 续期（重新计满 3s 才渐隐）', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseEnter(dock())
    expect(shown()).toBe(true)

    // 常驻已过 2s，此时再次进入 → 续期（此刻起重新计 3s）
    tick(2000)
    fireEvent.mouseEnter(dock())
    fireEvent.mouseLeave(dock())
    tick(2900)
    expect(shown()).toBe(true)
    tick(100)
    expect(shown()).toBe(false)
  })

  it('鼠标一直按在 dock 上：到点即自动续期，永不停留在渐隐态', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseEnter(dock())
    expect(shown()).toBe(true)

    // 不 mouseLeave：连续多个常驻周期都必须保持显形
    for (let i = 0; i < 5; i++) {
      tick(3000)
      expect(shown()).toBe(true)
    }
  })

  it('工具面板打开期间强制常驻：鼠标移开也不渐隐', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseEnter(dock())
    // 打开取色面板（activeTool 非空 → 钉住）
    fireEvent.click(screen.getByRole('button', { name: '取色' }))
    fireEvent.mouseLeave(dock())
    tick(10000)
    expect(shown()).toBe(true)
  })

  it('面板关闭后重新起 3s 收尾，不会永久驻留', () => {
    render(<DesktopToolbar />)
    fireEvent.mouseEnter(dock())
    // 打开取色面板：pinShown 会把常驻计时器清掉
    fireEvent.click(screen.getByRole('button', { name: '取色' }))
    fireEvent.mouseLeave(dock())
    tick(10000)
    expect(shown()).toBe(true)

    // 再点一次关闭面板 → 钉住解除，必须重新起计时（回归点：旧实现此处无定时器）
    fireEvent.click(screen.getByRole('button', { name: '取色' }))
    tick(2900)
    expect(shown()).toBe(true)
    tick(100)
    expect(shown()).toBe(false)
  })

  it('按钮本体不带 transition-delay / opacity —— 显隐全部交给 CSS 类', () => {
    render(<DesktopToolbar />)
    const column = Array.from(dock().children).filter(el =>
      el.classList.contains('desktop-toolbar-btn'),
    ) as HTMLElement[]
    for (const btn of column) {
      expect(btn.style.transitionDelay).toBe('')
      expect(btn.style.opacity).toBe('')
      // 逐个错峰已废止：不得再注入 --stagger-delay
      expect(btn.style.getPropertyValue('--stagger-delay')).toBe('')
    }
  })

  it('鼠标工具激活后：读数与关闭并排成一组，且都不占按钮列', () => {
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))

    const group = document.querySelector('.desktop-toolbar-pos-group') as HTMLElement
    expect(group).toBeTruthy()
    // 关**闭钮与读数同级**（并排），不再排在按钮列末尾、落在读数下方
    const readout = screen.getByTitle('点击复制坐标')
    const close = screen.getByRole('button', { name: '关闭鼠标坐标' })
    expect(readout.parentElement).toBe(group)
    expect(close.parentElement).toBe(group)
    // DOM 序：读数在前、关闭在后（视觉上关闭在坐标右侧）
    expect(Array.from(group.children).indexOf(readout)).toBeLessThan(
      Array.from(group.children).indexOf(close),
    )
    // 两者都不是列成员
    const column = Array.from(dock().children).filter(el =>
      el.classList.contains('desktop-toolbar-btn'),
    )
    expect(column).toHaveLength(6)
    expect(column.includes(close)).toBe(false)
  })

  it('点关闭后读数与关闭钮一起消失（常驻直到点关闭）', () => {
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))
    expect(document.querySelector('.desktop-toolbar-pos-group')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: '关闭鼠标坐标' }))
    expect(document.querySelector('.desktop-toolbar-pos-group')).toBeNull()
    expect(screen.queryByTitle('点击复制坐标')).toBeNull()
  })

  /**
   * 反向钉死「坐标读数为 (0,0) 但没人告诉用户为什么」这个 2026-10-17 报障形态：
   * 轮询被拒（automation_busy）时**必须**把原因摆到读数位上，不得静默停在初值。
   */
  it('坐标轮询被拒：读数位改显示「坐标不可用」，不再静默停在 (0, 0)', async () => {
    invoke.mockRejectedValue(new Error('automation_busy: 任务执行正在被占用，请稍后再试'))
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))

    // 第一次失败被容忍（防单次抖动），第二次才落错误态 —— 轮询 60ms
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200)
    })
    const errBtn = document.querySelector('.desktop-toolbar-pos--error') as HTMLElement
    expect(errBtn).toBeTruthy()
    // 初值的 (0, 0) 必须消失——那正是让人误判「点击无效」的东西
    expect(screen.queryByText('(0, 0)')).toBeNull()
    expect(errBtn.textContent).toBe('坐标不可用')
    expect(errBtn.getAttribute('title')).toContain('automation_busy')
    // 关闭钮仍在（用户能收尾）
    expect(screen.getByRole('button', { name: '关闭鼠标坐标' })).toBeTruthy()
  })

  it('关闭钮不带 inline 颜色（配色归 CSS 的 --error token，不写死十六进制）', () => {
    render(<DesktopToolbar />)
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))
    const close = screen.getByRole('button', { name: '关闭鼠标坐标' })
    expect(close.style.color).toBe('')
    expect(close.className).toContain('desktop-toolbar-pos-close')
    expect(document.body.innerHTML).not.toContain('#f87171')
    expect(document.body.innerHTML).not.toContain('#3b82f6')
  })

  it('图标尺寸统一 15（与 header 三个按钮的 IconSettings/IconPalette/IconBrowser 同规格）', () => {
    render(<DesktopToolbar />)
    const widths = () =>
      Array.from(document.querySelectorAll('.desktop-toolbar-dock svg')).map(svg =>
        svg.getAttribute('width'),
      )
    expect(widths()).toHaveLength(6)
    expect(new Set(widths())).toEqual(new Set(['15']))

    // mouse_pos 的「停止跟踪」按钮是条件渲染的，必须单独覆盖 —— 否则改回 14 无人发现
    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))
    const all = widths()
    expect(all).toHaveLength(7) // 6 工具图标 + 关闭钮 IconX（坐标读数是纯文本，无 svg）
    expect(new Set(all)).toEqual(new Set(['15']))
  })

  it('图标 stroke-width 一律取 lucide 默认 2（不显式传 strokeWidth，与 header 一致）', () => {
    render(<DesktopToolbar />)
    const strokes = () =>
      Array.from(document.querySelectorAll('.desktop-toolbar-dock svg')).map(svg =>
        svg.getAttribute('stroke-width'),
      )
    expect(strokes()).toHaveLength(6)
    expect(new Set(strokes())).toEqual(new Set(['2']))

    fireEvent.click(screen.getByRole('button', { name: '鼠标' }))
    expect(new Set(strokes())).toEqual(new Set(['2']))
  })
})

/**
 * 全局工具热键（Ctrl+1/2/3）契约：
 * 后端把热键解成工具名后派发 `desktop-tool-hotkey`，本组件必须走与点击完全相同的执行链
 * （`start_overlay_mask` + 轮询），不得另起一套。事件订阅只建一次，卸载时注销。
 */
describe('DesktopToolbar 全局工具热键（Ctrl+1/2/3）', () => {
  beforeEach(() => {
    invoke.mockReset()
    listen.mockReset()
    unlisten.mockClear()
    listen.mockResolvedValue(unlisten)
    localStorage.clear()
  })
  afterEach(cleanup)

  const hotkeyHandler = () => listen.mock.calls[0]?.[1] as (p: { tool?: string }) => void

  it('订阅 desktop-tool-hotkey，且卸载时注销', async () => {
    const view = render(<DesktopToolbar />)
    await waitFor(() => expect(listen).toHaveBeenCalled())
    expect(listen.mock.calls[0][0]).toBe('desktop-tool-hotkey')
    view.unmount()
    await waitFor(() => expect(unlisten).toHaveBeenCalled())
  })

  it.each([
    ['screenshot', 'screenshot'],
    ['picker', 'picker'],
  ])('热键 tool=%s → 走与点击相同的 start_overlay_mask(mode=%s)', async (tool, mode) => {
    render(<DesktopToolbar />)
    await waitFor(() => expect(listen).toHaveBeenCalled())
    hotkeyHandler()({ tool })
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('start_overlay_mask', { mode }))
  })

  it('热键 tool=color_picker → 与点击一致：打开取色面板（点色槽才开遮罩）', async () => {
    render(<DesktopToolbar />)
    await waitFor(() => expect(listen).toHaveBeenCalled())
    hotkeyHandler()({ tool: 'color_picker' })
    // 取色面板打开后才有「取色器」标题与 9 个槽位；此时不得直接抓屏
    await waitFor(() => expect(screen.getByText('取色器')).toBeInTheDocument())
    expect(invoke).not.toHaveBeenCalledWith('start_overlay_mask', expect.anything())
  })

  it('未知工具名不触发任何 overlay 调用（防脏事件）', async () => {
    render(<DesktopToolbar />)
    await waitFor(() => expect(listen).toHaveBeenCalled())
    hotkeyHandler()({ tool: 'not_a_tool' })
    hotkeyHandler()({})
    await new Promise(r => setTimeout(r, 0))
    expect(invoke).not.toHaveBeenCalledWith('start_overlay_mask', expect.anything())
  })
})
