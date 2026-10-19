import { act, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { composeAssistantReplies, foldAssistantHistory } from '../core/progressMessages'
import { chatReducer, initialChatState } from '../mobile/store'
import { useEvents, type EventHandlers } from '../hooks/useEvents'
import type { ChatMessage, NuphusEvent } from '../core/types'

const { callbacks } = vi.hoisted(() => ({
  callbacks: new Map<string, (payload: unknown) => void>(),
}))
vi.mock('../core/bridge', () => ({
  invoke: vi.fn(async () => undefined),
  listen: vi.fn(async (name: string, callback: (payload: unknown) => void) => {
    callbacks.set(name, callback)
    return () => callbacks.delete(name)
  }),
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ setFocus: vi.fn() }) }))
vi.mock('../ui/sound', () => ({ playUiSound: vi.fn() }))
vi.mock('../ui/islandChannel', () => ({ showAppFeedbackByHudPhase: vi.fn() }))

const started: NuphusEvent = {
  type: 'execution_started',
  session_id: 'session',
  turn_id: 'turn',
  step_index: 0,
  goal: 'Temporary file task',
  tools: [],
  source: 'workflow',
  mode: 'workflow',
}
const text: NuphusEvent = {
  type: 'llm_text_delta',
  text: '先检查已有流程。',
  is_thinking: false,
  from_task: false,
}
const completed: NuphusEvent = {
  type: 'execution_completed',
  step_index: 0,
  output: { result_message: '完成了。', step_index: 0, artifacts: [], tool_calls_count: 0 },
  total_duration_ms: 0,
  total_calls: 0,
}

