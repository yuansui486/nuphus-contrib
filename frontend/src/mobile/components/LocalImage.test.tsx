import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import LocalImage from './LocalImage'
import { fetchFileBlob } from '../api'

vi.mock('../api', () => ({ fetchFileBlob: vi.fn() }))

const fetchMock = vi.mocked(fetchFileBlob)
const WINDOWS_PATH = String.raw`E:\NUS\_settings_popup_wide.png`

/** jsdom 不实现 IntersectionObserver：手动触发「进入视口」以验证懒加载 */
class FakeIntersectionObserver {
  static instances: FakeIntersectionObserver[] = []
  callback: IntersectionObserverCallback
  observed: Element[] = []
  disconnected = 0

  constructor(callback: IntersectionObserverCallback) {
    this.callback = callback
    FakeIntersectionObserver.instances.push(this)
  }
  observe(el: Element): void {
    this.observed.push(el)
  }
  unobserve(): void {
    /* 本组件只使用单元素观察，无需实现 */
  }
  disconnect(): void {
    this.disconnected += 1
  }
  takeRecords(): IntersectionObserverEntry[] {
    return []
  }
  /** 模拟视口交叉回调 */
  trigger(isIntersecting: boolean): void {
    this.callback(
      [{ isIntersecting } as IntersectionObserverEntry],
      this as unknown as IntersectionObserver,
    )
  }
}

let createObjectURL: ReturnType<typeof vi.fn>
let revokeObjectURL: ReturnType<typeof vi.fn>

