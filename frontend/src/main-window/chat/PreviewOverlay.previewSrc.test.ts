import { describe, expect, it, vi } from 'vitest'

vi.mock('../lib/api', () => ({
  readFile: vi.fn(() => Promise.resolve('')),
  readFileBase64: vi.fn(() => Promise.resolve('')),
  openPath: vi.fn(() => Promise.resolve()),
  revealPath: vi.fn(() => Promise.resolve()),
  hudUpdate: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: vi.fn(),
}))

vi.mock('pdfjs-dist', () => ({ GlobalWorkerOptions: {}, getDocument: vi.fn() }))

import { convertFileSrc } from '@tauri-apps/api/core'
import { previewFileSrc } from './PreviewOverlay'

/**
 * HTML 预览相对资源回归：convertFileSrc 把整条绝对路径编码成单段 URL，
 * 浏览器按 '/' 解析相对资源时目录边界丢失（Windows 的 %5C 尤其明显）。
 * 这里锁住「编码分隔符 → 真实 '/'」的还原，覆盖子目录与 ../ 两种引用。
 */
describe('previewFileSrc', () => {
  it('Windows: 所有 %5C 还原为 /，assets 与 ../ 均按目录解析', () => {
    vi.mocked(convertFileSrc).mockReturnValue(
      'http://preview.localhost/C%3A%5CUsers%5Cme%5Csite%5Ccapability%5Cindex.html',
    )

    const url = previewFileSrc('C:\\Users\\me\\site\\capability\\index.html')

    expect(url).toBe('http://preview.localhost/C%3A/Users/me/site/capability/index.html')
    expect(new URL('assets/site.css', url).href).toBe(
      'http://preview.localhost/C%3A/Users/me/site/capability/assets/site.css',
    )
    expect(new URL('../assets/site.css', url).href).toBe(
      'http://preview.localhost/C%3A/Users/me/site/assets/site.css',
    )
  })

  it('Unix: 保留首个 %2F，其余还原为 /，../ 不丢根', () => {
    vi.mocked(convertFileSrc).mockReturnValue(
      'preview://localhost/%2FUsers%2Fme%2Fsite%2Fcapability%2Findex.html',
    )

    const url = previewFileSrc('/Users/me/site/capability/index.html')

    expect(url).toBe('preview://localhost/%2FUsers/me/site/capability/index.html')
    expect(new URL('assets/site.css', url).href).toBe(
      'preview://localhost/%2FUsers/me/site/capability/assets/site.css',
    )
    expect(new URL('../assets/site.css', url).href).toBe(
      'preview://localhost/%2FUsers/me/site/assets/site.css',
    )
  })
})