function desktop() {
  let messages: ChatMessage[] = []
  const refs = {
    streamingMsgId: { current: null as string | null },
    lastStreamingMsgId: { current: null as string | null },
    executionActiveRef: { current: false },
    processingRef: { current: false },
    turnMetaTokensRef: { current: {} },
    interruptedRef: { current: false },
    // 贴底跟随 followReset 回填位（useEvents execution_started / execution_completed 调）；
    // 本桩不渲染 ChatPanel，置 null = 无人回填，useEvents 侧 ?.() 静默跳过
    stickyFollowResetRef: { current: null },
  }
  const handlers = new Proxy(
    {
      refs,
      messages,
      setMessages: (value: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => {
        messages = typeof value === 'function' ? value(messages) : value
      },
      addMessage: (message: ChatMessage) => {
        messages.push(message)
      },
    },
    { get: (target, name) => (name in target ? target[name as keyof typeof target] : vi.fn()) },
  )
  renderHook(() => useEvents(handlers as unknown as EventHandlers))
  let seq = 0
  const send = (event: NuphusEvent) =>
    act(() => callbacks.get('nuphus-event')?.({ seq: ++seq, event }))
  send(started)
  return { send, messages: () => composeAssistantReplies(messages), records: () => messages, refs }
}

describe('workflow reply aligned with leader (single streaming bubble + final override)', () => {
  afterEach(() => {
    callbacks.clear()
    vi.useRealTimers()
  })
  it('streams agent text into the bubble and overrides with the final reply on completion', () => {
    const h = desktop()
    h.send(text)
    // 执行中：流式过程文本在气泡中可见
    expect(h.messages().map(m => m.content)).toEqual(['先检查已有流程。'])
    h.send(completed)
    // 完成后：最终回复覆盖流式过程文本（与 leader 同构，气泡不混杂）
    expect(h.messages().map(m => m.content)).toEqual(['完成了。'])
    expect(h.messages()[0].runtime).toBe('done')
  })
  it('keeps thinking out of the bubble but in the timeline', () => {
    const h = desktop()
    h.send({ ...text, text: '思考片段', is_thinking: true })
    h.send(text)
    h.send(completed)
    expect(h.messages().map(m => m.content)).toEqual(['完成了。'])
  })
  it('output after an appended user message stays after that message', () => {
    const h = desktop()
    h.send(text)
    h.send({ type: 'user_message_received', source: 'mobile', content: '改用新目录' })
    h.send({ ...text, text: '按新目录继续。' })
    h.send(completed)
    expect(h.messages().map(m => m.content)).toEqual(['先检查已有流程。', '改用新目录', '完成了。'])
    expect(new Set(h.messages().map(m => m.id)).size).toBe(3)
  })
  it('does not leave an empty bubble when stopped before any public text', () => {
    const h = desktop()
    h.send({ type: 'execution_error', step_index: 0, error: 'Stopped' })
    expect(h.messages()).toHaveLength(0)
  })
})

describe('history folding aligned with leader (consecutive assistant merge)', () => {
  it('history composition respects user and refine boundaries and preserves media', () => {
    const messages: ChatMessage[] = [
      {
        id: 'a1',
        role: 'assistant',
        content: '检查。',
        timestamp: 1,
      },
      {
        id: 'a2',
        role: 'assistant',
        content: '完成。',
        images: ['image'],
        audio: ['audio'],
        timestamp: 3,
      },
      { id: 'r', role: 'refine', content: '提炼', timestamp: 4 },
      { id: 'u', role: 'user', content: '新问题', timestamp: 6 },
      { id: 'f2', role: 'assistant', content: '新回复', timestamp: 7 },
    ]
    const visible = composeAssistantReplies(foldAssistantHistory(messages))
    expect(visible.map(m => m.content)).toEqual(['完成。', '提炼', '新问题', '新回复'])
    expect(visible[0]).toMatchObject({ images: ['image'], audio: ['audio'] })
    expect(composeAssistantReplies(visible)).toEqual(visible)
  })
  it('old history still folds consecutive assistant rounds', () => {
    expect(
      foldAssistantHistory([
        { role: 'assistant', content: 'old' },
        { role: 'assistant', content: 'final' },
      ]),
    ).toEqual([{ role: 'assistant', content: 'final', traceItems: [] }])
  })
  it('history retains audio-only answers and tool traces', () => {
    const folded = foldAssistantHistory([
      { role: 'assistant', content: 'checking' },
      { role: 'assistant', content: '', traceItems: ['last tool'] },
      { role: 'user', content: 'next' },
      { role: 'assistant', content: '', audio: ['data:audio/test'] },
    ])
    expect(folded).toHaveLength(3)
    expect(folded[0].traceItems).toEqual(['last tool'])
    expect(folded[2].audio).toEqual(['data:audio/test'])
  })
})

describe('mobile aligned with leader', () => {
  it('mobile streams text and overrides with final on completion', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: text })
    state = chatReducer(state, { type: 'event', event: completed })
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual(['完成了。'])
  })
  it('mobile ignores stale progress and does not expose subtask text as narration', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: { session_id: 'other' } as NuphusEvent })
    state = chatReducer(state, { type: 'event', event: { ...text, from_task: true } })
    expect(state.messages.every(m => !m.content)).toBe(true)
  })
  it('mobile keeps subsequent output after the user append', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: text })
    state = chatReducer(state, {
      type: 'optimistic',
      message: { id: 'u2', role: 'user', content: '追加' },
    })
    state = chatReducer(state, { type: 'event', event: { ...text, text: '收到。' } })
    state = chatReducer(state, { type: 'event', event: completed })
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。',
      '追加',
      '完成了。',
    ])
  })
  it('mobile preserves a streaming reply on pause and interruption without placeholders', () => {
    let state = chatReducer(initialChatState, { type: 'event', event: started })
    state = chatReducer(state, { type: 'event', event: text })
    state = chatReducer(state, {
      type: 'event',
      event: { type: 'execution_paused', action_id: 'a' },
    })
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。',
    ])
    state = chatReducer(state, { type: 'sync_running', running: false })
    expect(composeAssistantReplies(state.messages).map(m => m.content)).toEqual([
      '先检查已有流程。',
    ])
  })
})
