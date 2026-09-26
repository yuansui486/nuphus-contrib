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
  const { lang } = useLanguage()
  const ui = useCallback((zh: string, en: string) => (lang === 'zh' ? zh : en), [lang])
  const { toggleTheme } = useTheme()
  const [projects, setProjects] = useState<Project[]>([])
  const [projectId, setProjectId] = useState('')
  const [drafts, setDrafts] = useState<Draft[]>([])
  const [active, setActive] = useState<Draft | null>(null)
  const [epoch, setEpoch] = useState(0)
  const [runs, setRuns] = useState<Run[]>([])
  const [notice, setNotice] = useState('')
  const [search, setSearch] = useState('')
  const [creating, setCreating] = useState(false)
  const [page, setPage] = useState<'workflows' | 'clients' | 'models' | 'runs'>('workflows')
  const [endpoint, setEndpoint] = useState<Endpoint | null>(null)
  const [conflict, setConflict] = useState(false)
  const [intent, setIntent] = useState('')
  const [generationBusy, setGenerationBusy] = useState(false)
  const creatingRef = useRef(false)
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
    setDrafts([])
    setRuns([])
  }, [projectId])

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
    ++refreshSequence.current
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
    ++refreshSequence.current
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
    if (!projectId || creatingRef.current) return
    const requestedProject = projectId
    creatingRef.current = true
    setCreating(true)
    try {
      const draft = await call<Draft>('canvas.create', {
        project_id: requestedProject,
        name: ui('未命名工作流', 'Untitled workflow'),
      })
      // A slow create must not switch the user back to a project they left.
      if (projectRef.current !== requestedProject) return
      openDraft(draft)
      await refresh()
    } catch (error) {
      fail(error)
    } finally {
      creatingRef.current = false
      setCreating(false)
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
  const loadEndpoint = useCallback(async () => {
    setEndpoint(await clients<Endpoint>('status'))
  }, [])
  useEffect(() => {
    if (page !== 'clients') return
    void loadEndpoint().catch(fail)
    const timer = setInterval(() => void loadEndpoint().catch(fail), 2000)
    return () => clearInterval(timer)
  }, [page, loadEndpoint, fail])
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
                disabled={!projectId || creating}
                onClick={() => void create()}
                title={ui(
                  '新建空白工作流并直接在画布中编排',
                  'Create a blank workflow and open the canvas',
                )}
              >
                {creating ? ui('创建中…', 'Creating…') : ui('画布新建', 'New canvas')}
              </button>
            </header>
            <input
              aria-label={ui('搜索工作流', 'Search workflows')}
              placeholder={ui('搜索工作流…', 'Search workflows…')}
              value={search}
              onChange={e => setSearch(e.target.value)}
            />
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
                '本机端口随工作台自动启动，无需创建客户端或配置令牌。外部 Agent 可直接使用全部项目、画布、工作流和自动化能力。',
                'The local endpoint starts with Workbench. No client registration or token is needed; all projects, canvas, workflow and automation capabilities are available.',
              )}
            </p>
            <p role="status">
              {endpoint?.status === 'listening'
                ? ui('服务已启动 · 全部权限', 'Service running · Full access')
                : (endpoint?.message ?? ui('正在启动服务…', 'Starting service…'))}
            </p>
            {endpoint?.status === 'listening' && (
              <div className="wb-connections">
                {[
                  ['MCP', endpoint.mcp_url],
                  ['HTTP API', endpoint.url ? `${endpoint.url}/api/v1` : undefined],
                ].map(
                  ([label, url]) =>
                    url && (
                      <div className="wb-row" key={label}>
                        <div>
                          <strong>{label}</strong>
                          <code>{url}</code>
                        </div>
                        <button onClick={() => void navigator.clipboard.writeText(url).catch(fail)}>
                          {ui('复制地址', 'Copy address')}
                        </button>
                      </div>
                    ),
                )}
                <p>
                  {ui(
                    'MCP 客户端填写上面的地址即可连接，无需请求头。HTTP 可先 GET /api/v1/discover 查看接口。',
                    'Connect your MCP client using the address above, without authentication headers. For HTTP, start with GET /api/v1/discover.',
                  )}
                </p>
              </div>
            )}
            <p>
              {ui(
                '仅监听 127.0.0.1，拒绝网页来源请求。连接的本机程序拥有全部能力，执行可能操作真实应用和文件；退出工作台后端口关闭。外部 Agent 使用自己的模型，不需要配置内置模型。',
                'Only 127.0.0.1 is exposed and browser-origin requests are rejected. Local programs have full access and can operate real applications and files. Quitting Workbench closes the endpoint. External Agents use their own models.',
              )}
            </p>
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
