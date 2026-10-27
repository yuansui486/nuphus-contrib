import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LangProvider } from '../locales'
import { TenetsDialog } from '../main-window/dialogs/TenetsDialog'

const getTenets = vi.fn()
const getProjectBookmarks = vi.fn()
const getProjectDir = vi.fn()
const updateTenet = vi.fn()
const addTenet = vi.fn()
const deleteTenet = vi.fn()

vi.mock('../main-window/lib/api', () => ({
  getTenets: (...args: unknown[]) => getTenets(...args),
  getProjectBookmarks: (...args: unknown[]) => getProjectBookmarks(...args),
  getProjectDir: (...args: unknown[]) => getProjectDir(...args),
  updateTenet: (...args: unknown[]) => updateTenet(...args),
  addTenet: (...args: unknown[]) => addTenet(...args),
  deleteTenet: (...args: unknown[]) => deleteTenet(...args),
  getLanguage: async () => 'zh',
}))

const items = [
  {
    id: 'g',
    content: '通用规则',
    project_path: null,
    priority: 'high',
    enforce: 'warning',
    active: true,
    created_at: '',
  },
  {
    id: 'a',
    content: '项目规则',
    project_path: 'E:\\Proj\\A',
    priority: 'low',
    enforce: 'warning',
    active: true,
    created_at: '',
  },
]

function renderDialog() {
  return render(
    <LangProvider>
      <TenetsDialog onClose={vi.fn()} />
    </LangProvider>,
  )
}

describe('输入框原则弹窗：项目名 / 优先级 / 编辑', () => {
  beforeEach(() => {
    getTenets.mockReset().mockResolvedValue({ count: 2, total: 2, items })
    getProjectBookmarks.mockReset().mockResolvedValue([{ name: '甲项目', path: 'E:\\Proj\\A' }])
    getProjectDir.mockReset().mockResolvedValue({
      path: 'E:\\Proj\\A',
      name: '甲项目',
      tag: 'default',
    })
    updateTenet.mockReset().mockResolvedValue(true)
    addTenet.mockReset().mockResolvedValue(undefined)
    deleteTenet.mockReset().mockResolvedValue(true)
  })

  it('列出全部作用域，并在优先级前显示项目名', async () => {
    renderDialog()
    expect(await screen.findByText('通用规则')).toBeInTheDocument()
    expect(screen.getByText('项目规则')).toBeInTheDocument()
    expect(screen.getByText('通用原则')).toBeInTheDocument()
    expect(screen.getByText('甲项目')).toBeInTheDocument()

    const prioritySelects = screen.getAllByLabelText('优先级')
    expect(prioritySelects).toHaveLength(2)
    expect(prioritySelects[0]).toHaveValue('high')
    expect(prioritySelects[1]).toHaveValue('low')
  })

  it('编辑内容与优先级后调用 update_tenet，并保留项目归属', async () => {
    renderDialog()
    await screen.findByText('通用规则')

    fireEvent.click(screen.getAllByLabelText('编辑原则')[0])
    const textarea = screen.getByLabelText('原则内容 *')
    fireEvent.change(textarea, { target: { value: '通用规则改' } })

    fireEvent.change(screen.getAllByLabelText('优先级')[0], {
      target: { value: 'medium' },
    })
    expect(updateTenet).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => expect(updateTenet).toHaveBeenCalledWith('g', '通用规则改', 'medium'))
  })

  it('直接切换优先级立即保存当前内容', async () => {
    renderDialog()
    await screen.findByText('项目规则')

    fireEvent.change(screen.getAllByLabelText('优先级')[1], {
      target: { value: 'critical' },
    })
    await waitFor(() => expect(updateTenet).toHaveBeenCalledWith('a', '项目规则', 'critical'))
  })
})
