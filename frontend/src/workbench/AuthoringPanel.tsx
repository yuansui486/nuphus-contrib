import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import {
  IconArrowLeft,
  IconChevronDown,
  IconList,
  IconSend,
  IconSparkles,
  IconSquare,
} from '../ui/Icons'
import { useLanguage } from '../locales'
import { IntentFormPanel } from '../main-window/workflow-canvas/IntentFormPanel'
import { buildIntentTextTemplate } from '../main-window/workflow-canvas/intentText'
import type { Draft } from './api'
import { UserInputPrompt } from '../main-window/layout/UserInputPrompt'
import { SecurityPrompt } from '../main-window/layout/SecurityPrompt'
import './authoring.css'

interface Props {
  draft: Draft
  intent: string
  onIntentConsumed: () => void
  onBusyChange: (busy: boolean) => void
  onChanged: () => void
  beforeGenerate: () => Promise<boolean>
  collapsed?: boolean
  onCollapse?: () => void
}
interface History {
  busy: boolean
  turn_id?: string
  pending?: AuthoringPrompt[]
  session?: {
    messages?: Array<{
      role: string
      internal?: boolean
      content?: Array<{ type?: string; text?: string }>
    }>
  }
}
interface Progress {
  project_id: string
  workflow_id: string
  turn_id: string
  event: AuthoringPrompt & { text?: string; name?: string; success?: boolean; delta?: boolean }
}

interface AuthoringPrompt {
  type: string
  action_id?: string
  title?: string
  prompt?: string
  sensitive?: boolean
  input_type?: string
  icon_path?: string
  default_name?: string
  default_shortcut?: string
  rel_x?: number
  rel_y?: number
  default_note?: string
  default_stage?: string
  tool?: string
  risk?: 'low' | 'medium' | 'high' | 'critical'
  reason?: string
}

