import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { useLanguage } from '../locales'
import { IntentFormPanel } from '../main-window/workflow-canvas/IntentFormPanel'
import { buildIntentTextTemplate } from '../main-window/workflow-canvas/intentText'
import type { Draft } from './api'

interface Props {
  draft: Draft
  intent: string
  onIntentConsumed: () => void
  onBusyChange: (busy: boolean) => void
  onChanged: () => void
  beforeGenerate: () => Promise<boolean>
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
  const [history, setHistory] = useState<Array<{ role: string; text: string }>>([])
  const [progress, setProgress] = useState('')
  const [activity, setActivity] = useState('')
  const [error, setError] = useState('')
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
    const result = await invoke<History>('workbench_generate', {
      projectId: draft.project_id,
      workflowId: draft.workflow_id,
      action: 'history',
    })
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
    const unlisten = listen<Progress>('workbench-authoring', ({ payload }) => {
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
        callbacks.current.onBusyChange(false)
        setActivity('')
        setProgress(previous => [previous, event.text].filter(Boolean).join('\n\n'))
        if (!event.success) setError(event.text ?? '')
        callbacks.current.onChanged()
        void refreshHistory().catch(showError)
      }
    })
    void unlisten
      .then(() => {
        if (alive) setReady(true)
      })
      .catch(showError)
    void refreshHistory().catch(showError)
    return () => {
      alive = false
      void unlisten.then(stop => stop())
      callbacks.current.onBusyChange(false)
    }
  }, [draft.project_id, draft.workflow_id, refreshHistory])
  useEffect(() => {
    if (intent) {
      setInput(intent)
      onIntentConsumed()
    }
  }, [intent, onIntentConsumed])
  const generate = async () => {
    if (busy || !ready || !input.trim() || !(await beforeGenerate())) return
    setBusy(true)
    onBusyChange(true)
    setError('')
    setProgress('')
    setActivity('')
    const text = input
    try {
      await invoke('workbench_generate', { ...identity, action: 'start', input: text })
      setHistory(previous => [...previous, { role: 'user', text }])
      setInput('')
    } catch (reason) {
      showError(reason)
      setBusy(false)
      onBusyChange(false)
    }
  }
  return (
    <aside className="wb-authoring" aria-label={ui('工作流生成', 'Workflow authoring')}>
      <h2>{ui('内置生成', 'Internal generation')}</h2>
      <p>
        {ui(
          '使用工作流模型生成或修改当前画布，不会自动运行。手工修改和外部接入不需要模型密钥。',
          'Use your workflow model to edit this canvas, without automatically running it. Manual editing and external authoring do not need a model key.',
        )}
      </p>
      <div className="wb-actions">
        <button disabled={busy} onClick={() => setGuided(true)}>
          {ui('引导式生成', 'Guided generation')}
        </button>
        <button
          disabled={busy}
          onClick={() => document.getElementById('wb-generation-input')?.focus()}
        >
          {ui('一句话生成', 'Describe a task')}
        </button>
      </div>
      <div className="wb-authoring-log" aria-live="polite">
        {history.map((message, index) => (
          <article key={index}>
            <strong>{message.role === 'user' ? ui('你', 'You') : 'Nuphus'}</strong>
            <p>{message.text}</p>
          </article>
        ))}
        {busy && (
          <article>
            <strong>Nuphus</strong>
            <p>{progress || ui('正在准备工作流…', 'Preparing workflow…')}</p>
          </article>
        )}
      </div>
      {activity && (
        <small role="status">
          {ui('正在调用', 'Using')}: {activity}
        </small>
      )}
      {error && <p role="alert">{error}</p>}
      <label htmlFor="wb-generation-input">
        {ui('描述任务或需要修改的地方', 'Describe the task or requested changes')}
      </label>
      <textarea
        id="wb-generation-input"
        value={input}
        onChange={e => setInput(e.target.value)}
        placeholder={ui(
          '例如：读取项目中的 CSV，为每行执行指定操作…',
          'For example: read a project CSV and process each row…',
        )}
      />
      {busy ? (
        <button
          onClick={() => {
            void invoke('workbench_generate', { ...identity, action: 'cancel' }).catch(showError)
          }}
        >
          {ui('停止生成', 'Stop generation')}
        </button>
      ) : (
        <button disabled={!ready || !input.trim()} onClick={() => void generate()}>
          {ui('生成 / 修改当前工作流', 'Generate / edit workflow')}
        </button>
      )}
      {guided && (
        <IntentFormPanel
          initialName={draft.document.name}
          workflowId={draft.workflow_id}
          onClose={() => setGuided(false)}
          onSubmit={form => {
            setInput(buildIntentTextTemplate(form, draft.workflow_id, draft.document.name))
            setGuided(false)
            return true
          }}
        />
      )}
    </aside>
  )
}
