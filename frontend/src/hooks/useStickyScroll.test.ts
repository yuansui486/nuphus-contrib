/**
 * useStickyScroll 单测 —— 消息流「贴底跟随」滚动语义
 *
 * 覆盖以下不变量（对应任务验收项）：
 *  ① followKey 变化且贴底 → smooth 滚底
 *  ② 用户上翻离开底部 → 按钮显示，且 followKey 再变化不再拽回（冻结）
 *  ③ 点击 jumpToBottom → 立即滚底 + 按钮隐藏
 *  ④ 程序滚动期间的 scroll 事件（向下 / 静止 delta）不触发冻结
 *  ⑤ 程序滚动期间用户向上 delta 被识别为用户操作（不被屏蔽窗吞掉）
 *  ⑥ 冻结后静默 15s 无用户操作 → 自动滚底 + 按钮隐藏；期间任何用户滚动都重置计时
 *  ⑦ 卸载后恢复计时被清理（不再触发滚底）
 *  ⑧ 滚动续命（主判定）：每 20s 向上滚一次 ×5，累计 100s 不恢复 —— 宽限是「连续无上滚
 *     操作」的上限而非读死表，用户在读就永不恢复（下滚不冻结不续命、回底才恢复，见 ⑩）
 *  ⑨ resumeMs 参数化：60s 宽限下 59s 不恢复、满 60s 才恢复并滚底（真静默）
 *  ⑩ 向下滚动不弹回：未回底的下滚不恢复也不续命，滚回 80px 容差内才恢复（用户可自由下滚）
 *  ⑪ executing=false（空闲读秒豁免）：上翻冻结且不排恢复计时，180s 不恢复不滚底；
 *     滚回底部 / followReset 两个出口仍生效
 *  ⑫ followReset：新轮次 / 完成补拉的 hook 侧语义 —— 立即恢复跟随 + 滚底 + 作废旧计时
 *  ⑬ enterPanel 进场宽限：3s 内的滚动（上下）一律不判定 —— 不冻结、不恢复、不续命；
 *     lastScrollTopRef 随事件持续刷新，满 3s 后判定恢复（上滚重新冻结 + 计时重排）
 *
 * jsdom 局限说明（盲区）：jsdom 不做布局，scrollHeight/clientHeight/scrollTop 均为
 * 注入值，smooth 动画的逐帧过程也只能用手动改 scrollTop + 调 onScroll 模拟；
 * 这里断言的是判定逻辑，不是真实滚动手感（真实布局由真机验收）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, renderHook } from '@testing-library/react'
import type { RefObject } from 'react'
import { useStickyScroll, type StickyScroll, type StickyScrollOptions } from './useStickyScroll'

/** 容器几何：距底 0px（scrollHeight 1000 - scrollTop 600 - clientHeight 400）视为贴底 */
const GEO = { scrollHeight: 1000, clientHeight: 400, scrollTop: 600 }

/** followKey 用消息数组的身份变化模拟（与 ChatPanel 传 messages 同构） */
type FollowKey = number[]

let scrollToSpy: ReturnType<typeof vi.fn>

