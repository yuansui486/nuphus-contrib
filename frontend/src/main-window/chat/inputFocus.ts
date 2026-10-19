/**
 * inputFocus —— 输入框聚焦的工具函数。
 *
 * 仓库里两条「把内容送进输入区」的链路（右键「问问 Nuphus」的
 * append-to-chat、引用入栏）都要求事后焦点落在输入框、光标置末尾，
 * 否则用户必须再点一次才能打字。该语义只此一份实现。
 *
 * 为什么在 rAF 里调用（调用方负责）：入栏会触发引用栏重渲染，
 * 同一帧内 focus 可能被后续布局/重渲染打断；rAF 等本帧 DOM 落定后
 * 再取一次 textareaRef.current，与 append-to-chat 既有行为一致。
 */

/** 聚焦输入框并把光标移到末尾。空引用安全（no-op）。 */
export function focusInputAtEnd(ta: HTMLTextAreaElement | null): void {
  if (!ta) return
  ta.focus()
  const len = ta.value.length
  ta.setSelectionRange(len, len)
}
