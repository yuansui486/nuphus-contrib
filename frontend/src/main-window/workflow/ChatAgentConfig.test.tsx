import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('../lib/api', async importOriginal => ({
  ...(await importOriginal<typeof import('../lib/api')>()),
  listChatAgents: vi.fn(async () => []),
  getActiveChatAgent: vi.fn(async () => null),
  listWorkflows: vi.fn(),
  listChatAgentsInline: vi.fn(),
  updateChatAgentInline: vi.fn(),
  getCapabilities: vi.fn(async () => ({})),
}))

import * as api from '../lib/api'
import { ChatAgentConfig } from './ChatAgentConfig'

const entry = {
  workflow_id: 'wf-中文',
  workflow_name: '中文工作流',
  step_id: 'step-1',
  step_name: '回复步骤',
  config: { persona: '原角色', agent_id: 'template-1' },
}

const workflow = {
  id: entry.workflow_id,
  title: entry.workflow_name,
  steps: [],
  tags: [],
  created_at: 0,
  updated_at: 0,
  run_count: 0,
  status: 'draft' as const,
}

const inlineSection = () =>
  within(screen.getByRole('heading', { name: '工作流步骤' }).closest('section')!)

describe('inline Chat Agent configuration', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(api.listWorkflows).mockResolvedValue([workflow])
    vi.mocked(api.listChatAgentsInline).mockResolvedValue([structuredClone(entry)])
    vi.mocked(api.updateChatAgentInline).mockResolvedValue(null)
  })

  afterEach(cleanup)

  it('shows a load failure instead of claiming that no steps exist, and allows refresh', async () => {
    vi.mocked(api.listChatAgentsInline).mockRejectedValueOnce(new Error('读取失败'))
    render(<ChatAgentConfig onClose={vi.fn()} />)

    expect(await screen.findByRole('alert')).toHaveTextContent('中文工作流')
    expect(screen.getByRole('alert')).toHaveTextContent('读取失败')
    expect(screen.queryByText('暂无步骤配置')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '刷新' }))
    expect(await screen.findByText('回复步骤')).toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('keeps successfully loaded steps visible when another workflow fails', async () => {
    vi.mocked(api.listWorkflows).mockResolvedValue([
      workflow,
      { ...workflow, id: 'wf-broken', title: '损坏工作流' },
    ])
    vi.mocked(api.listChatAgentsInline)
      .mockResolvedValueOnce([structuredClone(entry)])
      .mockRejectedValueOnce(new Error('不可读取'))
    render(<ChatAgentConfig onClose={vi.fn()} />)

    expect(await screen.findByText('回复步骤')).toBeInTheDocument()
    expect(screen.getByRole('alert')).toHaveTextContent('损坏工作流')
    expect(screen.getByRole('alert')).toHaveTextContent('不可读取')
  })

  it('reports workflow enumeration failures', async () => {
    vi.mocked(api.listWorkflows).mockRejectedValueOnce(new Error('工作流列表不可用'))
    render(<ChatAgentConfig onClose={vi.fn()} />)

    expect(await screen.findByRole('alert')).toHaveTextContent('工作流列表不可用')
    expect(screen.queryByText('暂无步骤配置')).not.toBeInTheDocument()
  })

  it('still shows the empty state when loading succeeds with no inline steps', async () => {
    vi.mocked(api.listChatAgentsInline).mockResolvedValue([])
    render(<ChatAgentConfig onClose={vi.fn()} />)

    expect(await screen.findByText('暂无步骤配置')).toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('saves edited fields and reads the persisted configuration on refresh', async () => {
    render(<ChatAgentConfig onClose={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
    fireEvent.change(screen.getAllByRole('textbox')[0], { target: { value: '新角色' } })
    fireEvent.click(inlineSection().getByRole('button', { name: '保存' }))

    await waitFor(() =>
      expect(api.updateChatAgentInline).toHaveBeenCalledWith(
        entry.workflow_id,
        entry.step_id,
        expect.objectContaining({ persona: '新角色', agent_id: 'template-1' }),
      ),
    )
    expect(await screen.findByText('Persona: 新角色')).toBeInTheDocument()

    vi.mocked(api.listChatAgentsInline).mockResolvedValue([
      { ...entry, config: { ...entry.config, persona: '新角色' } },
    ])
    fireEvent.click(screen.getByRole('button', { name: '刷新' }))
    expect(await screen.findByText('Persona: 新角色')).toBeInTheDocument()
    expect(api.listChatAgentsInline).toHaveBeenCalledTimes(2)
  })

  it('preserves the draft and enables retry when saving fails', async () => {
    vi.mocked(api.updateChatAgentInline).mockRejectedValueOnce(new Error('磁盘写入失败'))
    render(<ChatAgentConfig onClose={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
    fireEvent.change(screen.getAllByRole('textbox')[0], { target: { value: '未保存的角色' } })
    fireEvent.click(inlineSection().getByRole('button', { name: '保存' }))

    expect(await screen.findByRole('alert')).toHaveTextContent('磁盘写入失败')
    expect(screen.getAllByRole('textbox')[0]).toHaveValue('未保存的角色')
    expect(inlineSection().getByRole('button', { name: '保存' })).toBeEnabled()

    fireEvent.click(inlineSection().getByRole('button', { name: '保存' }))
    expect(await screen.findByText('Persona: 未保存的角色')).toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })
})