beforeEach(() => {
  // jsdom 未实现 Element.prototype.scrollTo（与本仓其余 ChatPanel 测试同一处理）
  scrollToSpy = vi.fn()
  Object.defineProperty(window.Element.prototype, 'scrollTo', {
    value: scrollToSpy,
    writable: true,
    configurable: true,
  })
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

/** jsdom 无布局：给元素注入可读的滚动几何（scrollHeight/clientHeight 只读，scrollTop 可写） */
function makeScroller(): HTMLDivElement {
  const el = document.createElement('div')
  Object.defineProperty(el, 'scrollHeight', { value: GEO.scrollHeight, configurable: true })
  Object.defineProperty(el, 'clientHeight', { value: GEO.clientHeight, configurable: true })
  Object.defineProperty(el, 'scrollTop', {
    value: GEO.scrollTop,
    writable: true,
    configurable: true,
  })
  return el
}

/** renderHook 不渲染 DOM：hook 内部创建的 scrollRef 需手动挂到元素上 */
function attachScroller(scrollRef: RefObject<HTMLDivElement>): HTMLDivElement {
  const el = makeScroller()
  const mutable = scrollRef as { current: HTMLDivElement | null }
  mutable.current = el
  return el
}

/** 用户滚动：改 scrollTop 后触发 onScroll（rAF 里已排队的程序滚动不参与） */
function userScroll(result: { current: StickyScroll }, top: number): void {
  const el = result.current.scrollRef.current
  if (!el) throw new Error('scroller 未挂载')
  el.scrollTop = top
  act(() => {
    result.current.onScroll()
  })
}

function setup(opts?: StickyScrollOptions) {
  return renderHook((key: FollowKey) => useStickyScroll(key, opts), { initialProps: [1] })
}

/** 触发一次 followKey 变化并把 rAF 里的 scrollTo 跑出来 */
function changeFollowKey(rerender: (key: FollowKey) => void): void {
  act(() => {
    rerender([1, 2])
  })
  act(() => {
    vi.advanceTimersByTime(20) // rAF 约 16ms 一帧
  })
}

describe('useStickyScroll', () => {
  it('① followKey 变化且贴底 → smooth 滚底', () => {
    const { result, rerender } = setup()
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)

    expect(scrollToSpy).toHaveBeenCalledTimes(1)
    expect(scrollToSpy).toHaveBeenCalledWith({ top: GEO.scrollHeight, behavior: 'smooth' })
    expect(result.current.showJumpButton).toBe(false)
  })

  it('② 用户上翻离开底部 → 按钮显示，且 followKey 再变化不滚底（冻结跟随）', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500) // 等程序滚动屏蔽窗（400ms）过去
    })

    // 上翻到距底 400px（> 80px 容差）
    el.scrollTop = 200
    act(() => {
      result.current.onScroll()
    })
    expect(result.current.showJumpButton).toBe(true)

    // 冻结态：followKey 再变化（流式继续输出）也不拽回
    const callsBefore = scrollToSpy.mock.calls.length
    changeFollowKey(rerender)
    expect(scrollToSpy.mock.calls.length).toBe(callsBefore)
    expect(result.current.showJumpButton).toBe(true)
  })

  it('③ 点击 jumpToBottom → 立即滚底 + 按钮隐藏', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    el.scrollTop = 200
    act(() => {
      result.current.onScroll()
    })
    expect(result.current.showJumpButton).toBe(true)

    act(() => {
      result.current.jumpToBottom()
    })
    act(() => {
      vi.advanceTimersByTime(20)
    })

    expect(scrollToSpy).toHaveBeenCalledTimes(2)
    expect(scrollToSpy).toHaveBeenLastCalledWith({ top: GEO.scrollHeight, behavior: 'smooth' })
    expect(result.current.showJumpButton).toBe(false)
  })

  it('④ 程序滚动期间的 scroll 事件（向下 / 静止 delta）不触发冻结', () => {
    const { result, rerender } = setup()
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender) // 程序滚动进行中，屏蔽窗开启

    // smooth 动画中间态：向下推进 + 静止抖动，一律忽略
    userScroll(result, 800)
    userScroll(result, 1000)
    userScroll(result, 1001) // |delta| = 1px，静止容差内
    userScroll(result, 999)

    expect(result.current.showJumpButton).toBe(false)
    expect(scrollToSpy).toHaveBeenCalledTimes(1) // 仍只有程序自己发起的那一次
  })

  it('⑤ 程序滚动期间用户向上 delta 被识别为用户操作（不被屏蔽窗吞掉）', () => {
    const { result, rerender } = setup()
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender) // 屏蔽窗开启

    // 用户在 smooth 途中抢滚动条 / 反向滚轮：向上 300px
    userScroll(result, 300)

    expect(result.current.showJumpButton).toBe(true)

    // 屏蔽窗已随用户操作关闭：滚回底部（容差内）应立即恢复、隐藏按钮
    userScroll(result, GEO.scrollTop)
    expect(result.current.showJumpButton).toBe(false)
  })

  it('⑥ 冻结后静默 15s 无用户操作 → 自动滚底 + 按钮隐藏', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    el.scrollTop = 200
    act(() => {
      result.current.onScroll()
    })
    expect(result.current.showJumpButton).toBe(true)

    act(() => {
      vi.advanceTimersByTime(14_000)
    })
    expect(scrollToSpy).toHaveBeenCalledTimes(1) // 未满 15s 不恢复
    expect(result.current.showJumpButton).toBe(true)

    act(() => {
      vi.advanceTimersByTime(1_500)
    })
    act(() => {
      vi.advanceTimersByTime(20) // 恢复滚底的 rAF
    })

    expect(scrollToSpy).toHaveBeenCalledTimes(2)
    expect(scrollToSpy).toHaveBeenLastCalledWith({ top: GEO.scrollHeight, behavior: 'smooth' })
    expect(result.current.showJumpButton).toBe(false)
  })

  it('⑥b 15s 内任何用户滚动都重置计时（10s + 滚动 + 10s 不恢复，满 15s 才恢复）', () => {
    const { result, rerender } = setup()
    const el = attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    el.scrollTop = 200
    act(() => {
      result.current.onScroll()
    })

    act(() => {
      vi.advanceTimersByTime(10_000)
    })
    expect(result.current.showJumpButton).toBe(true)

    // 用户又滚动了一次（仍未回底）：计时重置
    userScroll(result, 150)

    act(() => {
      vi.advanceTimersByTime(10_000)
    })
    expect(result.current.showJumpButton).toBe(true) // 距上次用户操作仅 10s，不恢复
    expect(scrollToSpy).toHaveBeenCalledTimes(1)

    act(() => {
      vi.advanceTimersByTime(5_000)
    })
    act(() => {
      vi.advanceTimersByTime(20)
    })

    expect(scrollToSpy).toHaveBeenCalledTimes(2)
    expect(result.current.showJumpButton).toBe(false)
  })

  it('⑦ 卸载后恢复计时被清理（不再触发滚底）', () => {
    const { result, rerender, unmount } = setup()
    const el = attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    el.scrollTop = 200
    act(() => {
      result.current.onScroll()
    })

    unmount()

    act(() => {
      vi.advanceTimersByTime(20_000)
    })
    expect(scrollToSpy).toHaveBeenCalledTimes(1)
  })

  it('⑧ 滚动续命（主判定）：冻结态每 20s 向上滚一次 ×5，累计 100s 不恢复', () => {
    const { result, rerender } = setup({ resumeMs: 60_000, executing: true })
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500) // 等程序滚动屏蔽窗过去
    })
    userScroll(result, 200)
    expect(result.current.showJumpButton).toBe(true)

    // 用户在持续向上滚动（读历史）：每滚一次宽限计时清零，永远攒不满 60s 连续静默
    // ——「时间是上滚操作的上限，不是读死表」（下滚不续命，见 ⑩）
    for (let i = 0; i < 5; i++) {
      act(() => {
        vi.advanceTimersByTime(20_000)
      })
      expect(result.current.showJumpButton).toBe(true) // 距上次滚动仅 20s，不恢复
      userScroll(result, 200 - (i + 1) * 10)
    }

    // 全程只有初始那一次程序滚底：没有任何一次被宽限逻辑拽回底部
    expect(scrollToSpy).toHaveBeenCalledTimes(1)
    expect(result.current.showJumpButton).toBe(true)
  })

  it('⑨ resumeMs 参数化：60s 宽限下 59s 不恢复，满 60s（真静默）才恢复并滚底', () => {
    const { result, rerender } = setup({ resumeMs: 60_000, executing: true })
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    userScroll(result, 200)

    act(() => {
      vi.advanceTimersByTime(59_000)
    })
    expect(result.current.showJumpButton).toBe(true) // 未满 60s 不恢复
    expect(scrollToSpy).toHaveBeenCalledTimes(1)

    act(() => {
      vi.advanceTimersByTime(1_500) // 连续静默满 60s
    })
    act(() => {
      vi.advanceTimersByTime(20) // 恢复滚底的 rAF
    })
    expect(scrollToSpy).toHaveBeenCalledTimes(2)
    expect(scrollToSpy).toHaveBeenLastCalledWith({ top: GEO.scrollHeight, behavior: 'smooth' })
    expect(result.current.showJumpButton).toBe(false)
  })

  it('⑩ 向下滚动不弹回：未回底的下滚不恢复也不续命，滚回 80px 容差内才恢复', () => {
    const { result, rerender } = setup({ resumeMs: 60_000, executing: true })
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    userScroll(result, 300) // 上翻离开底部（距底 300px）→ 冻结
    expect(result.current.showJumpButton).toBe(true)

    act(() => {
      vi.advanceTimersByTime(20_000)
    })
    expect(result.current.showJumpButton).toBe(true) // 未满 60s 宽限

    // 向下滚 100px（距底 200px，仍在 80px 容差之外）：用户在往回走 —— 不弹回底部
    // （不恢复、不主动滚底），也不按读历史续命（不重置宽限计时）
    userScroll(result, 400)
    expect(result.current.showJumpButton).toBe(true)
    expect(scrollToSpy).toHaveBeenCalledTimes(1)

    // 不续命实证：计时从上次上翻起算、与这次下滚无关 —— 再静默 40s 即满 60s 恢复
    act(() => {
      vi.advanceTimersByTime(40_000)
    })
    act(() => {
      vi.advanceTimersByTime(20) // 恢复滚底的 rAF
    })
    expect(scrollToSpy).toHaveBeenCalledTimes(2)
    expect(result.current.showJumpButton).toBe(false)

    // 恢复后重新上翻冻结，再下滚进 80px 容差（距底 70px）—— 拉到底部附近才恢复
    act(() => {
      vi.advanceTimersByTime(500) // 等恢复滚底的程序滚动屏蔽窗过去
    })
    userScroll(result, 200) // 上翻重新冻结
    expect(result.current.showJumpButton).toBe(true)
    userScroll(result, 530) // 下滚 330px，距底 70px < 80px 容差
    expect(result.current.showJumpButton).toBe(false)
    expect(scrollToSpy).toHaveBeenCalledTimes(2) // 恢复本身不主动滚底（与宽限恢复不同）
  })

  it('⑬ enterPanel 进场宽限：3s 内的滚动（上下）一律不判定，满 3s 恢复判定', () => {
    const { result, rerender } = setup({ resumeMs: 60_000, executing: true })
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    userScroll(result, 200) // 先冻结（60s 宽限计时在跑）
    expect(result.current.showJumpButton).toBe(true)

    // 进场：enterPanel（缺省 3000ms）—— followReset 恢复 + 滚底，随后滚动一律不判定
    act(() => {
      result.current.enterPanel()
    })
    act(() => {
      vi.advanceTimersByTime(20) // followReset 的 rAF
    })
    expect(result.current.showJumpButton).toBe(false)
    expect(scrollToSpy).toHaveBeenCalledTimes(2) // 首次程序滚底 + followReset 补拉

    // 宽限内上滚：本会冻结 —— 不判定（进场惯性 / 渲染抖动豁免）
    userScroll(result, 150)
    expect(result.current.showJumpButton).toBe(false)

    // 宽限内下滚：方向书签持续刷新（满 3s 后的首个事件据此算增量，不误判）
    userScroll(result, 250)
    expect(result.current.showJumpButton).toBe(false)

    // 未满 3s：判定仍未恢复
    act(() => {
      vi.advanceTimersByTime(2_900)
    })
    userScroll(result, 240) // 上滚仍不判定
    expect(result.current.showJumpButton).toBe(false)

    // 满 3s：判定恢复 —— 上滚立即重新冻结，60s 宽限计时重新起排
    act(() => {
      vi.advanceTimersByTime(200)
    })
    userScroll(result, 230)
    expect(result.current.showJumpButton).toBe(true)

    act(() => {
      vi.advanceTimersByTime(59_000)
    })
    expect(result.current.showJumpButton).toBe(true) // 未满 60s 不恢复

    act(() => {
      vi.advanceTimersByTime(1_500) // 连续无上滚满 60s
    })
    act(() => {
      vi.advanceTimersByTime(20) // 恢复滚底的 rAF
    })
    expect(scrollToSpy).toHaveBeenCalledTimes(3)
    expect(result.current.showJumpButton).toBe(false)
  })

  it('⑪ executing=false：上翻冻结且不排恢复计时，180s 不恢复不滚底（空闲翻看不打扰）', () => {
    const { result, rerender } = setup({ resumeMs: 60_000, executing: false })
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    userScroll(result, 200)
    expect(result.current.showJumpButton).toBe(true)

    // 空闲期：不排恢复计时，180s 也不恢复、不滚底
    act(() => {
      vi.advanceTimersByTime(180_000)
    })
    expect(result.current.showJumpButton).toBe(true)
    expect(scrollToSpy).toHaveBeenCalledTimes(1)

    // 出口一：用户自己滚回底部（容差内）→ 立即恢复，无需再滚底
    userScroll(result, GEO.scrollTop)
    expect(result.current.showJumpButton).toBe(false)

    // 出口二：再上翻冻结 → followReset（程序侧）→ 立即恢复并滚底
    userScroll(result, 200)
    expect(result.current.showJumpButton).toBe(true)
    act(() => {
      result.current.followReset()
    })
    act(() => {
      vi.advanceTimersByTime(20)
    })
    expect(result.current.showJumpButton).toBe(false)
    expect(scrollToSpy).toHaveBeenCalledTimes(2)
  })

  it('⑫ followReset：立即恢复跟随 + 滚底 + 作废旧宽限计时（新轮次 / 完成补拉的 hook 侧语义）', () => {
    const { result, rerender } = setup({ resumeMs: 60_000, executing: true })
    attachScroller(result.current.scrollRef)

    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(500)
    })
    // 用户停在上一轮的上翻位置，冻结中（60s 宽限计时在跑）
    userScroll(result, 200)
    expect(result.current.showJumpButton).toBe(true)

    // 新轮次 execution_started → followReset：立即恢复 + 滚底
    act(() => {
      result.current.followReset()
    })
    act(() => {
      vi.advanceTimersByTime(20)
    })
    expect(result.current.showJumpButton).toBe(false)
    expect(scrollToSpy).toHaveBeenCalledTimes(2)

    // 恢复后 followKey 变化继续跟随滚底（后续流式不被拦）
    const callsAfterReset = scrollToSpy.mock.calls.length
    changeFollowKey(rerender)
    act(() => {
      vi.advanceTimersByTime(20)
    })
    expect(scrollToSpy.mock.calls.length).toBe(callsAfterReset + 1)

    // 旧宽限计时已随 followReset 作废：再静默 120s 不会有第二次「恢复滚底」
    act(() => {
      vi.advanceTimersByTime(120_000)
    })
    act(() => {
      vi.advanceTimersByTime(20)
    })
    expect(scrollToSpy).toHaveBeenCalledTimes(3)
  })
})
