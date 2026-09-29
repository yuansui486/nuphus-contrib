import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import ExternalAgentsStatusBar, { EXT_AGENT_PINNED_EVENT } from './ExternalAgentsStatusBar'
import * as api from '../lib/api'
import type { ExternalAgentStatus } from '../lib/api'

/**
 * 回归钉：外部 Agent 列表栏（.ext-agents-hover-zone 所属胶囊）的移出语义。
 *
 * 2026-09-27 重构前的两处错误（本轮修复）：
 *  1. 「从列表栏移除」只写 React 内存态——前端任何重载即复活，用户的选择被静默遗忘；
 *  2. 「配置中心删除」只删 team.toml 段、不清 status.json——列表栏唯一边据是
 *     status.json，删了配置芯片照显，直到 app 重启才被 idle 规则遮掉。
 *
 * 新语义（本文件的断言基线）：
 *  - 移除 = 后端共享态（SignalState.hidden_ext_agents）：**应用生命周期内保持、
 *    重启即净**（前端 UI 显示跟随应用生命周期）；前端重载不丢（遵循用户选择）；
 *  - 撤销只有两条显式路径且均由后端判定：① 时刻更晚的新门铃活动（被再次调用）；
 *    ② 配置中心保存（用户主动纳入）；
 *  - 前端纯投影：隐显只看 listAgentStatuses 每项的 hidden 标注；
 *  - 列表栏只显示「被调用过的（非 idle）」或「本轮配置中心保存过的（pin）」agent。
 */

vi.mock('./PreviewOverlay', () => ({ PreviewOverlay: () => null }))

vi.mock('../lib/api', () => ({
  listAgentStatuses: vi.fn(),
  listAgentDeliverables: vi.fn(),
  listExternalAgents: vi.fn(),
  deleteAgentDeliverable: vi.fn(),
  notifyExtAgentRemoved: vi.fn().mockResolvedValue(undefined),
  extractAgentIcon: vi.fn().mockResolvedValue(null),
}))

const AVATAR_LABEL = '外部 Agent · opencode'
/** 固定在过去的时间戳：默认状态下移出后不应被 poll 误判为「有新活动」 */
const PAST = '2026-01-01T00:00:00+08:00'

function status(over: Partial<ExternalAgentStatus> = {}): ExternalAgentStatus {
  return { agent: 'opencode', state: 'done', task_id: '0916-01', updated_at: PAST, ...over }
}

function mountBar() {
  const onNotice = vi.fn()
  render(<ExternalAgentsStatusBar onNotice={onNotice} />)
  return onNotice
}

beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
  vi.mocked(api.listAgentStatuses).mockResolvedValue([status()])
  vi.mocked(api.listAgentDeliverables).mockResolvedValue([])
  vi.mocked(api.listExternalAgents).mockResolvedValue([])
})

afterEach(() => {
  cleanup()
})

describe('外部 Agent 列表栏 · 移出语义（后端共享态，应用生命周期）', () => {
  it('移除 = 通知后端 + 一句提示；下一轮 poll 带回 hidden 后头像真消失', async () => {
    const onNotice = mountBar()
    // 「被调用过」（state=done）的 agent 常驻显示
    fireEvent.click(await screen.findByRole('button', { name: AVATAR_LABEL }))

    fireEvent.click(await screen.findByRole('button', { name: '从列表栏移除' }))

    // 指令发给后端（写共享显示态 + prompt 提示）；用户面一句 HUD 反馈，不说实现细节
    expect(api.notifyExtAgentRemoved).toHaveBeenCalledWith('opencode')
    expect(onNotice).toHaveBeenCalledTimes(1)
    expect(onNotice.mock.calls[0][0]).toBe('已从列表栏移出「opencode」（重启前保持移出）')

    // 后端共享态落定 → 下一轮 poll hidden=true → 头像从 DOM 真移除
    vi.mocked(api.listAgentStatuses).mockResolvedValue([status({ hidden: true })])
    await waitFor(() => expect(screen.queryByRole('button', { name: AVATAR_LABEL })).toBeNull(), {
      timeout: 6000,
    })
  }, 10_000)

  it('后端 hidden=true 的头像不渲染（前端重载后移除依然有效）', async () => {
    // 模拟「用户已移出 + 前端重新挂载」：poll 第一批即带 hidden
    vi.mocked(api.listAgentStatuses).mockResolvedValue([status({ hidden: true })])
    mountBar()

    await waitFor(() => expect(api.listAgentStatuses).toHaveBeenCalled())
    expect(screen.queryByRole('button', { name: AVATAR_LABEL })).toBeNull()
  })

  it('被再次调用（后端撤销 hidden）→ 自动回到列表栏', async () => {
    vi.mocked(api.listAgentStatuses).mockResolvedValue([status({ hidden: true })])
    mountBar()
    await waitFor(() => expect(screen.queryByRole('button', { name: AVATAR_LABEL })).toBeNull(), {
      timeout: 6000,
    })

    // 时刻更晚的新门铃活动 → 后端撤销 hidden → poll 带回 hidden:false + 新状态
    vi.mocked(api.listAgentStatuses).mockResolvedValue([
      status({
        state: 'in_progress',
        updated_at: new Date(Date.now() + 3_600_000).toISOString(),
      }),
    ])

    await waitFor(() => expect(screen.getByRole('button', { name: AVATAR_LABEL })).toBeTruthy(), {
      timeout: 6000,
    })
  }, 10_000)

  it('历史遗留的持久化隐藏/pin 记录被清理，不再产生幽灵条目', async () => {
    localStorage.setItem('nuphus.extAgents.hiddenAgents', JSON.stringify(['opencode']))
    localStorage.setItem('nuphus.extAgents.pinned', JSON.stringify(['opencode']))
    mountBar()

    // 被调用的 agent 照常渲染（旧 hidden 记录不生效）
    expect(await screen.findByRole('button', { name: AVATAR_LABEL })).toBeTruthy()
    await waitFor(() => expect(localStorage.getItem('nuphus.extAgents.hiddenAgents')).toBeNull())
    expect(localStorage.getItem('nuphus.extAgents.pinned')).toBeNull()
  })

  it('空闲且未经配置中心保存的 agent 不占位；配置中心保存（pin）后立即显示', async () => {
    vi.mocked(api.listAgentStatuses).mockResolvedValue([status({ state: 'idle', task_id: '' })])
    mountBar()

    // idle = 本轮未经门铃验证的历史残留 → 不渲染（列表栏只显示被调用过的）
    await waitFor(() => expect(api.listExternalAgents).toHaveBeenCalled())
    expect(screen.queryByRole('button', { name: AVATAR_LABEL })).toBeNull()

    // 配置中心保存 → pin → 立即出现在列表栏（后端同步撤销 hidden）
    fireEvent(window, new CustomEvent(EXT_AGENT_PINNED_EVENT, { detail: 'opencode' }))
    expect(await screen.findByRole('button', { name: AVATAR_LABEL })).toBeTruthy()
  })
})
