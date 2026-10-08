import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import { MarkdownInline } from '../main-window/chat/MarkdownContent'
import './updates.css'

export interface UpdateStatus {
  revision: number
  current_version: string
  version: string | null
  notes: string | null
  phase:
    | 'idle'
    | 'checking'
    | 'current'
    | 'available'
    | 'downloading'
    | 'verifying'
    | 'ready'
    | 'installing'
    | 'failed'
  automatic: boolean
  last_check: number
  downloaded: number
  total: number | null
  bytes_per_second: number
  eta_seconds: number | null
  error: string | null
}
type Save = () => Promise<boolean>
const defaultContext = { open: () => {}, busy: false, registerSave: (_save: Save | null) => {} }
const UpdatesContext = createContext(defaultContext)
export const useAppUpdates = () => useContext(UpdatesContext)

export function newerStatus(current: UpdateStatus | null, next: UpdateStatus) {
  return !current || next.revision > current.revision ? next : current
}

const phases: Record<UpdateStatus['phase'], string> = {
  idle: '可手动检查更新',
  checking: '正在检查更新…',
  current: '已是最新版',
  available: '发现新版本',
  downloading: '正在下载…',
  verifying: '正在校验签名…',
  ready: '更新已下载并通过校验',
  installing: '正在安装，请稍候…',
  failed: '更新未完成',
}

