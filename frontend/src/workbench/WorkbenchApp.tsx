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
  IconArrowLeft as ArrowLeft,
  IconPlug as Cable,
  IconHistory as History,
  IconLayoutDashboard as LayoutDashboard,
  IconPlus as Plus,
  IconSettings as Settings,
  IconSparkles as Sparkles,
} from '../ui/Icons'
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
import { WorkbenchSettings } from './WorkbenchSettings'
import { ExternalConnections } from './ExternalConnections'
import { ScheduleControl, ScheduleHistory, type ScheduleSummary } from './ScheduleControls'
import './workbench.css'
import { LingqueLogo } from './LingqueLogo'

const ModelsPage = lazy(() =>
  import('../main-window/pages/ModelsPage').then(m => ({ default: m.ModelsPage })),
)
const terminal = (status: string) =>
  ['completed', 'failed', 'cancelled', 'interrupted', 'skipped'].includes(status)

export default function WorkbenchApp({ storageScope }: { storageScope?: string } = {}) {
  const projectStorageKey = storageScope
    ? `workbench:${storageScope}:last-project`
    : 'workbench:last-project'
  const { lang } = useLanguage()
  const ui = useCallback((zh: string, en: string) => (lang === 'zh' ? zh : en), [lang])
  const { toggleTheme } = useTheme()
  const [projects, setProjects] = useState<Project[]>([])
  const [projectId, setProjectId] = useState('')
  const [drafts, setDrafts] = useState<Draft[]>([])
  const [active, setActive] = useState<Draft | null>(null)
  const [epoch, setEpoch] = useState(0)
  const [runs, setRuns] = useState<Run[]>([])
  const [schedules, setSchedules] = useState<ScheduleSummary[]>([])
  const [scheduledOnly, setScheduledOnly] = useState(false)
  const [notice, setNotice] = useState('')
  const [search, setSearch] = useState('')
  const [creating, setCreating] = useState(false)
  const [page, setPage] = useState<'workflows' | 'clients' | 'models' | 'runs'>('workflows')
  const [endpoint, setEndpoint] = useState<Endpoint | null>(null)
  const [conflict, setConflict] = useState(false)
  const [intent, setIntent] = useState('')
  const [generationBusy, setGenerationBusy] = useState(false)
  const [assistantOpen, setAssistantOpen] = useState(true)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const creatingRef = useRef(false)
  const dirty = useRef(false)
  const activeRef = useRef(active)
  activeRef.current = active
  const projectRef = useRef(projectId)
  projectRef.current = projectId
  const refreshSequence = useRef(0)
  const navigationSequence = useRef(0)
  const generationRef = useRef(generationBusy)
  generationRef.current = generationBusy
  const { register, leave } = useCanvasLeaveGuard()
  const fail = useCallback((error: unknown) => setNotice(String(error)), [])
  useEffect(() => {
    setDrafts([])
    setRuns([])
    setSchedules([])
    if (projectId) {
      try {
        localStorage.setItem(projectStorageKey, projectId)
      } catch {
        /* Storage is optional. */
      }
    }
  }, [projectId, projectStorageKey])

  useEffect(() => {
    let alive = true
    call<Project[]>('project.list')
      .then(items => {
        if (!alive) return
        setProjects(items)
        let previous: string | null = null
        try {
          previous = localStorage.getItem(projectStorageKey)
        } catch {
          /* Storage is optional. */
        }
        const selected = items.find(item => item.project_id === previous)
        setProjectId(selected?.project_id ?? items[0]?.project_id ?? '')
        if (previous && !selected)
          setNotice(
            ui(
              '上次的项目不可用，已切换到可用项目。',
              'The previous project is unavailable; another project was selected.',
            ),
          )
      })
      .catch(fail)
    void invoke('finish_startup').catch(fail)
    return () => {
      alive = false
    }
  }, [fail, ui, projectStorageKey])

  const refresh = useCallback(async () => {
    if (!projectId) return
    const sequence = ++refreshSequence.current
    const [nextDrafts, nextRuns, nextSchedules] = await Promise.all([
      call<Draft[]>('workflow.list', { project_id: projectId }),
      call<Run[]>('run.list', { project_id: projectId }),
      call<ScheduleSummary[]>('workflow.schedule.list', { project_id: projectId }),
    ])
    if (projectRef.current !== projectId || sequence !== refreshSequence.current) return
    setDrafts(nextDrafts)
    setRuns(nextRuns)
    setSchedules(nextSchedules ?? [])
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
    ++navigationSequence.current
    ++refreshSequence.current
    setActive(draft)
    setProjectId(draft.project_id)
    setPage('workflows')
    setConflict(false)
    setAssistantOpen(true)
    setSettingsOpen(false)
    setIntent('')
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
      void leave(() => {
        ++navigationSequence.current
        action()
      })
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
    const navigation = navigationSequence.current
    creatingRef.current = true
    setCreating(true)
    try {
      const draft = await call<Draft>('canvas.create', {
        project_id: requestedProject,
        name: ui('未命名工作流', 'Untitled workflow'),
      })
      // Persist the result, but never reopen it after the user navigated elsewhere.
      if (projectRef.current !== requestedProject || navigationSequence.current !== navigation)
        return
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
    const navigation = navigationSequence.current
    const directory = await open({ directory: true, multiple: false })
    if (typeof directory !== 'string') return
    const project = await call<Project>('project.register', {
      directory,
      name: directory.split(/[\\/]/).filter(Boolean).slice(-1)[0] || 'Workspace',
    })
    setProjects(await call<Project[]>('project.list'))
    if (navigationSequence.current !== navigation) return
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
      const navigation = navigationSequence.current
      void call<{ draft: Draft }>('canvas.update', {
        project_id: projectId,
        workflow_id: active.workflow_id,
        revision: activeRef.current?.revision,
        operations: [{ op: 'set_authoring_mode', mode: value }],
      })
        .then(result => {
          if (navigationSequence.current !== navigation) return
          if (dirty.current || generationRef.current) {
            setConflict(true)
            return
          }
          openDraft(result.draft)
        })
        .catch(fail)
    })
  }

  const switchPage = (value: typeof page) =>
    navigate(() => {
      setSettingsOpen(false)
      setPage(value)
      setActive(null)
    })
  const selectedProject = projects.find(item => item.project_id === projectId)
  const statusText = (status: string) =>
    ({
      queued: ui('排队中', 'Queued'),
      running: ui('运行中', 'Running'),
      paused: ui('已暂停', 'Paused'),
      awaiting_human: ui('等待确认', 'Awaiting confirmation'),
      completed: ui('已完成', 'Completed'),
      failed: ui('失败', 'Failed'),
      cancelled: ui('已取消', 'Cancelled'),
      interrupted: ui('已中断', 'Interrupted'),
      skipped: ui('已跳过', 'Skipped'),
      starting: ui('准备中', 'Starting'),
    })[status] ?? status

  return (
    <div className="wb-app">
      <TitleBar brand="灵雀 Lingque" brandIcon={<LingqueLogo />} />
      <nav className="wb-nav" aria-label={ui('工作台导航', 'Workbench navigation')}>
        <div className="wb-nav-primary">
          <button
            aria-current={page === 'workflows' ? 'page' : undefined}
            onClick={() => switchPage('workflows')}
          >
            {active && page === 'workflows' ? (
              <ArrowLeft size={16} />
            ) : (
              <LayoutDashboard size={16} />
            )}
            {active && page === 'workflows'
              ? ui('返回工作流', 'Back to workflows')
              : ui('工作流', 'Workflows')}
          </button>
          <button
            aria-current={page === 'runs' ? 'page' : undefined}
            onClick={() => switchPage('runs')}
          >
            <History size={16} />
            {ui('运行记录', 'Runs')}
          </button>
        </div>
        <div className="wb-nav-secondary">
          <button
            aria-current={page === 'clients' ? 'page' : undefined}
            onClick={() => switchPage('clients')}
          >
            <Cable size={16} />
            {ui('外部接入', 'Connections')}
          </button>
          {active && page === 'workflows' && (
            <button
              id="wb-assistant-toggle"
              aria-label={
                active.authoring_mode === 'external'
                  ? ui('启用 AI 助手', 'Enable AI assistant')
                  : ui('AI 助手', 'AI assistant')
              }
              aria-pressed={active.authoring_mode === 'internal' && assistantOpen}
              aria-controls="wb-assistant-panel"
              onClick={() => {
                if (active.authoring_mode === 'external') void mode('internal')
                else setAssistantOpen(value => !value)
              }}
            >
              <Sparkles size={16} />
              {active.authoring_mode === 'external'
                ? ui('启用 AI 助手', 'Enable AI assistant')
                : ui('AI 助手', 'AI assistant')}
              {generationBusy && (
                <span
                  className="wb-busy-dot"
                  role="status"
                  aria-label={ui('生成中', 'Generating')}
                />
              )}
            </button>
          )}
          <button
            aria-expanded={settingsOpen}
            aria-haspopup="dialog"
            onClick={() => setSettingsOpen(true)}
          >
            <Settings size={16} />
            {ui('设置', 'Settings')}
          </button>
        </div>
      </nav>
      {settingsOpen && (
        <WorkbenchSettings
          ui={ui}
          projects={projects}
          projectId={projectId}
          mode={page === 'workflows' ? active?.authoring_mode : undefined}
          busy={generationBusy}
          onClose={() => setSettingsOpen(false)}
          onModels={() => switchPage('models')}
          onTheme={toggleTheme}
          onProject={id =>
            navigate(() => {
              setProjectId(id)
              setActive(null)
              setSettingsOpen(false)
              setPage('workflows')
            })
          }
          onOpenProject={() =>
            navigate(() => {
              setSettingsOpen(false)
              void chooseProject().catch(fail)
            })
          }
          onMode={value => void mode(value)}
        />
      )}
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
                    '把重复的工作，变成可重复使用的流程。',
                    'Turn repetitive work into reusable workflows.',
                  )}
                </p>
                {selectedProject && selectedProject.project_id !== projects[0]?.project_id && (
                  <small className="wb-location" title={selectedProject.directory}>
                    {ui('项目', 'Project')} · {selectedProject.name}
                  </small>
                )}
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
                <Plus size={16} />
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
                  <div className="wb-card-group" key={draft.workflow_id}>
                    <button
                      className="wb-card"
                      key={draft.workflow_id}
                      onClick={() => openDraft(draft)}
                    >
                      <h2>{draft.document.name}</h2>
                      <p>
                        {draft.authoring_mode === 'internal'
                          ? ui('AI 辅助', 'AI-assisted')
                          : ui('手动 · 外部编排', 'Manual · external')}
                      </p>
                      <small>
                        {ui('修订', 'Revision')} {draft.revision} · {draft.document.steps.length}{' '}
                        {ui('个顶层节点', 'top-level steps')}
                      </small>
                      <time
                        className="wb-updated"
                        dateTime={new Date(draft.updated_at).toISOString()}
                      >
                        {ui('更新于', 'Updated')}{' '}
                        {new Date(draft.updated_at).toLocaleString(
                          lang === 'zh' ? 'zh-CN' : 'en-US',
                        )}
                      </time>
                    </button>
                    <ScheduleControl
                      draft={draft}
                      summary={schedules.find(s => s.workflow_id === draft.workflow_id)}
                      onChanged={() => {
                        void refresh().catch(fail)
                      }}
                    />
                  </div>
                ))}
            </div>
          </section>
        )}
        {page === 'workflows' && active && backend && (
          <section className="wb-editor">
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
                  collapsed={!assistantOpen}
                  onCollapse={() => {
                    setAssistantOpen(false)
                    document.getElementById('wb-assistant-toggle')?.focus()
                  }}
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
                      // CanvasPage already checks unsaved edits; only add the host's generation lock.
                      if (generationRef.current) {
                        setNotice(
                          ui(
                            '请先停止当前生成，再切换画布。',
                            'Stop generation before switching canvases.',
                          ),
                        )
                        return
                      }
                      const next = drafts.find(draft => draft.workflow_id === id)
                      if (next) openDraft(next)
                    }}
                    onEditorState={editorState}
                    onGenerateIntent={text => {
                      setAssistantOpen(true)
                      setIntent(text)
                    }}
                  />
                </CanvasBackendContext.Provider>
              </div>
            </div>
          </section>
        )}
        {page === 'clients' && <ExternalConnections endpoint={endpoint} />}
        {page === 'runs' && (
          <section className="wb-dashboard">
            <h1>{ui('运行记录', 'Runs')}</h1>
            <div className="wb-actions">
              <button aria-pressed={!scheduledOnly} onClick={() => setScheduledOnly(false)}>
                {ui('全部运行', 'All runs')}
              </button>
              <button aria-pressed={scheduledOnly} onClick={() => setScheduledOnly(true)}>
                {ui('定时触发', 'Scheduled')}
              </button>
            </div>
            <p>
              {ui(
                '关闭连接或隐藏窗口不会取消执行。取消会等待正在进行的本地动作结束。',
                'Disconnecting or hiding the window does not cancel a run. Cancellation waits for an in-flight local action to finish.',
              )}
            </p>
            {scheduledOnly && projectId && (
              <ScheduleHistory key={projectId} projectId={projectId} />
            )}
            {runs
              .filter(run => !scheduledOnly || run.source === 'schedule')
              .map(run => (
                <article className="wb-run" key={run.run_id}>
                  <div className="wb-row">
                    <strong>
                      {drafts.find(draft => draft.workflow_id === run.workflow_id)?.document.name ??
                        run.workflow_id}
                    </strong>
                    <span className={`wb-run-status wb-run-status--${run.status}`}>
                      {statusText(run.status)}
                    </span>
                  </div>
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
                    <summary>{ui('运行详情', 'Run details')}</summary>
                    <code>{run.run_id}</code>
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
