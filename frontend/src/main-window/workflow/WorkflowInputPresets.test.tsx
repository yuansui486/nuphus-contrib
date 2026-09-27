import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkflowInputSpec, WorkflowItem } from '../../core/types'
import { WorkflowRunModal } from './WorkflowRunModal'
import { WorkflowInputsDialog } from './WorkflowInputsForm'

function workflow(inputs: WorkflowInputSpec[], id = 'wf-中文'): WorkflowItem {
  return {
    id,
    title: '测试',
    inputs,
    steps: [],
    tags: [],
    created_at: 0,
    updated_at: 0,
    run_count: 0,
    status: 'draft',
  }
}

function modal(inputs: WorkflowInputSpec[], id = 'wf-中文', onRun = vi.fn()) {
  return <WorkflowRunModal open workflow={workflow(inputs, id)} onRun={onRun} onCancel={() => {}} />
}

function save(name: string) {
  fireEvent.change(screen.getByRole('textbox', { name: '方案名称' }), { target: { value: name } })
  fireEvent.click(screen.getByRole('button', { name: '保存方案' }))
}

function select(name: string) {
  fireEvent.change(screen.getByRole('combobox', { name: '参数方案' }), { target: { value: name } })
}

describe('workflow input presets shared by the list and canvas run dialogs', () => {
  beforeEach(() => localStorage.clear())
  afterEach(() => vi.restoreAllMocks())

  it('saves only on request and round-trips zero, false, JSON and Chinese paths across both entry points', () => {
    const specs: WorkflowInputSpec[] = [
      { name: 'count', type: 'number', default: 5 },
      { name: 'enabled', type: 'boolean', default: true },
      { name: 'payload', type: 'json' },
      { name: 'folder', type: 'path' },
      { name: 'secret', sensitive: true, required: true },
    ]
    const list = render(modal(specs))
    fireEvent.change(screen.getByLabelText(/count/), { target: { value: '0' } })
    fireEvent.click(screen.getByLabelText(/enabled/))
    fireEvent.change(screen.getByLabelText(/payload/), { target: { value: '{"客户":[1,false]}' } })
    fireEvent.change(screen.getByLabelText(/folder/), { target: { value: 'D:\\客户 甲\\报表' } })
    fireEvent.change(screen.getByLabelText(/secret/), { target: { value: 'private-test-value' } })
    expect(localStorage.length).toBe(0)
    save('正式数据')
    expect(screen.getByRole('status')).toHaveTextContent('参数方案已保存')
    expect(localStorage.length).toBe(1)
    expect(localStorage.getItem(localStorage.key(0)!)).not.toContain('private-test-value')
    expect(localStorage.getItem(localStorage.key(0)!)).not.toContain('secret')
    list.unmount()

    const onConfirm = vi.fn()
    render(
      <WorkflowInputsDialog
        open
        workflowId="wf-中文"
        resetToken="wf-中文"
        specs={specs}
        onConfirm={onConfirm}
        onCancel={() => {}}
      />,
    )
    expect(screen.getByLabelText(/count/)).toHaveValue(5)
    select('正式数据')
    expect(screen.getByLabelText(/count/)).toHaveValue(0)
    expect(screen.getByLabelText(/enabled/)).not.toBeChecked()
    expect(screen.getByLabelText(/secret/)).toHaveValue('')
    expect(screen.getByRole('button', { name: '启动' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText(/secret/), { target: { value: 'new-secret' } })
    fireEvent.click(screen.getByRole('button', { name: '启动' }))
    expect(onConfirm).toHaveBeenCalledWith({
      count: 0,
      enabled: false,
      payload: { 客户: [1, false] },
      folder: 'D:\\客户 甲\\报表',
      secret: 'new-secret',
    })
    expect(localStorage.getItem(localStorage.key(0)!)).not.toContain('new-secret')
  })

  it('isolates workflow IDs even when the same preset name is reused', () => {
    const specs: WorkflowInputSpec[] = [{ name: 'topic', default: 'one' }]
    const view = render(modal(specs, 'first'))
    save('常用')
    view.rerender(modal([{ name: 'topic', default: 'two' }], 'second'))
    expect(screen.queryByRole('option', { name: '常用' })).not.toBeInTheDocument()
    save('常用')
    view.rerender(modal(specs, 'first'))
    select('常用')
    expect(screen.getByLabelText(/topic/)).toHaveValue('one')
    expect(localStorage.length).toBe(2)
  })

  it('allows saving non-sensitive inputs before required secrets are entered', () => {
    render(
      modal([
        { name: 'folder', default: 'D:\\资料' },
        { name: 'token', required: true, sensitive: true },
      ]),
    )
    save('目录')
    expect(screen.getByRole('status')).toHaveTextContent('参数方案已保存')
    expect(screen.getByRole('button', { name: '启动' })).toBeDisabled()
  })

  it('reconciles removed, newly sensitive and changed-type fields with the latest defaults', () => {
    const first = render(
      modal([
        { name: 'removed', default: 'old' },
        { name: 'private', default: 'formerly-public' },
        { name: 'count', default: 'text' },
        { name: 'optional' },
      ]),
    )
    save('旧声明')
    first.unmount()
    const onRun = vi.fn()
    render(
      modal(
        [
          { name: 'private', sensitive: true },
          { name: 'count', type: 'number', default: 7 },
          { name: 'optional', required: true },
          { name: 'added', default: 'new' },
        ],
        'wf-中文',
        onRun,
      ),
    )
    select('旧声明')
    expect(screen.getByLabelText(/private/)).toHaveValue('')
    expect(screen.getByLabelText(/count/)).toHaveValue(7)
    expect(screen.getByRole('button', { name: '启动' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText(/optional/), { target: { value: 'filled' } })
    fireEvent.click(screen.getByRole('button', { name: '启动' }))
    expect(onRun).toHaveBeenCalledWith('wf-中文', { count: 7, optional: 'filled', added: 'new' })
    save('新声明')
    const stored = localStorage.getItem(localStorage.key(0)!)!
    expect(stored).not.toContain('formerly-public')
    expect(stored).not.toContain('removed')
  })

  it('deletes a preset persistently without changing the current inputs', () => {
    const specs = [{ name: 'topic', default: 'original' }]
    const view = render(modal(specs))
    save('删除测试')
    fireEvent.change(screen.getByLabelText(/topic/), { target: { value: 'edited' } })
    fireEvent.click(screen.getByRole('button', { name: '删除方案' }))
    expect(screen.getByLabelText(/topic/)).toHaveValue('edited')
    expect(localStorage.length).toBe(0)
    view.unmount()
    render(modal(specs))
    expect(screen.queryByRole('option', { name: '删除测试' })).not.toBeInTheDocument()
  })

  it('does not silently overwrite a duplicate name', () => {
    render(modal([{ name: 'topic', default: 'original' }]))
    save('常用')
    fireEvent.change(screen.getByLabelText(/topic/), { target: { value: 'changed' } })
    save(' 常用 ')
    expect(screen.getByRole('alert')).toHaveTextContent('方案名称已存在')
    select('常用')
    expect(screen.getByLabelText(/topic/)).toHaveValue('original')
  })

  it('does not save invalid non-sensitive inputs', () => {
    render(modal([{ name: 'payload', type: 'json', required: true }]))
    fireEvent.change(screen.getByLabelText(/payload/), { target: { value: '{broken' } })
    fireEvent.change(screen.getByRole('textbox', { name: '方案名称' }), {
      target: { value: '无效' },
    })
    expect(screen.getByRole('button', { name: '保存方案' })).toBeDisabled()
    expect(localStorage.length).toBe(0)
  })

  it('reports storage failures without falsely reporting success or blocking manual runs', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('Full', 'QuotaExceededError')
    })
    const onRun = vi.fn()
    render(modal([{ name: 'topic', default: 'manual' }], 'wf-中文', onRun))
    save('失败')
    expect(screen.getByRole('alert')).toHaveTextContent('无法保存参数方案的更改')
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    expect(screen.queryByRole('option', { name: '失败' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '启动' }))
    expect(onRun).toHaveBeenCalledWith('wf-中文', { topic: 'manual' })
  })

  it('handles corrupt stored data without breaking the run dialog', () => {
    localStorage.setItem('nuphus:workflow-input-presets:v1:wf-中文', '{invalid')
    render(modal([{ name: 'topic', default: 'manual' }]))
    expect(screen.getByRole('alert')).toHaveTextContent('无法读取已保存的参数方案')
    expect(screen.getByRole('button', { name: '启动' })).toBeEnabled()
  })

  it('has no preset controls for workflows with only sensitive inputs', () => {
    render(modal([{ name: 'secret', sensitive: true }]))
    expect(screen.queryByRole('combobox', { name: '参数方案' })).not.toBeInTheDocument()
    expect(localStorage.length).toBe(0)
  })
})