export function AuthoringPanel({
  draft,
  intent,
  onIntentConsumed,
  onBusyChange,
  onChanged,
  beforeGenerate,
  collapsed = false,
  onCollapse,
}: Props) {
  const { lang } = useLanguage()
  const ui = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const draftKey = `workbench:authoring:${draft.project_id}:${draft.workflow_id}`
  const [input, setInput] = useState(() => {
    try {
      return sessionStorage.getItem(draftKey) ?? ''
    } catch {
      return ''
    }
  })
  const [guided, setGuided] = useState(false)
  const [busy, setBusy] = useState(false)
  const [ready, setReady] = useState(false)
  const [initializationFailed, setInitializationFailed] = useState(false)
  const [initializing, setInitializing] = useState(true)
  const [history, setHistory] = useState<Array<{ role: string; text: string }>>([])
  const [progress, setProgress] = useState('')
  const [activity, setActivity] = useState('')
  const [error, setError] = useState('')
  const [inputNotice, setInputNotice] = useState('')
  const [cancelling, setCancelling] = useState(false)
  const [awayFromBottom, setAwayFromBottom] = useState(false)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const logRef = useRef<HTMLDivElement>(null)
  const following = useRef(true)
  const composing = useRef(false)
  const consumedIntent = useRef('')
  const starting = useRef(false)
  const mounted = useRef(true)
  const historySequence = useRef(0)
  const retryInitialization = useRef<() => void>(() => {})
  const showError = (reason: unknown) =>
    setError(
      reason && typeof reason === 'object' && 'message' in reason
        ? String(reason.message)
        : String(reason),
    )
  useEffect(() => {
    try {
      sessionStorage.setItem(draftKey, input)
    } catch {
      /* Optional draft persistence. */
    }
  }, [draftKey, input])
  const callbacks = useRef({ onBusyChange, onChanged })
  callbacks.current = { onBusyChange, onChanged }
  const identity = { projectId: draft.project_id, workflowId: draft.workflow_id }
  const [prompts, setPrompts] = useState<AuthoringPrompt[]>([])
  const activeTurn = useRef<string | undefined>(undefined)
  const completedTurns = useRef(new Set<string>())
  const eventSequence = useRef(0)
  const refreshHistory = useCallback(async () => {
    const sequence = ++historySequence.current
    const eventsAtRequest = eventSequence.current
    const result = await invoke<History>('workbench_generate', {
      projectId: draft.project_id,
      workflowId: draft.workflow_id,
      action: 'history',
    })
    if (!mounted.current || sequence !== historySequence.current) return
    const rows = (result.session?.messages ?? [])
      .filter(message => !message.internal && ['user', 'assistant'].includes(message.role))
      .map(message => ({
        role: message.role,
        text: (message.content ?? [])
          .filter(block => block.type === 'text' && block.text)
          .map(block => block.text)
          .join('\n'),
      }))
      .filter(message => message.text)
    setHistory(rows)
    // A prompt or completion received while history was loading is newer
    // than that snapshot. Do not resurrect its old busy/prompt state.
    if (eventsAtRequest !== eventSequence.current) return
    setBusy(result.busy)
    activeTurn.current = result.turn_id
    setPrompts(result.pending ?? [])
    callbacks.current.onBusyChange(result.busy)
  }, [draft.project_id, draft.workflow_id])
  useEffect(() => {
    let alive = true
    let initializingNow = false
    let stopListening: (() => void) | undefined
    mounted.current = true
    activeTurn.current = undefined
    const onProgress = (payload: Progress) => {
      if (
        !alive ||
        payload.project_id !== draft.project_id ||
        payload.workflow_id !== draft.workflow_id
      )
        return
      const event = payload.event
      if (completedTurns.current.has(payload.turn_id)) return
      if (activeTurn.current && payload.turn_id !== activeTurn.current) return
      if (!activeTurn.current) activeTurn.current = payload.turn_id
      ++eventSequence.current
      if (event.type !== 'completed') {
        setBusy(true)
        callbacks.current.onBusyChange(true)
      }
      if (event.type === 'user_input_request' || event.type === 'security_check') {
        setPrompts(previous => [...previous.filter(p => p.action_id !== event.action_id), event])
      }
      if (event.type === 'prompt_timeout')
        setPrompts(previous => previous.filter(p => p.action_id !== event.action_id))
      if (event.type === 'run') callbacks.current.onChanged()
      if (event.type === 'tool_result') {
        setActivity(event.name ?? '')
        if (['canvas_update', 'workflow_save', 'canvas_create'].includes(event.name ?? ''))
          callbacks.current.onChanged()
      }
      if (event.type === 'progress')
        setProgress(previous =>
          event.delta
            ? previous + (event.text ?? '')
            : previous
              ? `${previous}\n\n${event.text ?? ''}`
              : (event.text ?? ''),
        )
      if (event.type === 'tool') {
        setActivity(event.name ?? '')
        setProgress(previous =>
          previous && !previous.endsWith('\n\n') ? `${previous}\n\n` : previous,
        )
      }
      if (event.type === 'error') setError(event.text ?? '')
      if (event.type === 'completed') {
        completedTurns.current.add(payload.turn_id)
        setPrompts([])
        setBusy(false)
        setCancelling(false)
        callbacks.current.onBusyChange(false)
        setActivity('')
        setProgress(previous => [previous, event.text].filter(Boolean).join('\n\n'))
        if (!event.success) setError(event.text ?? '')
        callbacks.current.onChanged()
        void refreshHistory().catch(showError)
      }
    }
    const initialize = async () => {
      if (!alive || initializingNow) return
      initializingNow = true
      setInitializing(true)
      setError('')
      try {
        if (!stopListening) {
          const unsubscribe = await listen<Progress>('workbench-authoring', ({ payload }) =>
            onProgress(payload),
          )
          if (!alive) {
            unsubscribe()
            return
          }
          stopListening = unsubscribe
        }
        // Re-read after subscribing, including retries, so a turn completed
        // while the listener was unavailable cannot leave a stale busy state.
        await refreshHistory()
        if (alive) {
          setReady(true)
          setInitializationFailed(false)
        }
      } catch (reason) {
        if (alive) {
          showError(reason)
          setInitializationFailed(true)
        }
      } finally {
        initializingNow = false
        if (alive) setInitializing(false)
      }
    }
    retryInitialization.current = () => void initialize()
    void initialize()
    return () => {
      alive = false
      mounted.current = false
      stopListening?.()
      callbacks.current.onBusyChange(false)
    }
  }, [draft.project_id, draft.workflow_id, refreshHistory])
  const appendInput = useCallback((text: string) => {
    setInput(previous => (previous.trim() ? `${previous.trimEnd()}\n\n${text}` : text))
    requestAnimationFrame(() => inputRef.current?.focus())
  }, [])
  useEffect(() => {
    if (intent && consumedIntent.current !== intent) {
      consumedIntent.current = intent
      appendInput(intent)
      setInputNotice(
        lang === 'zh'
          ? '已加入画布意图，可继续补充后发送。'
          : 'Canvas intent added. Review and send when ready.',
      )
      onIntentConsumed()
    } else if (!intent) consumedIntent.current = ''
  }, [intent, onIntentConsumed, appendInput, lang])
  useLayoutEffect(() => {
    const field = inputRef.current
    if (!field || collapsed) return
    field.style.height = '72px'
    field.style.height = `${Math.min(180, Math.max(72, field.scrollHeight))}px`
  }, [input, collapsed])
  useLayoutEffect(() => {
    const log = logRef.current
    if (log && !collapsed && following.current) log.scrollTop = log.scrollHeight
  }, [history, progress, busy, collapsed])
  const generate = async () => {
    if (starting.current || busy || !ready || !input.trim()) return
    starting.current = true
    try {
      if (!(await beforeGenerate())) return
      activeTurn.current = undefined
      setPrompts([])
      setBusy(true)
      ++historySequence.current
      onBusyChange(true)
      setError('')
      setProgress('')
      setActivity('')
      setInputNotice('')
      following.current = true
      setAwayFromBottom(false)
      const text = input
      try {
        const started = await invoke<{ turn_id: string }>('workbench_generate', {
          ...identity,
          action: 'start',
          input: text,
        })
        if (!completedTurns.current.has(started.turn_id)) activeTurn.current = started.turn_id
        setHistory(previous => [...previous, { role: 'user', text }])
        setInput(current => (current === text ? '' : current))
      } catch (reason) {
        showError(reason)
        setBusy(false)
        onBusyChange(false)
      }
    } catch (reason) {
      showError(reason)
    } finally {
      starting.current = false
    }
  }
  const respond = async (actionId: string, response: Record<string, unknown>) => {
    await invoke('workbench_generate', {
      ...identity,
      action: 'respond',
      input: JSON.stringify({ ...response, action_id: actionId, turn_id: activeTurn.current }),
    })
    setPrompts(previous => previous.filter(p => p.action_id !== actionId))
    return 'responded'
  }
  const pending = prompts[0]
  return (
    <aside
      id="wb-assistant-panel"
      className="wb-authoring"
      hidden={collapsed}
      aria-label={ui('工作流助手', 'Workflow assistant')}
    >
      {!collapsed && pending?.action_id && pending.type === 'user_input_request' && (
        <UserInputPrompt
          key={pending.action_id}
          actionId={pending.action_id}
          title={pending.title ?? ''}
          prompt={pending.prompt ?? ''}
          sensitive={pending.sensitive ?? false}
          inputType={pending.input_type ?? 'text'}
          iconPath={pending.icon_path}
          defaultName={pending.default_name}
          defaultShortcut={pending.default_shortcut}
          relX={pending.rel_x}
          relY={pending.rel_y}
          defaultNote={pending.default_note}
          defaultStage={pending.default_stage}
          submitInput={(id, value) => respond(id, { value })}
          rejectInput={id => respond(id, { cancelled: true })}
          onSubmit={() => {}}
          onReject={() => {}}
        />
      )}
      {!collapsed && pending?.action_id && pending.type === 'security_check' && (
        <SecurityPrompt
          key={pending.action_id}
          actionId={pending.action_id}
          tool={pending.tool ?? ''}
          risk={pending.risk ?? 'medium'}
          reason={pending.reason ?? ''}
          decide={async choice => {
            try {
              await respond(pending.action_id!, {
                approved: choice !== 'deny',
                session: choice === 'session',
              })
            } catch (reason) {
              showError(reason)
            }
          }}
          onApprove={() => {}}
          onReject={() => {}}
        />
      )}
      <header className="wb-authoring-header">
        <h2>{ui('工作流助手', 'Workflow assistant')}</h2>
        <button className="wb-authoring-guided" disabled={busy} onClick={() => setGuided(true)}>
          <IconList size={15} aria-hidden="true" />
          {ui('引导填写', 'Guided input')}
        </button>
        {onCollapse && (
          <button
            className="wb-authoring-icon"
            onClick={onCollapse}
            aria-label={ui('收起助手', 'Collapse assistant')}
            title={ui('收起助手', 'Collapse assistant')}
          >
            <IconArrowLeft size={16} aria-hidden="true" />
          </button>
        )}
      </header>
      <div className="wb-authoring-conversation">
        <div
          className="wb-authoring-log"
          ref={logRef}
          role="log"
          aria-live="polite"
          aria-label={ui('生成对话', 'Authoring conversation')}
          onScroll={() => {
            const log = logRef.current
            if (!log) return
            following.current = log.scrollHeight - log.clientHeight - log.scrollTop <= 64
            setAwayFromBottom(!following.current)
          }}
        >
          {!history.length && !busy && (
            <div className="wb-authoring-empty">
              <IconSparkles size={23} aria-hidden="true" />
              <h3>{ui('想自动完成什么？', 'What would you like to automate?')}</h3>
              <p>
                {ui(
                  '直接描述任务，我会编排到右侧画布。',
                  'Describe your task and I will build it on the canvas.',
                )}
              </p>
              <div className="wb-authoring-examples">
                <button
                  onClick={() =>
                    appendInput(
                      ui(
                        '读取项目中的 CSV，逐行处理数据并汇总结果。',
                        'Read a project CSV, process each row, and summarize the results.',
                      ),
                    )
                  }
                >
                  {ui('批量处理表格', 'Process a spreadsheet')}
                </button>
                <button
                  onClick={() =>
                    appendInput(
                      ui(
                        '为当前工作流补充异常处理和重试步骤。',
                        'Add error handling and retry steps to the current workflow.',
                      ),
                    )
                  }
                >
                  {ui('完善现有流程', 'Improve this workflow')}
                </button>
              </div>
            </div>
          )}
          {history.map((message, index) => (
            <article
              className={`wb-authoring-message wb-authoring-message--${message.role === 'user' ? 'user' : 'assistant'}`}
              key={index}
            >
              <strong>{message.role === 'user' ? ui('你', 'You') : 'Nuphus'}</strong>
              <p>{message.text}</p>
            </article>
          ))}
          {busy && (
            <article className="wb-authoring-message wb-authoring-message--assistant">
              <strong>Nuphus</strong>
              <p>{progress || ui('正在准备工作流…', 'Preparing workflow…')}</p>
            </article>
          )}
        </div>
        {awayFromBottom && (
          <button
            className="wb-authoring-latest"
            onClick={() => {
              following.current = true
              setAwayFromBottom(false)
              if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight
            }}
          >
            <IconChevronDown size={14} aria-hidden="true" />
            {ui('回到最新', 'Jump to latest')}
          </button>
        )}
      </div>
      <footer className="wb-authoring-footer">
        {activity && (
          <small className="wb-authoring-activity" role="status">
            {ui('正在调用', 'Using')}: {activity}
          </small>
        )}
        {error && (
          <p className="wb-authoring-error" role="alert">
            {error}
          </p>
        )}
        {initializationFailed && (
          <button
            className="wb-authoring-guided"
            disabled={initializing}
            onClick={() => retryInitialization.current()}
          >
            {initializing ? ui('正在重试…', 'Retrying…') : ui('重试', 'Retry')}
          </button>
        )}
        {inputNotice && (
          <p className="wb-authoring-input-notice" role="status">
            {inputNotice}
          </p>
        )}
        <div className="wb-authoring-composer">
          <label className="wb-authoring-sr-only" htmlFor="wb-generation-input">
            {ui('描述任务或需要修改的地方', 'Describe the task or requested changes')}
          </label>
          <textarea
            id="wb-generation-input"
            ref={inputRef}
            value={input}
            onChange={e => {
              setInput(e.target.value)
              setInputNotice('')
            }}
            onCompositionStart={() => {
              composing.current = true
            }}
            onCompositionEnd={() => {
              composing.current = false
            }}
            onKeyDown={event => {
              if (
                event.key === 'Enter' &&
                (event.ctrlKey || event.metaKey) &&
                !event.nativeEvent.isComposing &&
                !composing.current &&
                event.keyCode !== 229
              ) {
                event.preventDefault()
                void generate()
              }
            }}
            placeholder={ui(
              '描述任务，或告诉我如何修改当前工作流…',
              'Describe a task or how to change this workflow…',
            )}
          />
          <div className="wb-authoring-compose-actions">
            <small>{ui('Ctrl / ⌘ + Enter 发送', 'Ctrl / ⌘ + Enter to send')}</small>
            {busy ? (
              <button
                className="wb-authoring-send"
                disabled={cancelling}
                onClick={() => {
                  setCancelling(true)
                  void invoke('workbench_generate', { ...identity, action: 'cancel' }).catch(
                    reason => {
                      showError(reason)
                      setCancelling(false)
                    },
                  )
                }}
              >
                <IconSquare size={13} aria-hidden="true" />
                {cancelling ? ui('正在停止…', 'Stopping…') : ui('停止', 'Stop')}
              </button>
            ) : (
              <button
                className="wb-authoring-send"
                disabled={!ready || !input.trim()}
                onClick={() => void generate()}
              >
                <IconSend size={15} aria-hidden="true" />
                {ui('发送', 'Send')}
              </button>
            )}
          </div>
        </div>
        <small className="wb-authoring-hint">
          {ui(
            '助手可编辑和试运行工作流，进度与结果会显示在对话中。',
            'The assistant can edit and test workflows. Follow progress and results in the conversation.',
          )}
        </small>
      </footer>
      {guided &&
        createPortal(
          <IntentFormPanel
            initialName={draft.document.name}
            workflowId={draft.workflow_id}
            draftScope={`workbench:${draft.project_id}`}
            submitLabel={ui('填入描述', 'Add to description')}
            onClose={() => setGuided(false)}
            onSubmit={form => {
              appendInput(
                buildIntentTextTemplate(form, draft.workflow_id, draft.document.name, {
                  target: 'workbench',
                }),
              )
              setInputNotice(
                ui(
                  '已加入引导内容，可继续补充后发送。',
                  'Guided details added. Review and send when ready.',
                ),
              )
              setGuided(false)
              return true
            }}
          />,
          document.body,
        )}
    </aside>
  )
}
