import { describe, expect, it, vi, beforeEach } from 'vitest'
import { filesToAttachments, isImageFile, isImageName } from '../main-window/chat/attachments'

/**
 * 附件统一通道回归测试。
 *
 * 重构背景：拖拽与附件按钮统一为 HTML5 通道（图片/文档一律进引用栏）。
 * 非图片文档浏览器拿不到绝对路径，必须经后端 `save_dropped_file` 落盘换路径。
 */

const invokeMock = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))

/** jsdom 的 FileReader 可用，这里用真实 File 以走通 readAsDataURL。 */
function makeFile(name: string, type: string, content = 'hello'): File {
  return new File([content], name, { type })
}

beforeEach(() => {
  invokeMock.mockReset()
  invokeMock.mockResolvedValue({ path: 'C:\\cache\\dropped\\a.txt', name: 'a.txt' })
})

describe('图片判定', () => {
  it('按 MIME 与扩展名双路判定图片', () => {
    expect(isImageFile(makeFile('x.png', 'image/png'))).toBe(true)
    expect(isImageFile(makeFile('x.jpg', 'image/jpeg'))).toBe(true)
    // 无 MIME 时靠扩展名兜底（剪贴板路径场景）
    expect(isImageFile(makeFile('x.JPEG', ''))).toBe(true)
    expect(isImageFile(makeFile('x.txt', 'text/plain'))).toBe(false)
    expect(isImageFile(makeFile('noext', 'text/plain'))).toBe(false)
  })

  it('isImageName 按文件名判定', () => {
    expect(isImageName('a.PNG')).toBe(true)
    expect(isImageName('a.webp')).toBe(true)
    expect(isImageName('a.txt')).toBe(false)
  })
})

describe('filesToAttachments · 图片', () => {
  it('图片转 dataUrl 并进引用栏（不落盘、不调后端）', async () => {
    const { entries, failures } = await filesToAttachments([
      makeFile('shot.png', 'image/png', 'PNGDATA'),
    ])
    expect(failures).toEqual([])
    expect(entries).toHaveLength(1)
    const entry = entries[0]
    expect(entry.kind).toBe('image')
    if (entry.kind === 'image') {
      expect(entry.image.name).toBe('shot.png')
      expect(entry.image.dataUrl.startsWith('data:image/png;base64,')).toBe(true)
    }
    // 图片不需要后端换路径
    expect(invokeMock).not.toHaveBeenCalled()
  })
})

describe('filesToAttachments · 文档', () => {
  it('非图片经后端落盘换真实路径进引用栏', async () => {
    const { entries, failures } = await filesToAttachments([
      makeFile('doc.txt', 'text/plain', 'CONTENT'),
    ])
    expect(failures).toEqual([])
    expect(invokeMock).toHaveBeenCalledWith('save_dropped_file', expect.anything())
    const entry = entries[0]
    expect(entry.kind).toBe('file')
    if (entry.kind === 'file') {
      expect(entry.file.path).toBe('C:\\cache\\dropped\\a.txt')
      expect(entry.file.name).toBe('a.txt')
    }
  })

  it('落盘失败不阻断其余附件（逐个隔离）', async () => {
    invokeMock.mockRejectedValueOnce(new Error('落盘失败'))
    const { entries, failures } = await filesToAttachments([
      makeFile('bad.txt', 'text/plain'),
      makeFile('good.png', 'image/png'),
    ])
    // 好的图片仍在
    expect(entries).toHaveLength(1)
    expect(entries[0].kind).toBe('image')
    // 失败被记录
    expect(failures).toHaveLength(1)
    expect(failures[0]).toContain('bad.txt')
  })
})
