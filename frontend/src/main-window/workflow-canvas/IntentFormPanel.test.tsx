import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { IntentFormPanel } from './IntentFormPanel'
import { LangProvider } from '../../locales'

const listDataDirs = vi.hoisted(() =>
  vi.fn(async () => [{ key: 'plugin', path: 'test-workspace' }]),
)
vi.mock('../lib/api', () => ({
  listDataDirs,
  getLanguage: async () => '',
}))
beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
})
describe('intent form lifecycle', () => {
  it('isolates project scopes for the same workflow and restores them without native directory lookup', async () => {
    const props = {
      initialName: 'Workflow',
      workflowId: 'shared-id',
      onClose: vi.fn(),
      onSubmit: vi.fn(),
    }
    const view = render(<IntentFormPanel {...props} draftScope="workbench:one" />)
    await waitFor(() => expect(screen.getByPlaceholderText(/阶段名称/)).not.toBeDisabled())
    fireEvent.change(screen.getByPlaceholderText(/阶段名称/), {
      target: { value: 'First project' },
    })
    fireEvent.change(screen.getByPlaceholderText(/子步骤 1.1/), { target: { value: 'First step' } })
    view.rerender(<IntentFormPanel {...props} draftScope="workbench:two" />)
    await waitFor(() => expect(screen.getByPlaceholderText(/阶段名称/)).not.toBeDisabled())
    expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('')
    fireEvent.change(screen.getByPlaceholderText(/阶段名称/), {
      target: { value: 'Second project' },
    })
    view.rerender(<IntentFormPanel {...props} draftScope="workbench:one" />)
    await waitFor(() =>
      expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('First project'),
    )
    expect(screen.getByPlaceholderText(/子步骤 1.1/)).toHaveValue('First step')
    expect(listDataDirs).not.toHaveBeenCalled()
    view.rerender(<IntentFormPanel {...props} />)
    await waitFor(() => expect(screen.getByPlaceholderText(/阶段名称/)).not.toBeDisabled())
    expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('')
    expect(listDataDirs).toHaveBeenCalledTimes(1)
  })

  it('uses a caller label and submits only when the user confirms the filled form', async () => {
    const submit = vi.fn()
    render(
      <IntentFormPanel
        initialName="Workflow"
        workflowId="wf"
        draftScope="workbench:one"
        submitLabel="填入输入框"
        onClose={vi.fn()}
        onSubmit={submit}
      />,
    )
    await waitFor(() => expect(screen.getByPlaceholderText(/阶段名称/)).not.toBeDisabled())
    const button = screen.getByRole('button', { name: '填入输入框' })
    expect(button).toBeDisabled()
    fireEvent.change(screen.getByPlaceholderText(/阶段名称/), { target: { value: ' Stage ' } })
    fireEvent.change(screen.getByPlaceholderText(/子步骤 1.1/), { target: { value: ' Step ' } })
    expect(submit).not.toHaveBeenCalled()
    expect(button).toHaveAttribute('title', '填入输入框')
    fireEvent.click(button)
    await waitFor(() => expect(button).not.toBeDisabled())
    expect(submit).toHaveBeenCalledWith({
      workflowName: 'Workflow',
      stages: [
        {
          id: expect.any(String),
          name: 'Stage',
          steps: [{ id: expect.any(String), intent: 'Step' }],
        },
      ],
    })
  })

  it('restores on reopening, keeps rejected submissions, and clears only explicitly', async () => {
    const submit = vi.fn().mockResolvedValue(false)
    const props = { initialName: 'Workflow', workflowId: 'wf', onClose: vi.fn(), onSubmit: submit }
    const first = render(<IntentFormPanel {...props} />)
    await waitFor(() => expect(screen.getByPlaceholderText(/阶段名称/)).not.toBeDisabled())
    fireEvent.change(screen.getByPlaceholderText(/阶段名称/), { target: { value: 'Stage draft' } })
    fireEvent.change(screen.getByPlaceholderText(/子步骤 1.1/), { target: { value: 'Step draft' } })
    first.unmount()
    render(<IntentFormPanel {...props} />)
    await waitFor(() => expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('Stage draft'))
    expect(screen.getByPlaceholderText(/子步骤 1.1/)).toHaveValue('Step draft')
    fireEvent.click(screen.getByRole('button', { name: '发送给 WorkflowAgent 生成工作流' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('提交失败')
    expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('Stage draft')
    fireEvent.click(screen.getByRole('button', { name: '清空草稿' }))
    expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('Stage draft')
    fireEvent.click(screen.getByRole('button', { name: '确认清空' }))
    expect(screen.getByPlaceholderText(/阶段名称/)).toHaveValue('')
  })
  it('renders the complete intent form in English', async () => {
    localStorage.setItem('nuphus_language', 'en')
    render(
      <LangProvider>
        <IntentFormPanel
          initialName="My workflow"
          workflowId="en"
          onClose={() => {}}
          onSubmit={() => false}
        />
      </LangProvider>,
    )
    await waitFor(() => expect(screen.getByPlaceholderText(/Stage name/)).not.toBeDisabled())
    expect(screen.getByRole('dialog')).not.toHaveTextContent(/[\u4e00-\u9fff]/)
  })
})
