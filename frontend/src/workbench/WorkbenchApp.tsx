import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { TitleBar } from '../main-window/layout/TitleBar'
import { CanvasPage } from '../main-window/workflow-canvas/CanvasPage'
import { CanvasBackendContext } from '../main-window/workflow-canvas/CanvasBackend'
import { useCanvasLeaveGuard } from '../main-window/workflow-canvas/useCanvasLeaveGuard'
import { invoke } from '@tauri-apps/api/core'
import { useLanguage } from '../locales'
import { useTheme } from '../hooks/useTheme'
import {
  call,
  canvasBackend,
  clients,
  type Client,
  type Draft,
  type Endpoint,
  type Project,
  type Run,
} from './api'
import { AuthoringPanel } from './AuthoringPanel'
import './workbench.css'

const ModelsPage = lazy(() =>
  import('../main-window/pages/ModelsPage').then(m => ({ default: m.ModelsPage })),
)
const terminal = (status: string) =>
  ['completed', 'failed', 'cancelled', 'interrupted'].includes(status)

export default function WorkbenchApp() {
  const { lang, setLang } = useLanguage()
  const ui = useCallback((zh: string, en: string) => (lang === 'zh' ? zh : en), [lang])
  const { toggleTheme } = useTheme()
  const [projects, setProjects] = useState<Project[]>([])
  const [projectId, setProjectId] = useState('')
  const [drafts, setDrafts] = useState<Draft[]>([])
  const [active, setActive] = useState<Draft | null>(null)
  const [epoch, setEpoch] = useState(0)
  const [runs, setRuns] = useState<Run[]>([])
  const [notice, setNotice] = useState('')
  const [name, setName] = useState('')
  const [search, setSearch] = useState('')
  const [creating, setCreating] = useState(false)
  const [page, setPage] = useState<'workflows' | 'clients' | 'models' | 'runs'>('workflows')
  const [clientList, setClientList] = useState<Client[]>([])
  const [endpoint, setEndpoint] = useState<Endpoint | null>(null)
  const [clientName, setClientName] = useState('')
  const [clientCaps, setClientCaps] = useState(['read', 'edit', 'run'])
  const [token, setToken] = useState('')
  const [conflict, setConflict] = useState(false)
  const [intent, setIntent] = useState('')
  const [generationBusy, setGenerationBusy] = useState(false)
  const [busy, setBusy] = useState(false)
  const dirty = useRef(false)
  const activeRef = useRef(active)
  activeRef.current = active
  const projectRef = useRef(projectId)
  projectRef.current = projectId
  const refreshSequence = useRef(0)
  const generationRef = useRef(generationBusy)
  generationRef.current = generationBusy
  const { register, leave } = useCanvasLeaveGuard()
  const fail = useCallback((error: unknown) => setNotice(String(error)), [])

  useEffect(() => {
    let alive = true
    call<Project[]>('project.list')
      .then(items => {
        if (!alive) return
        setProjects(items)
        setProjectId(items[0]?.project_id ?? '')
      })
      .catch(fail)
    void invoke('finish_startup').catch(fail)
    return () => {
      alive = false
    }
  }, [fail])

  const refresh = useCallback(async () => {
    if (!projectId) return
    const sequence = ++refreshSequence.current
    const [nextDrafts, nextRuns] = await Promise.all([
      call<Draft[]>('workflow.list', { project_id: projectId }),
      call<Run[]>('run.list', { project_id: projectId }),
    ])
    if (projectRef.current !== projectId || sequence !== refreshSequence.current) return
    setDrafts(nextDrafts)
    setRuns(nextRuns)
    const current = activeRef.current
    if (!current || current.project_id !== projectId) return
    const next = nextDrafts.find(draft => draft.workflow_id === current.workflow_id)
    if (
      !next ||
      next.revision !== current.revision ||
      next.layout_revision !== current.layout_revision
    ) {
      if (dirty.current || generationRef.current) setConflict(true)
      else if (next) {
        setActive(next)
        setEpoch(value => value + 1)
        setConflict(false)
      } else {
        setActive(null)
        setNotice(
          ui(
            '此工作流已被外部客户端删除；历史版本和运行记录仍保留。',
            'This draft was deleted by another client; versions and run evidence are retained.',
          ),
        )
      }
    }
  }, [projectId, ui])
  useEffect(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout>
    const poll = async () => {
      try {
        if (!stopped) await refresh()
      } catch (error) {
        if (!stopped) fail(error)
      }
      if (!stopped) timer = setTimeout(() => void poll(), 2000)
    }
    void poll()
    return () => {
      stopped = true
      clearTimeout(timer)
    }
  }, [refresh, fail])

  const openDraft = useCallback((draft: Draft) => {
    setActive(draft)
    setProjectId(draft.project_id)
    setPage('workflows')
    setConflict(false)
    dirty.current = false
    setEpoch(value => value + 1)
  }, [])
  const navigate = useCallback(
    (action: () => void) => {
      if (generationRef.current) {
        setNotice(
          ui('请先停止当前生成，再切换画布。', 'Stop generation before switching canvases.'),
        )
        return
      }
      void leave(action)
    },
    [leave, ui],
  )
  useEffect(() => {
    const subscription = listen<{ project_id: string; workflow_id: string }>(
      'workbench-open',
      event => {
        navigate(() => {
          void call<Draft>('canvas.get', event.payload).then(openDraft).catch(fail)
        })
      },
    )
    return () => {
      void subscription.then(unlisten => unlisten())
    }
  }, [navigate, openDraft, fail])

  const editorState = useCallback(
    (state: { dirty: boolean; selection: string[]; layerId: string }) => {
      dirty.current = state.dirty
      const current = activeRef.current
      if (current)
        void invoke('workbench_view_state', {
          projectId: current.project_id,
          workflowId: current.workflow_id,
          view: { open: true, ...state },
        }).catch(fail)
    },
    [fail],
  )
  useEffect(() => {
    if (!active) return
    return () => {
      void invoke('workbench_view_state', {
        projectId: active.project_id,
        workflowId: active.workflow_id,
        view: { open: false, selection: null },
      }).catch(fail)
    }
  }, [active?.workflow_id, active?.project_id, fail])
  const saved = useCallback((draft: Draft, kind?: 'layout') => {
    const next =
      kind === 'layout' && activeRef.current
        ? { ...activeRef.current, layout: draft.layout, layout_revision: draft.layout_revision }
        : draft
    activeRef.current = next
    setActive(next)
    if (kind !== 'layout') setConflict(false)
  }, [])
  const started = useCallback(
    (id: string) => {
      setNotice(`${ui('已启动，运行编号', 'Started; run ID')}: ${id}`)
      void refresh().catch(fail)
    },
    [refresh, fail, ui],
  )
  const startedRef = useRef(started)
  startedRef.current = started
  // Deliberately tied to a mounted editor, not each poll or save result.
  const backend = useMemo(
    () => (active ? canvasBackend(active, saved, id => startedRef.current(id)) : null),
    [epoch, active?.workflow_id, saved],
  )
  const create = async () => {
    if (!name.trim() || busy) return
    setBusy(true)
    try {
      const draft = await call<Draft>('canvas.create', { project_id: projectId, name: name.trim() })
      setName('')
      setCreating(false)
      openDraft(draft)
      await refresh()
    } catch (error) {
      fail(error)
    } finally {
      setBusy(false)
    }
  }
  const chooseProject = async () => {
    const directory = await open({ directory: true, multiple: false })
    if (typeof directory !== 'string') return
    const project = await call<Project>('project.register', {
      directory,
      name: directory.split(/[\\/]/).filter(Boolean).slice(-1)[0] || 'Workspace',
    })
    setProjects(await call<Project[]>('project.list'))
    setProjectId(project.project_id)
    setActive(null)
  }
  const loadClients = useCallback(async () => {
    const [items, status] = await Promise.all([
      clients<Client[]>('list'),
      clients<Endpoint>('status'),
    ])
    setClientList(items)
    setEndpoint(status)
  }, [])
  useEffect(() => {
    if (page === 'clients') void loadClients().catch(fail)
  }, [page, loadClients, fail])
  const createClient = async () => {
    setBusy(true)
    try {
      const result = await clients<{ token: string }>('create', {
        name: clientName,
        projects: [projectId],
        capabilities: clientCaps,
      })
      setToken(result.token)
      setClientName('')
      await loadClients()
    } catch (error) {
      fail(error)
    } finally {
      setBusy(false)
    }
  }
  const mode = async (value: 'internal' | 'external') => {
    if (!active || generationBusy) return
    navigate(() => {
      void call<{ draft: Draft }>('canvas.update', {
        project_id: projectId,
        workflow_id: active.workflow_id,
        revision: activeRef.current?.revision,
        operations: [{ op: 'set_authoring_mode', mode: value }],
      })
        .then(result => openDraft(result.draft))
        .catch(fail)
    })
  }

  return (
    <div className="wb-app">
      <TitleBar brand="Nuphus Workbench" />
      <nav className="wb-nav" aria-label={ui('工作台导航', 'Workbench navigation')}>
        <select
          aria-label={ui('当前项目', 'Current project')}
          value={projectId}
          onChange={event =>
            navigate(() => {
              setProjectId(event.target.value)
              setActive(null)
            })
          }
        >
          {projects.map(project => (
            <option key={project.project_id} value={project.project_id}>
              {project.name}
            </option>
          ))}
        </select>
        <button onClick={() => navigate(() => void chooseProject().catch(fail))}>
          {ui('打开项目', 'Open project')}
        </button>
        {(['workflows', 'runs', 'clients', 'models'] as const).map((item, index) => (
          <button
            key={item}
            aria-current={page === item ? 'page' : undefined}
            onClick={() =>
              navigate(() => {
                setPage(item)
                setActive(null)
                setToken('')
              })
            }
          >
            {
              [
                ui('工作流', 'Workflows'),
                ui('运行记录', 'Runs'),
                ui('外部接入', 'Connections'),
                ui('模型配置', 'Models'),
              ][index]
            }
          </button>
        ))}
        <button className="wb-theme" onClick={toggleTheme}>
          {ui('切换主题', 'Toggle theme')}
        </button>
        <button onClick={() => setLang(lang === 'zh' ? 'en' : 'zh')}>
          {lang === 'zh' ? 'English' : '中文'}
        </button>
      </nav>
      {notice && (
        <div className="wb-notice" role="status">
          <span>{notice}</span>
          <button onClick={() => setNotice('')} aria-label={ui('关闭提示', 'Dismiss notice')}>
            ×
          </button>
        </div>
      )}
      <main className="wb-main">
        {page === 'models' && (
          <Suspense fallback={<p>{ui('正在加载…', 'Loading…')}</p>}>
            <ModelsPage onClose={() => setPage('workflows')} />
          </Suspense>
        )}
        {page === 'workflows' && !active && (
          <section className="wb-dashboard">
            <header>
              <div>
                <h1>{ui('工作流', 'Workflows')}</h1>
                <p>
                  {ui(
                    '在画布编排，或让外部 Agent 接入。运行与版本由本机工作台管理。',
                    'Build on the canvas or connect an external Agent. This local workbench manages versions and execution.',
                  )}
                </p>
              </div>
              <button
                className="wb-primary"
                disabled={!projectId}
                onClick={() => setCreating(true)}
              >
                {ui('新建工作流', 'New workflow')}
              </button>
            </header>
            <input
              aria-label={ui('搜索工作流', 'Search workflows')}
              placeholder={ui('搜索工作流…', 'Search workflows…')}
              value={search}
              onChange={e => setSearch(e.target.value)}
            />
            {creating && (
              <form
                className="wb-create"
                onSubmit={event => {
                  event.preventDefault()
                  void create()
                }}
              >
                <label>
                  {ui('工作流名称', 'Workflow name')}
                  <input autoFocus required value={name} onChange={e => setName(e.target.value)} />
                </label>
                <button type="submit" disabled={busy || !name.trim()}>
                  {ui('创建并打开画布', 'Create and open canvas')}
                </button>
                <button type="button" onClick={() => setCreating(false)}>
                  {ui('取消', 'Cancel')}
                </button>
              </form>
            )}
            {!drafts.length && (
              <div className="wb-empty">
                <h2>{ui('从一个工作流开始', 'Start with a workflow')}</h2>
                <p>
                  {ui(
                    '手工编辑或外部接入不需要模型密钥。仅在使用内置生成、AI 节点时配置模型。',
                    'Manual editing and external authoring need no model key. Configure a model when using internal generation or AI steps.',
                  )}
                </p>
              </div>
            )}
            <div className="wb-grid">
              {drafts
                .filter(draft => draft.document.name.toLowerCase().includes(search.toLowerCase()))
                .map(draft => (
                  <button
                    className="wb-card"
                    key={draft.workflow_id}
                    onClick={() => openDraft(draft)}
                  >
                    <h2>{draft.document.name}</h2>
                    <p>
                      {draft.authoring_mode === 'internal'
                        ? ui('内置生成', 'Internal authoring')
                        : ui('外部编排', 'External authoring')}
                    </p>
                    <small>
                      {ui('修订', 'Revision')} {draft.revision} · {draft.document.steps.length}{' '}
                      {ui('个顶层节点', 'top-level steps')}
                    </small>
                  </button>
                ))}
            </div>
          </section>
        )}
        {page === 'workflows' && active && backend && (
          <section className="wb-editor">
            <div className="wb-editor-mode">
              <span>{ui('编排方式', 'Authoring')}</span>
              <select
                value={active.authoring_mode}
                disabled={generationBusy}
                onChange={e => void mode(e.target.value as 'internal' | 'external')}
              >
                <option value="internal">{ui('内置生成', 'Internal generation')}</option>
                <option value="external">
                  {ui('外部 Agent / 手工', 'External Agent / manual')}
                </option>
              </select>
              <small>
                {ui(
                  '切换仅改变生成入口，不删除画布内容。',
                  'Switching changes the authoring panel, not canvas content.',
                )}
              </small>
            </div>
            {conflict && (
              <div className="wb-conflict" role="alert">
                {ui(
                  '外部客户端已修改画布。你的编辑尚未覆盖新版本；保存时会检查冲突。',
                  'Another client changed this canvas. Your edits have not overwritten it; saves check revision conflicts.',
                )}
                <button
                  onClick={() =>
                    navigate(() => {
                      void call<Draft>('canvas.get', {
                        project_id: projectId,
                        workflow_id: active.workflow_id,
                      })
                        .then(openDraft)
                        .catch(fail)
                    })
                  }
                >
                  {ui('检查未保存编辑并重新加载', 'Review unsaved edits and reload')}
                </button>
              </div>
            )}
            <div className="wb-editor-body">
              {active.authoring_mode === 'internal' && (
                <AuthoringPanel
                  key={`${active.project_id}:${active.workflow_id}`}
                  draft={active}
                  intent={intent}
                  onIntentConsumed={() => setIntent('')}
                  onBusyChange={setGenerationBusy}
                  onChanged={() => {
                    void refresh().catch(fail)
                  }}
                  beforeGenerate={async () =>
                    !dirty.current ||
                    (setNotice(
                      ui('请先保存画布编辑再生成。', 'Save canvas edits before generating.'),
                    ),
                    false)
                  }
                />
              )}
              <div className="wb-canvas">
                <CanvasBackendContext.Provider value={backend}>
                  <CanvasPage
                    key={`${active.workflow_id}:${epoch}`}
                    workflowId={active.workflow_id}
                    onClose={() => navigate(() => setActive(null))}
                    registerLeaveGuard={register}
                    onSwitchWorkflow={id => {
                      const next = drafts.find(draft => draft.workflow_id === id)
                      if (next) openDraft(next)
                    }}
                    onEditorState={editorState}
                    onGenerateIntent={setIntent}
                  />
                </CanvasBackendContext.Provider>
              </div>
            </div>
          </section>
        )}
        {page === 'clients' && (
          <section className="wb-dashboard">
            <h1>{ui('外部接入', 'External connections')}</h1>
            <p>
              {ui(
                '为当前项目创建访问令牌。外部 Agent 使用自己的模型；不会自动使用本机模型配置。',
                'Create a token for the selected project. External Agents use their own models; connecting does not invoke the internal Agent.',
              )}
            </p>
            <code>
              {endpoint?.mcp_url ??
                endpoint?.message ??
                ui('正在读取服务状态…', 'Reading service status…')}
            </code>
            <form
              className="wb-create"
              onSubmit={event => {
                event.preventDefault()
                void createClient()
              }}
            >
              <label>
                {ui('客户端名称', 'Client name')}
                <input required value={clientName} onChange={e => setClientName(e.target.value)} />
              </label>
              <fieldset>
                <legend>{ui('允许能力', 'Capabilities')}</legend>
                {['read', 'edit', 'run', 'respond', 'automation'].map(cap => (
                  <label key={cap}>
                    <input
                      type="checkbox"
                      checked={clientCaps.includes(cap)}
                      onChange={e =>
                        setClientCaps(values =>
                          e.target.checked
                            ? [...values, cap]
                            : values.filter(value => value !== cap),
                        )
                      }
                    />
                    {cap}
                  </label>
                ))}
              </fieldset>
              <button disabled={!projectId || !clientName.trim() || busy}>
                {ui('创建令牌', 'Create token')}
              </button>
            </form>
            {token && (
              <div className="wb-token">
                <p>
                  {ui(
                    '仅此时显示，请保存到客户端的安全配置中，不要放进工作流。',
                    'Shown only now. Save in your client’s secure configuration, not in a workflow.',
                  )}
                </p>
                <input
                  aria-label={ui('新令牌', 'New token')}
                  readOnly
                  value={token}
                  onFocus={e => e.target.select()}
                />
                <button
                  onClick={() => {
                    void navigator.clipboard.writeText(token).catch(fail)
                  }}
                >
                  {ui('复制', 'Copy')}
                </button>
                <button onClick={() => setToken('')}>{ui('隐藏', 'Hide')}</button>
              </div>
            )}
            {clientList.map(client => (
              <div className="wb-row" key={client.client_id}>
                <div>
                  <strong>{client.name}</strong>
                  <p>{client.capabilities.join(' · ')}</p>
                </div>
                <button
                  disabled={client.revoked}
                  onClick={() => {
                    void clients('revoke', { client_id: client.client_id })
                      .then(loadClients)
                      .catch(fail)
                  }}
                >
                  {client.revoked ? ui('已撤销', 'Revoked') : ui('撤销', 'Revoke')}
                </button>
              </div>
            ))}
          </section>
        )}
        {page === 'runs' && (
          <section className="wb-dashboard">
            <h1>{ui('运行记录', 'Runs')}</h1>
            <p>
              {ui(
                '关闭连接或隐藏窗口不会取消执行。取消会等待正在进行的本地动作结束。',
                'Disconnecting or hiding the window does not cancel a run. Cancellation waits for an in-flight local action to finish.',
              )}
            </p>
            {runs.map(run => (
              <article className="wb-run" key={run.run_id}>
                <div className="wb-row">
                  <strong>
                    {drafts.find(draft => draft.workflow_id === run.workflow_id)?.document.name ??
                      run.workflow_id}
                  </strong>
                  <span>{run.status}</span>
                </div>
                <code>{run.run_id}</code>
                {!terminal(run.status) && (
                  <div className="wb-actions">
                    {(run.status === 'running' || run.status === 'paused') && (
                      <button
                        onClick={() => {
                          void call(run.status === 'paused' ? 'run.resume' : 'run.pause', {
                            project_id: projectId,
                            run_id: run.run_id,
                          })
                            .then(refresh)
                            .catch(fail)
                        }}
                      >
                        {run.status === 'paused' ? ui('继续', 'Resume') : ui('暂停', 'Pause')}
                      </button>
                    )}
                    <button
                      onClick={() => {
                        void call('run.cancel', { project_id: projectId, run_id: run.run_id })
                          .then(refresh)
                          .catch(fail)
                      }}
                    >
                      {ui('取消运行', 'Cancel run')}
                    </button>
                  </div>
                )}
                {run.pending_request && (
                  <div className="wb-conflict">
                    <p>{run.pending_request.prompt}</p>
                    <button
                      onClick={() => {
                        void call('run.respond', {
                          project_id: projectId,
                          run_id: run.run_id,
                          request_id: run.pending_request?.request_id,
                          decision: 'continue',
                        })
                          .then(refresh)
                          .catch(fail)
                      }}
                    >
                      {ui('确认并继续', 'Confirm and continue')}
                    </button>
                  </div>
                )}
                <details>
                  <summary>{ui('输入与结果', 'Inputs and result')}</summary>
                  <pre>{JSON.stringify({ inputs: run.inputs, result: run.result }, null, 2)}</pre>
                </details>
              </article>
            ))}
          </section>
        )}
      </main>
    </div>
  )
}