beforeEach(() => {
  fetchMock.mockReset()
  localStorage.removeItem('nuphus_language')
  FakeIntersectionObserver.instances = []
  createObjectURL = vi.fn(() => 'blob:nuphus-image')
  revokeObjectURL = vi.fn()
  // jsdom 未实现 objectURL API，按浏览器语义打桩
  URL.createObjectURL = createObjectURL as unknown as typeof URL.createObjectURL
  URL.revokeObjectURL = revokeObjectURL as unknown as typeof URL.revokeObjectURL
  // 默认无 IntersectionObserver（与旧 WebView 一致 → 挂载即拉取）；懒加载用例自行注入
  ;(globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = undefined
})

describe('LocalImage', () => {
  it('拉取成功渲染缩略图，点击放大，卸载后回收 objectURL', async () => {
    fetchMock.mockResolvedValue(new Blob([new Uint8Array([1, 2, 3])], { type: 'image/png' }))
    const { unmount } = render(<LocalImage path={WINDOWS_PATH} />)

    const img = await screen.findByRole('img')
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(fetchMock).toHaveBeenCalledWith(WINDOWS_PATH)
    expect(createObjectURL).toHaveBeenCalledTimes(1)
    expect(img).toHaveAttribute('src', 'blob:nuphus-image')
    // 复用既有消息图片样式（圆角 + 限宽 + zoom-in）
    expect(img).toHaveClass('mobile-msg-image')

    fireEvent.click(img)
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    fireEvent.click(screen.getByLabelText('关闭预览'))
    expect(screen.queryByRole('dialog')).toBeNull()

    // 仍在使用中：不提前回收
    expect(revokeObjectURL).not.toHaveBeenCalled()
    unmount()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:nuphus-image'))
  })

  it('拉取失败降级为原始路径文本，且失败不驻留缓存（重挂载可重试）', async () => {
    fetchMock.mockRejectedValue(new Error('file failed: 404'))
    const first = render(<LocalImage path={WINDOWS_PATH} />)

    const fallback = await screen.findByText(WINDOWS_PATH)
    expect(fallback).toHaveClass('m-local-image-error')
    expect(screen.queryByRole('img')).toBeNull()
    first.unmount()

    fetchMock.mockResolvedValue(new Blob([new Uint8Array([1])], { type: 'image/png' }))
    render(<LocalImage path={WINDOWS_PATH} />)
    expect(await screen.findByRole('img')).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalledTimes(2)
  })

  it('下载成功但缩略图解码失败时降级为原始路径，卸载后回收 objectURL', async () => {
    const path = String.raw`C:\截图\中文目录\损坏图片.png`
    fetchMock.mockResolvedValue(new Blob(['invalid PNG bytes'], { type: 'image/png' }))
    const { unmount } = render(<LocalImage path={path} />)

    const img = await screen.findByRole('img')
    expect(fetchMock).toHaveBeenCalledWith(path)
    expect(img).toHaveAttribute('src', 'blob:nuphus-image')
    fireEvent.error(img)

    expect(screen.getByText(path)).toHaveClass('m-local-image-error')
    expect(screen.queryByRole('img')).toBeNull()
    expect(screen.queryByRole('dialog')).toBeNull()

    unmount()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:nuphus-image'))
    expect(revokeObjectURL).toHaveBeenCalledTimes(1)
  })

  it('预览图解码失败时关闭预览并降级，切换路径后恢复缩略图', async () => {
    const nextPath = String.raw`C:\截图\新图片.png`
    createObjectURL
      .mockReturnValueOnce('blob:nuphus-broken')
      .mockReturnValueOnce('blob:nuphus-recovered')
    fetchMock.mockResolvedValue(new Blob([new Uint8Array([1])], { type: 'image/png' }))
    const { rerender, unmount } = render(<LocalImage path={WINDOWS_PATH} />)

    fireEvent.click(await screen.findByRole('img'))
    const preview = within(screen.getByRole('dialog')).getByRole('img')
    fireEvent.error(preview)

    expect(screen.getByText(WINDOWS_PATH)).toHaveClass('m-local-image-error')
    expect(screen.queryByRole('img')).toBeNull()
    expect(screen.queryByRole('dialog')).toBeNull()

    rerender(<LocalImage path={nextPath} />)

    const recovered = await screen.findByRole('img', { name: nextPath })
    expect(recovered).toHaveAttribute('src', 'blob:nuphus-recovered')
    expect(fetchMock).toHaveBeenCalledWith(nextPath)
    expect(screen.queryByRole('dialog')).toBeNull()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:nuphus-broken'))
    expect(revokeObjectURL).not.toHaveBeenCalledWith('blob:nuphus-recovered')

    unmount()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:nuphus-recovered'))
    expect(revokeObjectURL).toHaveBeenCalledTimes(2)
  })

  it('同一路径多处渲染只下载一次，最后一个使用者卸载才回收', async () => {
    fetchMock.mockResolvedValue(new Blob([new Uint8Array([1])], { type: 'image/png' }))
    const { unmount } = render(
      <div>
        <LocalImage path={WINDOWS_PATH} />
        <LocalImage path={WINDOWS_PATH} />
      </div>,
    )

    await waitFor(() => expect(screen.getAllByRole('img')).toHaveLength(2))
    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(createObjectURL).toHaveBeenCalledTimes(1)
    expect(revokeObjectURL).not.toHaveBeenCalled()

    unmount()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledTimes(1))
  })

  it('懒加载：进入视口才发起请求', async () => {
    globalThis.IntersectionObserver =
      FakeIntersectionObserver as unknown as typeof IntersectionObserver
    fetchMock.mockResolvedValue(new Blob([new Uint8Array([1])], { type: 'image/png' }))
    render(<LocalImage path={WINDOWS_PATH} />)

    const observer = FakeIntersectionObserver.instances[0]
    expect(observer).toBeDefined()
    expect(observer.observed).toHaveLength(1)
    expect(fetchMock).not.toHaveBeenCalled()
    expect(screen.queryByRole('img')).toBeNull()

    act(() => observer.trigger(true))
    expect(await screen.findByRole('img')).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalledTimes(1)
    // 已进入视口：观察器随即释放，避免重复触发
    expect(observer.disconnected).toBe(1)
  })

  it('网络失败后就地重试保留中文路径，加载期间禁用重复点击并跳过懒加载等待', async () => {
    const path = String.raw`C:\截图\中文目录\最新结果.png`
    globalThis.IntersectionObserver =
      FakeIntersectionObserver as unknown as typeof IntersectionObserver
    let completeRetry!: (blob: Blob) => void
    fetchMock.mockRejectedValueOnce(new Error('network unavailable')).mockImplementationOnce(
      () =>
        new Promise<Blob>(resolve => {
          completeRetry = resolve
        }),
    )
    render(<LocalImage path={path} />)
    act(() => FakeIntersectionObserver.instances[0].trigger(true))

    fireEvent.click(await screen.findByRole('button', { name: '重新加载图片' }))

    const loading = screen.getByRole('button', { name: '正在加载图片…' })
    expect(loading).toBeDisabled()
    expect(screen.getByText(path)).toBeInTheDocument()
    fireEvent.click(loading)
    fireEvent.click(loading)
    expect(fetchMock).toHaveBeenCalledTimes(2)
    expect(FakeIntersectionObserver.instances).toHaveLength(1)

    await act(async () => completeRetry(new Blob(['image'], { type: 'image/png' })))
    expect(await screen.findByRole('img', { name: path })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '正在加载图片…' })).toBeNull()
    expect(fetchMock).toHaveBeenLastCalledWith(path)
  })

  it('解码失败重试下载新图片，同时保留其他实例仍在使用的旧 objectURL', async () => {
    createObjectURL.mockReturnValueOnce('blob:old-image').mockReturnValueOnce('blob:new-image')
    fetchMock.mockResolvedValue(new Blob(['image'], { type: 'image/png' }))
    const first = render(<LocalImage path={WINDOWS_PATH} />)
    const second = render(<LocalImage path={WINDOWS_PATH} />)
    const firstView = within(first.container)
    const secondView = within(second.container)
    fireEvent.click(await firstView.findByRole('img'))
    fireEvent.error(within(firstView.getByRole('dialog')).getByRole('img'))
    expect(firstView.queryByRole('dialog')).toBeNull()

    fireEvent.click(firstView.getByRole('button', { name: '重新加载图片' }))
    expect(await firstView.findByRole('img')).toHaveAttribute('src', 'blob:new-image')
    expect(secondView.getByRole('img')).toHaveAttribute('src', 'blob:old-image')
    expect(fetchMock).toHaveBeenCalledTimes(2)
    expect(revokeObjectURL).not.toHaveBeenCalledWith('blob:old-image')

    second.unmount()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:old-image'))
    expect(revokeObjectURL).not.toHaveBeenCalledWith('blob:new-image')
    const third = render(<LocalImage path={WINDOWS_PATH} />)
    expect(await within(third.container).findByRole('img')).toHaveAttribute('src', 'blob:new-image')
    expect(fetchMock).toHaveBeenCalledTimes(2)

    first.unmount()
    expect(revokeObjectURL).not.toHaveBeenCalledWith('blob:new-image')
    third.unmount()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:new-image'))
    expect(revokeObjectURL).toHaveBeenCalledTimes(2)
  })

  it('重试再次失败时保留路径并允许再试，英文界面显示英文状态', async () => {
    localStorage.setItem('nuphus_language', 'en')
    fetchMock.mockRejectedValue(new Error('network unavailable'))
    render(<LocalImage path={WINDOWS_PATH} />)

    fireEvent.click(await screen.findByRole('button', { name: 'Reload image' }))

    expect(await screen.findByRole('button', { name: 'Reload image' })).toBeEnabled()
    expect(screen.getByText(WINDOWS_PATH)).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalledTimes(2)
    expect(screen.queryByRole('img')).toBeNull()
  })

  it('同路径失败实例先后重试共享新请求，旧引用清理不会回收新图片', async () => {
    let completeRetry!: (blob: Blob) => void
    fetchMock.mockRejectedValueOnce(new Error('network unavailable')).mockImplementationOnce(
      () =>
        new Promise<Blob>(resolve => {
          completeRetry = resolve
        }),
    )
    const first = render(<LocalImage path={WINDOWS_PATH} />)
    const second = render(<LocalImage path={WINDOWS_PATH} />)
    const firstView = within(first.container)
    const secondView = within(second.container)

    fireEvent.click(await firstView.findByRole('button', { name: '重新加载图片' }))
    fireEvent.click(await secondView.findByRole('button', { name: '重新加载图片' }))
    expect(fetchMock).toHaveBeenCalledTimes(2)
    first.unmount()

    await act(async () => completeRetry(new Blob(['image'], { type: 'image/png' })))
    expect(await secondView.findByRole('img')).toHaveAttribute('src', 'blob:nuphus-image')
    expect(revokeObjectURL).not.toHaveBeenCalled()
    const third = render(<LocalImage path={WINDOWS_PATH} />)
    expect(await within(third.container).findByRole('img')).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalledTimes(2)

    second.unmount()
    expect(revokeObjectURL).not.toHaveBeenCalled()
    third.unmount()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledTimes(1))
  })

  it('重试未完成时卸载，迟到的 objectURL 仍被回收', async () => {
    let completeRetry!: (blob: Blob) => void
    fetchMock.mockRejectedValueOnce(new Error('network unavailable')).mockImplementationOnce(
      () =>
        new Promise<Blob>(resolve => {
          completeRetry = resolve
        }),
    )
    const { unmount } = render(<LocalImage path={WINDOWS_PATH} />)
    fireEvent.click(await screen.findByRole('button', { name: '重新加载图片' }))
    unmount()

    await act(async () => completeRetry(new Blob(['image'], { type: 'image/png' })))

    expect(revokeObjectURL).toHaveBeenCalledExactlyOnceWith('blob:nuphus-image')
    expect(screen.queryByRole('img')).toBeNull()
  })

  it('重试未完成时切换路径，迟到的旧图片不会覆盖新路径', async () => {
    const nextPath = String.raw`C:\新目录\截图.png`
    let completeRetry!: (blob: Blob) => void
    fetchMock
      .mockRejectedValueOnce(new Error('network unavailable'))
      .mockImplementationOnce(
        () =>
          new Promise<Blob>(resolve => {
            completeRetry = resolve
          }),
      )
      .mockResolvedValueOnce(new Blob(['next image'], { type: 'image/png' }))
    createObjectURL.mockReturnValueOnce('blob:next-image').mockReturnValueOnce('blob:late-image')
    const { rerender, unmount } = render(<LocalImage path={WINDOWS_PATH} />)
    fireEvent.click(await screen.findByRole('button', { name: '重新加载图片' }))
    rerender(<LocalImage path={nextPath} />)
    expect(await screen.findByRole('img', { name: nextPath })).toHaveAttribute(
      'src',
      'blob:next-image',
    )

    await act(async () => completeRetry(new Blob(['old image'], { type: 'image/png' })))

    expect(screen.getByRole('img', { name: nextPath })).toHaveAttribute('src', 'blob:next-image')
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:late-image')
    expect(revokeObjectURL).not.toHaveBeenCalledWith('blob:next-image')
    unmount()
    await waitFor(() => expect(revokeObjectURL).toHaveBeenCalledWith('blob:next-image'))
  })
})
