import type { PendingFile, PendingImage } from '../../core/types'

/** 引用栏条目：由 HTML5 `File` 转换而来（拖拽与附件按钮共用同一条通道）。 */
export type AttachmentEntry =
  { kind: 'image'; image: PendingImage } | { kind: 'file'; file: PendingFile }

/** 图片扩展名白名单——与后端 `read_image_base64` 保持一致。 */
const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'ico'])

export function isImageFile(file: File): boolean {
  if (file.type.startsWith('image/')) return true
  return isImageName(file.name)
}

/** 按文件名判断图片（粘贴的剪贴板路径分支没有 `File.type`，只能看扩展名）。 */
export function isImageName(name: string): boolean {
  const dot = name.lastIndexOf('.')
  if (dot < 0) return false
  return IMAGE_EXTS.has(name.slice(dot + 1).toLowerCase())
}

/** 读 File 为 data URL（`data:<mime>;base64,<...>`）。 */
export function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const url = reader.result
      typeof url === 'string' ? resolve(url) : reject(new Error('读取文件失败'))
    }
    reader.onerror = () => reject(reader.error ?? new Error('读取文件失败'))
    reader.readAsDataURL(file)
  })
}

/** data URL → 纯 base64 载荷（去掉 `data:...;base64,` 前缀）。 */
function stripDataUrlPrefix(dataUrl: string): string {
  const idx = dataUrl.indexOf(',')
  return idx >= 0 ? dataUrl.slice(idx + 1) : dataUrl
}

/**
 * 落盘换真实路径：浏览器屏蔽 `File.path`，非图片附件必须经后端写入缓存目录，
 * 拿到绝对路径后才能像历史原生通道一样被引用栏与后端读文件消费。
 */
async function persistFileForPath(file: File): Promise<PendingFile> {
  const { invoke } = await import('@tauri-apps/api/core')
  const base64 = stripDataUrlPrefix(await readFileAsDataUrl(file))
  const saved = await invoke<{ path: string; name: string }>('save_dropped_file', {
    fileName: file.name,
    base64Data: base64,
  })
  return { path: saved.path, name: saved.name || file.name }
}

/**
 * 单个 File → 引用栏条目。
 *
 * 图片：objectURL 立即给出可预览的 dataUrl 占位语义（这里直接用 dataUrl，
 * 保证发送链路拿到的就是可上传的字符串），调用方可先渲染再补齐。
 * 文档：落盘换绝对路径。
 */
export async function fileToAttachment(file: File): Promise<AttachmentEntry> {
  if (isImageFile(file)) {
    const dataUrl = await readFileAsDataUrl(file)
    return { kind: 'image', image: { dataUrl, name: file.name } }
  }
  return { kind: 'file', file: await persistFileForPath(file) }
}

/** 批量转换：逐个处理，单个失败不影响其余（返回成功项 + 失败描述）。 */
export async function filesToAttachments(
  files: File[],
): Promise<{ entries: AttachmentEntry[]; failures: string[] }> {
  const entries: AttachmentEntry[] = []
  const failures: string[] = []
  for (const file of files) {
    try {
      entries.push(await fileToAttachment(file))
    } catch (e) {
      failures.push(`${file.name}: ${e instanceof Error ? e.message : String(e)}`)
    }
  }
  return { entries, failures }
}
