import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

import { invoke } from '@tauri-apps/api/core'
import {
  listChatAgentsInline,
  processInput,
  submitExecutionRating,
  updateChatAgentInline,
} from './api'

const tauriInvoke = vi.mocked(invoke)

/** Exercise the real API wrapper and bridge at the Tauri command boundary. */
describe('desktop command argument contracts', () => {
  beforeEach(() => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    tauriInvoke.mockReset()
  })

  afterEach(() => {
    Reflect.deleteProperty(window, '__TAURI_INTERNALS__')
  })

  it('submits a rating with the required Tauri argument names', async () => {
    const steps = [{ tool: 'Read', durationMs: 12, success: true }]
    tauriInvoke.mockResolvedValue('rating-中文会话')

    await expect(submitExecutionRating('读取中文路径', 5, '完成', steps, '中文会话')).resolves.toBe(
      'rating-中文会话',
    )
    expect(tauriInvoke).toHaveBeenCalledWith('submit_execution_rating', {
      goal: '读取中文路径',
      rating: 5,
      comment: '完成',
      toolsSummary: 'Read',
      stepsJson: JSON.stringify(steps),
      sessionId: '中文会话',
    })
  })

  it('reads inline steps using workflowId without changing response field names', async () => {
    const entries = [{ workflow_id: '工作流', step_id: '步骤', config: { agent_id: '模板' } }]
    tauriInvoke.mockResolvedValue(entries)

    await expect(listChatAgentsInline('工作流')).resolves.toBe(entries)
    expect(tauriInvoke).toHaveBeenCalledWith('chat_agent_list_inline', { workflowId: '工作流' })
  })

  it('updates inline steps with camelCase command args and an unchanged nested config', async () => {
    const config = { agent_id: '模板', max_iterations: 7, persona: '中文角色' }
    tauriInvoke.mockResolvedValue(null)

    await updateChatAgentInline('工作流', '步骤', config)
    expect(tauriInvoke).toHaveBeenCalledWith('chat_agent_update_inline', {
      workflowId: '工作流',
      stepId: '步骤',
      config,
    })
  })

  it.each([
    [true, true],
    [false, false],
    [undefined, false],
  ])('sends newSession=%s as %s at the IPC boundary', async (newSession, expected) => {
    const response = { result: 'accepted' }
    tauriInvoke.mockResolvedValue(response)

    await expect(
      processInput('你好', [], undefined, 'send-1', 'leader', undefined, undefined, newSession),
    ).resolves.toBe(response)
    expect(tauriInvoke).toHaveBeenCalledWith('send_message_cmd', {
      message: '你好',
      history: [],
      relation: undefined,
      sendId: 'send-1',
      mode: 'leader',
      images: undefined,
      references: undefined,
      newSession: expected,
    })
  })
})
