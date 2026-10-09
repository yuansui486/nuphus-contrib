import { useEffect } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { AppUpdatesProvider, newerStatus, useAppUpdates, type UpdateStatus } from './Updates'
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listener: null as null | ((event: { payload: UpdateStatus }) => void),
  stop: vi.fn(),
  save: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: async () => '0.1.1' }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (_: string, cb: typeof mocks.listener) => {
    mocks.listener = cb
    return mocks.stop
  },
}))
vi.mock('../main-window/chat/MarkdownContent', () => ({
  MarkdownInline: ({ text }: { text: string }) => <span>{text}</span>,
}))
const status = (phase: UpdateStatus['phase'] = 'available', revision = 1): UpdateStatus => ({
  phase,
  revision,
  current_version: '0.1.1',
  version: '0.1.2',
  notes: '更新说明',
  automatic: true,
  last_check: 0,
  downloaded: 512,
  total: null,
  bytes_per_second: 0,
  eta_seconds: null,
  error: null,
})
function App() {
  const updates = useAppUpdates()
  useEffect(() => {
    updates.registerSave(mocks.save)
    return () => updates.registerSave(null)
  }, [updates.registerSave])
  return (
    <>
      <button onClick={updates.open}>版本与更新入口</button>
      <p>{updates.busy ? '已冻结编辑' : '可编辑'}</p>
    </>
  )
}
async function open(phase: UpdateStatus['phase'] = 'available') {
  mocks.invoke.mockResolvedValue(status(phase))
  render(
    <AppUpdatesProvider>
      <App />
    </AppUpdatesProvider>,
  )
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('get_app_update_status'))
  fireEvent.click(screen.getByText('版本与更新入口'))
  await screen.findByText('新版本 0.1.2')
}
beforeEach(() => {
  vi.clearAllMocks()
  mocks.save.mockResolvedValue(true)
  mocks.listener = null
})
describe('灵雀独立更新入口', () => {
  it('忽略迟到事件和命令响应；登录前也能显示更新', async () => {
    await open()
    act(() => mocks.listener?.({ payload: status('ready', 4) }))
    act(() => mocks.listener?.({ payload: status('downloading', 2) }))
    expect(screen.getByText('更新已下载并通过校验')).toBeTruthy()
    expect(newerStatus(status('ready', 4), status('failed', 3)).phase).toBe('ready')
    expect(mocks.invoke).not.toHaveBeenCalledWith('download_app_update', undefined)
  })
  it('下载必须确认，未知长度使用不定进度，允许后台和取消', async () => {
    await open()
    mocks.invoke.mockResolvedValue(status('downloading', 2))
    fireEvent.click(screen.getByText('下载更新'))
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('download_app_update', undefined))
    const progress = await screen.findByRole('progressbar')
    expect(progress.hasAttribute('value')).toBe(false)
    fireEvent.click(screen.getByText('取消下载'))
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('cancel_app_update', undefined))
    fireEvent.click(screen.getByText('后台下载'))
    expect(screen.queryByRole('dialog')).toBeNull()
    expect(mocks.invoke).not.toHaveBeenCalledWith('install_app_update')
  })
  it('保存失败不安装、不重启并恢复编辑', async () => {
    await open('ready')
    mocks.save.mockResolvedValue(false)
    fireEvent.click(screen.getByText('保存并重启安装'))
    await screen.findByRole('alert')
    expect(mocks.invoke).not.toHaveBeenCalledWith('install_app_update')
    expect(screen.getByText('可编辑')).toBeTruthy()
  })
  it('等待保存后安装，重复点击不会重复执行；原生拒绝后可重试', async () => {
    await open('ready')
    let finish!: (saved: boolean) => void
    mocks.save.mockImplementation(
      () =>
        new Promise<boolean>(resolve => {
          finish = resolve
        }),
    )
    mocks.invoke.mockRejectedValue('仍有任务运行，请稍后安装')
    const button = screen.getByText('保存并重启安装')
    fireEvent.click(button)
    fireEvent.click(button)
    expect(mocks.save).toHaveBeenCalledTimes(1)
    expect(screen.getByText('已冻结编辑')).toBeTruthy()
    expect(mocks.invoke).not.toHaveBeenCalledWith('install_app_update')
    await act(async () => finish(true))
    expect(await screen.findByText('仍有任务运行，请稍后安装')).toBeTruthy()
    expect(screen.getByText('可编辑')).toBeTruthy()
  })
  it('键盘焦点限制在对话框内，退出恢复焦点', async () => {
    await open()
    const dialog = screen.getByRole('dialog')
    expect(document.activeElement).toBe(dialog)
    fireEvent.keyDown(document, { key: 'Tab' })
    expect(document.activeElement).toBe(screen.getByText('关闭'))
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()
  })
})
