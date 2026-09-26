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
  event: { type: string; text?: string; name?: string; success?: boolean }
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
  const refreshHistory = useCallback(async () => {
    const sequence = ++historySequence.current
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
    setBusy(result.busy)
    callbacks.current.onBusyChange(result.busy)
  }, [draft.project_id, draft.workflow_id])
  useEffect(() => {
    let alive = true
    let initializingNow = false
    let stopListening: (() => void) | undefined
    mounted.current = true
    const onProgress = (payload: Progress) => {
      if (
        !alive ||
        payload.project_id !== draft.project_id ||
        payload.workflow_id !== draft.workflow_id
      )
        return
      const event = payload.event
      if (event.type === 'progress')
        setProgress(previous =>
          previous ? `${previous}\n\n${event.text ?? ''}` : (event.text ?? ''),
        )
      if (event.type === 'tool') setActivity(event.name ?? '')
      if (event.type === 'error') setError(event.text ?? '')
      if (event.type === 'completed') {
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
        await invoke('workbench_generate', { ...identity, action: 'start', input: text })
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
  return (
    <aside
      id="wb-assistant-panel"
      className="wb-authoring"
      hidden={collapsed}
      aria-label={ui('工作流助手', 'Workflow assistant')}
    >
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
            '生成后可在画布检查，不会自动运行。',
            'Review the canvas before running. Generation does not run it.',
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
