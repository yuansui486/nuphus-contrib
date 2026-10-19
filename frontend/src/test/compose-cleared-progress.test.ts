import { describe, expect, it } from 'vitest'
import { composeAssistantReplies, foldAssistantHistory } from '../core/progressMessages'
import type { ChatMessage } from '../core/types'

// 对齐改造后的展示层契约：无 progress 收据机制，历史/实时统一「连续 assistant 合并 +
// 最终回复覆盖」。本文件承接原 compose-cleared-progress 与 workflow-reply-bubble 的
// 展示断言（机制组件删除后，单气泡渲染由 ChatPanel 既有逻辑保证）。

const history: ChatMessage[] = [
  { id: 'user', role: 'user', content: '完成临时文件任务。', timestamp: 1 },
  { id: 'a1', role: 'assistant', content: '先检查文件。', timestamp: 2 },
  { id: 'a2', role: 'assistant', content: '正在验证结果。', timestamp: 3 },
  { id: 'a3', role: 'assistant', content: '任务已完成。', timestamp: 4, runtime: 'done' },
]

describe('single workflow reply (leader-aligned)', () => {
  it('history folds consecutive assistant rounds into one reply', () => {
    const folded = foldAssistantHistory(history)
    expect(folded.map(m => m.content)).toEqual(['完成临时文件任务。', '任务已完成。'])
    const visible = composeAssistantReplies(folded)
    expect(visible.map(m => m.content)).toEqual(['完成临时文件任务。', '任务已完成。'])
    expect(visible).toHaveLength(2)
  })
  it('compose is idempotent on folded history', () => {
    const folded = foldAssistantHistory(history)
    const visible = composeAssistantReplies(folded)
    expect(composeAssistantReplies(visible)).toEqual(visible)
  })
})
