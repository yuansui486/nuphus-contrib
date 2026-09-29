/**
 * 选中聊天区文字 → 引用追问
 *
 * 复用既有的 ChatReference 通道（输入区引用 chip + `resolve_references` 前缀注入），
 * 不新增 IPC 协议位：`type: 'quote'` 走同一条链，原文由 `label` 承载。
 *
 * 为什么让 label 承载全文而不是再加一个 text 字段：
 *   - 引用 chip 的视觉截断由 `.ref-chip-label`（max-width 180px + ellipsis）负责，
 *     后端注入要的是完整文本——两者天然异长，但语义同源（这段被引用的内容本身）。
 *   - 多一个字段就多一处必须同步的协议位：`mobile_server.rs` 的 MobileMessage
 *     复用同一个 Rust 结构体，桌面 / 移动两端都要跟上。
 *   - 代价是 title 提示与注入共用一份字符串，故 ReferenceBar 对 quote 特判 title。
 *
 * 边界：单次引用有长度上限——选区可能被误拖到整篇长文，进 textarea 与 prompt 都会爆。
 */

/** 单次引用的字符上限（超出截断并在 label 尾部标注，避免模型把片段当全文） */
export const QUOTE_MAX_CHARS = 2000

export interface QuotePayload {
  /** 截断后的文本；truncated=false 时等于 trim 后的原文 */
  text: string
  truncated: boolean
}

/** 去首尾空白后按上限截断。空串原样返回（是否可用由 buildQuoteRef 判定）。 */
export function truncateQuote(raw: string, max: number = QUOTE_MAX_CHARS): QuotePayload {
  const text = raw.trim()
  if (text.length <= max) return { text, truncated: false }
  return { text: text.slice(0, max), truncated: true }
}

/**
 * djb2 → base36。`addReference` 按 `type + id` 去重，没有稳定 id 时引用两段不同
 * 文本会被静默吞掉第二段（id 同为空串即判重）。同内容同 id 同时避免重复引用同一段。
 */
export function quoteId(text: string): string {
  let h = 5381
  for (let i = 0; i < text.length; i++) h = ((h << 5) + h + text.charCodeAt(i)) | 0
  return `q${(h >>> 0).toString(36)}`
}

/**
 * 选区锚点是否落在消息气泡正文内。`.message-content` 由 ChatPanel 的消息气泡渲染，
 * 命中说明这次 selection 是用户在读消息内容，而不是在拖侧栏 / 选中界面文案。
 */
export function isSelectableInBubble(node: Node | null): boolean {
  if (!node) return false
  const el = node instanceof Element ? node : node.parentElement
  return !!el?.closest('.message-content')
}

/** 选中文本 → 引用 chip 数据；空选区返回 null（调用方据此不弹浮条）。 */
export function buildQuoteRef(raw: string): {
  type: 'quote'
  id: string
  label: string
} | null {
  const { text, truncated } = truncateQuote(raw)
  if (!text) return null
  return {
    type: 'quote',
    id: quoteId(text),
    label: truncated ? `${text}\n…（已截断）` : text,
  }
}
