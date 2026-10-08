import { useEffect, useRef } from 'react'
import {
  IconFolderOpen as FolderOpen,
  IconMoon as Moon,
  IconSettings as Settings,
  IconSlidersHorizontal as SlidersHorizontal,
  IconX as X,
} from '../ui/Icons'
import type { Project } from './api'
import { useAppUpdates } from './Updates'

interface Props {
  ui: (zh: string, en: string) => string
  projects: Project[]
  projectId: string
  mode?: 'internal' | 'external'
  busy: boolean
  onClose: () => void
  onModels: () => void
  onTheme: () => void
  onProject: (id: string) => void
  onOpenProject: () => void
  onMode: (mode: 'internal' | 'external') => void
}

/** Workbench-only controls; shared model settings and canvas stay upstream-owned. */
export function WorkbenchSettings(props: Props) {
  const updates = useAppUpdates()
  const { ui, onClose } = props
  const panel = useRef<HTMLDivElement>(null)
  const close = useRef(onClose)
  close.current = onClose
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null
    panel.current?.querySelector<HTMLButtonElement>('button')?.focus()
    const keydown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        close.current()
      }
      if (event.key !== 'Tab') return
      const items = Array.from(
        panel.current?.querySelectorAll<HTMLElement>(
          'button:not(:disabled), select:not(:disabled), [href], [tabindex="0"]',
        ) ?? [],
      )
      const first = items[0]
      const last = items[items.length - 1]
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault()
        last?.focus()
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault()
        first?.focus()
      }
    }
    document.addEventListener('keydown', keydown)
    return () => {
      document.removeEventListener('keydown', keydown)
      if (previous?.isConnected) previous.focus()
    }
  }, [])
  const project = props.projects.find(item => item.project_id === props.projectId)
  return (
    <div className="wb-settings-backdrop" onClick={onClose}>
      <div
        className="wb-settings"
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={ui('设置', 'Settings')}
        onClick={event => event.stopPropagation()}
      >
        <header>
          <h2>
            <Settings size={16} />
            {ui('设置', 'Settings')}
          </h2>
          <button
            className="wb-icon-button"
            onClick={onClose}
            aria-label={ui('关闭设置', 'Close settings')}
          >
            <X size={16} />
          </button>
        </header>
        <div className="wb-settings-actions">
          <button
            onClick={() => {
              onClose()
              updates.open()
            }}
          >
            版本与更新
          </button>
          <button onClick={props.onModels}>
            <SlidersHorizontal size={16} />
            <span>{ui('模型配置', 'Models')}</span>
          </button>
          <button onClick={props.onTheme}>
            <Moon size={16} />
            <span>{ui('切换主题', 'Toggle theme')}</span>
          </button>
        </div>
        {props.mode && (
          <section>
            <h3>{ui('当前工作流', 'Current workflow')}</h3>
            <label>
              {ui('编辑方式', 'Authoring mode')}
              <select
                value={props.mode}
                disabled={props.busy}
                onChange={event => props.onMode(event.target.value as 'internal' | 'external')}
              >
                <option value="internal">{ui('AI 辅助', 'AI-assisted')}</option>
                <option value="external">{ui('手动 · 外部编排', 'Manual · external')}</option>
              </select>
            </label>
            <small>
              {ui(
                '不影响画布内容或外部接入服务。',
                'Canvas content and external connections are unchanged.',
              )}
            </small>
          </section>
        )}
        <section>
          <h3>{ui('项目目录', 'Project folders')}</h3>
          <label>
            {ui('当前项目', 'Current project')}
            <select
              value={props.projectId}
              disabled={props.busy}
              onChange={event => props.onProject(event.target.value)}
            >
              {props.projects.map(item => (
                <option key={item.project_id} value={item.project_id}>
                  {item.name}
                </option>
              ))}
            </select>
          </label>
          <p className="wb-project-path" title={project?.directory}>
            {project?.directory}
          </p>
          <button disabled={props.busy} onClick={props.onOpenProject}>
            <FolderOpen size={16} />
            {ui('打开其他项目', 'Open another project')}
          </button>
        </section>
      </div>
    </div>
  )
}
