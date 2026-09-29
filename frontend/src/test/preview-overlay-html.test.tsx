import { render, screen, fireEvent } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { FilePreviewContent, PreviewOverlay } from '../main-window/chat/PreviewOverlay'
import { convertFileSrc } from '@tauri-apps/api/core'
import { openPath, readFile, readFileBase64, revealPath } from '../main-window/lib/api'

// 回归背景（2026-09-20）：html/htm 曾被「非文本类型 → 系统默认程序打开」的早退分支
// 截走——从 preview:// 沙箱底座上线起，本文件末尾的 iframe 分支就不可达，点 HTML 交付物
// 只会看到「已请求系统默认程序打开」占位。这两条测试分别钉住「不再截走」与「不回归兜底」。

vi.mock('../main-window/lib/api', () => ({
  readFile: vi.fn(() => Promise.resolve('')),
  readFileBase64: vi.fn(() => Promise.resolve('')),
  openPath: vi.fn(() => Promise.resolve()),
  revealPath: vi.fn(() => Promise.resolve()),
}))

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: vi.fn((p: string, protocol = 'asset') => `http://${protocol}.localhost/${p}`),
}))

vi.mock('pdfjs-dist', () => ({ GlobalWorkerOptions: {}, getDocument: vi.fn() }))

describe('FilePreviewContent 类型分流', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it.each([
    ['html', String.raw`C:\Users\me\Nuphus\frontend\popout-test.html`],
    ['htm', '/Users/me/repo/demo.htm'],
  ])('%s 走 preview:// 沙箱 iframe，不调系统默认程序', (_ext, path) => {
    const { container } = render(<FilePreviewContent path={path} />)

    const iframe = container.querySelector('iframe.pv-iframe')
    expect(iframe).toBeInTheDocument()
    expect(iframe).toHaveAttribute('src', `http://preview.localhost/${path}`)
    expect(iframe).toHaveAttribute('sandbox', expect.stringContaining('allow-scripts'))
    expect(convertFileSrc).toHaveBeenCalledWith(path, 'preview')

    // 关键回归点：HTML 绝不能再落进「非文本 → openPath」那条路
    expect(openPath).not.toHaveBeenCalled()
    expect(readFile).not.toHaveBeenCalled()
    expect(readFileBase64).not.toHaveBeenCalled()
    expect(screen.queryByText('已请求系统默认程序打开')).not.toBeInTheDocument()
    expect(screen.queryByText('读取中…')).not.toBeInTheDocument()
  })

  it('docx 等真·非预览类型仍走系统默认程序打开', () => {
    const path = String.raw`C:\repo\out\报告.docx`
    render(<FilePreviewContent path={path} />)

    expect(openPath).toHaveBeenCalledWith(path)
    expect(screen.getByText('已请求系统默认程序打开')).toBeInTheDocument()
  })
})

// 回归背景（issue #85）：PreviewOverlay 工具栏的「系统打开 / 在文件夹显示」此前用
// `.catch(() => undefined)` 把 revealPath / openPath 的错误静默吞掉，用户看到的是
// 「点了没反应」；.pv-open-error 横幅样式自上线起没有 .tsx 引用。这两条钉住两个入口
// 都会把后端给出的中文错误显示出来。
describe('PreviewOverlay 工具栏：系统打开/定位失败必须可见', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('「在文件夹显示」失败 → 渲染 .pv-open-error 横幅与后端错误文案', async () => {
    vi.mocked(revealPath).mockRejectedValueOnce('路径不存在，无法定位：/tmp/nope/a.txt')
    render(<PreviewOverlay path="/tmp/nope/a.txt" onClose={() => undefined} />)

    fireEvent.click(screen.getByTitle('在文件管理器中定位'))

    expect(await screen.findByText('路径不存在，无法定位：/tmp/nope/a.txt')).toBeInTheDocument()
    expect(document.querySelector('.pv-open-error')).not.toBeNull()
  })

  it('「系统打开」失败 → 同一横幅承接 openPath 错误', async () => {
    vi.mocked(openPath).mockRejectedValueOnce('系统打开失败：拒绝访问')
    render(<PreviewOverlay path="/tmp/nope/a.txt" onClose={() => undefined} />)

    fireEvent.click(screen.getByTitle('用系统默认程序打开'))

    expect(await screen.findByText('系统打开失败：拒绝访问')).toBeInTheDocument()
  })

  it('再次尝试前先清掉上一条失败，不叠出两条横幅', async () => {
    vi.mocked(revealPath)
      .mockRejectedValueOnce('路径不存在，无法定位：/tmp/nope/a.txt')
      .mockResolvedValueOnce(undefined)
    render(<PreviewOverlay path="/tmp/nope/a.txt" onClose={() => undefined} />)
    const btn = screen.getByTitle('在文件管理器中定位')

    fireEvent.click(btn)
    expect(await screen.findByText('路径不存在，无法定位：/tmp/nope/a.txt')).toBeInTheDocument()

    fireEvent.click(btn)
    expect(document.querySelector('.pv-open-error')).toBeNull()
  })
})
