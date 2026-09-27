import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { LangProvider } from '../../locales'
import ExternalAgentsStatusBar from './ExternalAgentsStatusBar'
import * as api from '../lib/api'
import type { ExternalAgentStatus } from '../lib/api'

vi.mock('../lib/api', () => ({
  listAgentStatuses: vi.fn(),
  listAgentDeliverables: vi.fn(),
  listExternalAgents: vi.fn(),
  deleteAgentDeliverable: vi.fn(),
  notifyExtAgentRemoved: vi.fn().mockResolvedValue(undefined),
  extractAgentIcon: vi.fn().mockResolvedValue(null),
  getLanguage: vi.fn().mockResolvedValue(null),
  readFile: vi.fn(),
  readFileBase64: vi.fn(),
  openPath: vi.fn().mockResolvedValue(undefined),
  revealPath: vi.fn().mockResolvedValue(undefined),
}))

vi.mock('pdfjs-dist', () => ({ GlobalWorkerOptions: {}, getDocument: vi.fn() }))

const REPORT_PATH = String.raw`C:\项目 资料\reports\验收 报告.md`
const LATEST_REPORT = '最近上报报告'

function status(lastEvent?: ExternalAgentStatus['last_event']): ExternalAgentStatus {
  return {
    agent: 'opencode',
    state: 'in_progress',
    task_id: 'new-task',
    updated_at: '2026-01-02T00:00:00Z',
    last_event: lastEvent,
  }
}

async function openAgent(lastEvent?: ExternalAgentStatus['last_event'], lang = 'zh') {
  vi.mocked(api.listAgentStatuses).mockResolvedValue([status(lastEvent)])
  localStorage.setItem('nuphus_language', lang)
  await act(async () => {
    render(
      <LangProvider>
        <ExternalAgentsStatusBar />
      </LangProvider>,
    )
  })
  await act(async () => {
    fireEvent.click(
      screen.getByRole('button', {
        name: lang === 'en' ? 'External agent → opencode' : '外部 Agent · opencode',
      }),
    )
  })
  return screen.getByRole('dialog', { name: 'opencode' })
}

beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
  vi.mocked(api.listAgentDeliverables).mockResolvedValue([])
  vi.mocked(api.listExternalAgents).mockResolvedValue([])
  vi.mocked(api.readFile).mockResolvedValue('# 验收完成\n\n全部检查通过。')
})

afterEach(() => {
  cleanup()
  localStorage.clear()
})

describe('外部 Agent 最近上报报告预览', () => {
  it('把旧事件的报告标为最近上报，使用原始中文空格路径打开真实预览并可关闭', async () => {
    const dialog = await openAgent({
      status: 'done',
      summary: '上一项任务完成',
      report_path: REPORT_PATH,
      ts: '2026-01-01T00:00:00Z',
    })

    expect(within(dialog).queryByText('暂无交付物')).not.toBeInTheDocument()
    expect(api.readFile).not.toHaveBeenCalled()
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: LATEST_REPORT }))
    })

    expect(api.readFile).toHaveBeenCalledWith(REPORT_PATH)
    expect(screen.getByRole('heading', { name: '验收完成' })).toBeInTheDocument()
    expect(screen.getByText('全部检查通过。')).toBeInTheDocument()
    expect(screen.getByTitle(REPORT_PATH)).toHaveTextContent('验收 报告.md')
    expect(api.openPath).not.toHaveBeenCalled()

    fireEvent.click(screen.getByTitle('关闭预览'))
    expect(screen.queryByRole('heading', { name: '验收完成' })).not.toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: LATEST_REPORT })).toBeInTheDocument()
  })

  it.each([
    undefined,
    null,
    {},
    { report_path: null },
    { report_path: '' },
    { report_path: ' \t ' },
  ])('没有有效报告路径时不显示入口：%j', async lastEvent => {
    const dialog = await openAgent(lastEvent)
    expect(within(dialog).queryByRole('button', { name: LATEST_REPORT })).not.toBeInTheDocument()
    expect(api.readFile).not.toHaveBeenCalled()
  })

  it('交付物列表仍在加载时也可预览最近上报报告', async () => {
    vi.mocked(api.listAgentDeliverables).mockReturnValue(new Promise(() => {}))
    const dialog = await openAgent({ report_path: REPORT_PATH })
    expect(within(dialog).getByText('加载中…')).toBeInTheDocument()

    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: LATEST_REPORT }))
    })
    expect(screen.getByRole('heading', { name: '验收完成' })).toBeInTheDocument()
  })

  it('缺失文件沿用预览错误反馈，并可退出预览', async () => {
    vi.mocked(api.readFile).mockRejectedValue('报告文件不存在')
    const dialog = await openAgent({ report_path: REPORT_PATH })
    await act(async () => {
      fireEvent.click(within(dialog).getByRole('button', { name: LATEST_REPORT }))
    })

    expect(screen.getByText('无法预览此文件')).toBeInTheDocument()
    expect(screen.getByText('报告文件不存在')).toBeInTheDocument()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(screen.queryByText('无法预览此文件')).not.toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: LATEST_REPORT })).toBeInTheDocument()
  })

  it('英文界面明确标为上次上报的报告', async () => {
    const dialog = await openAgent({ report_path: REPORT_PATH }, 'en')
    expect(within(dialog).getByRole('button', { name: 'Last reported report' })).toBeInTheDocument()
    expect(within(dialog).queryByText(LATEST_REPORT)).not.toBeInTheDocument()
  })
})