export function AppUpdatesProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<UpdateStatus | null>(null)
  const [version, setVersion] = useState('—')
  const [opened, setOpened] = useState(false)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [pending, setPending] = useState(false)
  const save = useRef<Save | null>(null)
  const installing = useRef(false)
  const content = useRef<HTMLDivElement>(null)
  const dialog = useRef<HTMLDivElement>(null)
  const open = useCallback(() => setOpened(true), [])
  const registerSave = useCallback((callback: Save | null) => {
    save.current = callback
  }, [])
  const receive = useCallback((next: UpdateStatus) => {
    if (next && typeof next.revision === 'number') setStatus(current => newerStatus(current, next))
  }, [])
  useEffect(() => {
    let alive = true
    const subscription = listen<UpdateStatus>('app-update-progress', event => {
      if (alive) receive(event.payload)
    })
    void subscription
      .then(async () => {
        const initial = await invoke<UpdateStatus>('get_app_update_status')
        if (alive) receive(initial)
      })
      .catch(e => {
        if (alive) setError(String(e))
      })
    void getVersion()
      .then(value => {
        if (alive) setVersion(value)
      })
      .catch(() => {})
    return () => {
      alive = false
      void subscription.then(stop => stop()).catch(() => {})
    }
  }, [receive])

  useEffect(() => {
    if (!opened) return
    const previous = document.activeElement as HTMLElement | null
    if (content.current) content.current.inert = true
    dialog.current?.focus()
    const keyboard = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        event.stopImmediatePropagation()
        if (!installing.current) setOpened(false)
      }
      if (event.key !== 'Tab') return
      event.stopImmediatePropagation()
      const items = Array.from(
        dialog.current?.querySelectorAll<HTMLElement>(
          'button:not(:disabled), input:not(:disabled)',
        ) ?? [],
      )
      const first = items[0],
        last = items[items.length - 1]
      if (!first) {
        event.preventDefault()
        return
      }
      if (
        event.shiftKey &&
        (document.activeElement === first || document.activeElement === dialog.current)
      ) {
        event.preventDefault()
        last.focus()
      } else if (
        !event.shiftKey &&
        (document.activeElement === last || document.activeElement === dialog.current)
      ) {
        event.preventDefault()
        first.focus()
      }
    }
    document.addEventListener('keydown', keyboard, true)
    return () => {
      document.removeEventListener('keydown', keyboard, true)
      if (content.current) content.current.inert = false
      if (previous?.isConnected) previous.focus()
    }
  }, [opened])

  const command = async (name: string, args?: Record<string, unknown>) => {
    setPending(true)
    setError('')
    try {
      receive(await invoke<UpdateStatus>(name, args))
    } catch (e) {
      setError(String(e))
    } finally {
      setPending(false)
    }
  }
  const install = async () => {
    if (installing.current) return
    installing.current = true
    setBusy(true)
    setError('')
    try {
      if (save.current && !(await save.current()))
        throw new Error('保存尚未完成或存在冲突，未安装更新。请回到画布处理后重试。')
      receive(await invoke<UpdateStatus>('install_app_update'))
    } catch (e) {
      setError(String(e))
    } finally {
      installing.current = false
      setBusy(false)
    }
  }
  const downloading = status?.phase === 'downloading' || status?.phase === 'verifying'
  const nativeBusy = busy || status?.phase === 'installing'
  const percent = status?.total
    ? Math.min(100, Math.round((status.downloaded / status.total) * 100))
    : undefined
  return (
    <UpdatesContext.Provider value={{ open, busy: nativeBusy, registerSave }}>
      <div ref={content}>{children}</div>
      {!opened && (status?.phase === 'available' || status?.phase === 'ready') && (
        <button className="lq-update-notice" onClick={open}>
          灵雀 {status.version} 可更新 · 查看
        </button>
      )}
      {opened && (
        <div className="lq-update-backdrop">
          <div
            className="lq-update-dialog"
            role="dialog"
            aria-modal="true"
            aria-label="版本与更新"
            tabIndex={-1}
            ref={dialog}
          >
            <header>
              <h2>版本与更新</h2>
              <button disabled={nativeBusy} onClick={() => setOpened(false)}>
                关闭
              </button>
            </header>
            <p>灵雀 · 当前版本 {status?.current_version ?? version}</p>
            <label>
              <input
                type="checkbox"
                checked={status?.automatic ?? true}
                disabled={!status || nativeBusy || pending}
                onChange={event =>
                  void command('set_app_update_preferences', { automatic: event.target.checked })
                }
              />
              每天自动检查更新
            </label>
            <p role="status">{status ? phases[status.phase] : '正在读取更新状态…'}</p>
            {status?.version && <h3>新版本 {status.version}</h3>}
            {status?.notes && (
              <div className="lq-update-notes">
                {status.notes.split('\n').map((line, index) => (
                  <p key={index}>
                    <MarkdownInline text={line} />
                  </p>
                ))}
              </div>
            )}
            {downloading && (
              <>
                <progress aria-label="更新下载进度" max={100} value={percent} />
                <p>
                  {percent === undefined ? '已下载' : `${percent}% · 已下载`}{' '}
                  {((status?.downloaded ?? 0) / 1048576).toFixed(1)} MB
                  {!!status?.bytes_per_second &&
                    ` · ${(status.bytes_per_second / 1048576).toFixed(1)} MB/s`}
                  {status?.eta_seconds != null && ` · 约剩 ${status.eta_seconds} 秒`}
                </p>
              </>
            )}
            {(error || status?.error) && <p role="alert">{error || status?.error}</p>}
            <footer>
              <button
                disabled={nativeBusy || pending || downloading}
                onClick={() => void command('check_app_update')}
              >
                检查更新
              </button>
              {(status?.phase === 'available' ||
                (status?.phase === 'failed' && status.version)) && (
                <button
                  disabled={nativeBusy || pending}
                  onClick={() => void command('download_app_update')}
                >
                  下载更新
                </button>
              )}
              {downloading && (
                <>
                  <button disabled={nativeBusy} onClick={() => void command('cancel_app_update')}>
                    取消下载
                  </button>
                  <button disabled={nativeBusy} onClick={() => setOpened(false)}>
                    后台下载
                  </button>
                </>
              )}
              {status?.phase === 'ready' && (
                <button disabled={nativeBusy || pending} onClick={() => void install()}>
                  保存并重启安装
                </button>
              )}
            </footer>
            <small>
              下载不会自动安装。安装前将保存画布；仍有运行、暂停或模型准备任务时不会重启。
            </small>
          </div>
        </div>
      )}
    </UpdatesContext.Provider>
  )
}
