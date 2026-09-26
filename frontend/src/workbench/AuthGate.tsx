import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { TitleBar } from '../main-window/layout/TitleBar'
import WorkbenchApp from './WorkbenchApp'
import './auth.css'

export interface AuthStatus {
  state: string
  authorized: boolean
  message: string
  epoch: string | null
  subject: {
    id: string
    tenant_id: string
    tenant_name: string
    username: string
    display_name?: string
  } | null
  policy: { concurrent_device_limit: number; active_session_count: number } | null
  offline_until: string | null
}
const auth = <T,>(action: string, args: Record<string, unknown> = {}) =>
  invoke<T>('lingque_auth', { action, ...args })
const message = (error: unknown) =>
  typeof error === 'object' && error && 'message' in error ? String(error.message) : String(error)

export default function AuthGate() {
  const [status, setStatus] = useState<AuthStatus | null>(null)
  const [tenant, setTenant] = useState('')
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [projects, setProjects] = useState<Array<{
    project_id: string
    name: string
    directory: string
  }> | null>(null)
  const busyRef = useRef(false)
  const epoch = useRef<string | null>(null)
  const mounted = useRef(true)
  const apply = useCallback((next: AuthStatus) => {
    if (!mounted.current) return
    setStatus(next)
    if (!next.authorized || next.epoch !== epoch.current) {
      epoch.current = next.authorized ? next.epoch : null
      setProjects(null)
    }
  }, [])
  useEffect(() => {
    mounted.current = true
    void invoke('finish_startup').catch(() => {})
    void auth<AuthStatus>('status')
      .then(apply)
      .catch(e => setError(message(e)))
    const listener = listen<AuthStatus>('lingque-auth-changed', event => apply(event.payload))
    return () => {
      mounted.current = false
      void listener.then(stop => stop())
    }
  }, [apply])
  useEffect(() => {
    if (!status?.authorized) return
    let alive = true
    void auth<NonNullable<typeof projects>>('unclaimed')
      .then(items => {
        if (alive) setProjects(items)
      })
      .catch(e => {
        if (alive) setError(message(e))
      })
    return () => {
      alive = false
    }
  }, [status?.authorized, status?.epoch])

  const action = async (kind: 'login' | 'verify' | 'logout' | 'claim' | 'status') => {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(true)
    setError('')
    try {
      if (kind === 'claim') {
        await auth('claim', {
          expectedEpoch: status?.epoch,
          projectIds: projects?.map(p => p.project_id),
        })
        setProjects(await auth('unclaimed'))
      } else {
        if (kind === 'logout') {
          setStatus(null)
          setProjects(null)
          setPassword('')
        }
        apply(
          await auth<AuthStatus>(
            kind,
            kind === 'login' ? { tenantCode: tenant, username, password } : {},
          ),
        )
      }
    } catch (e) {
      setError(message(e))
      try {
        apply(await auth<AuthStatus>('status'))
      } catch {
        /* Keep the error, never fabricate authorization. */
      }
    } finally {
      setPassword('')
      busyRef.current = false
      setBusy(false)
    }
  }
  const submit = (event: FormEvent) => {
    event.preventDefault()
    void action('login')
  }

  if (status?.authorized && projects?.length === 0) {
    return (
      <div className="lq-authorized">
        <div className="lq-account" role="status">
          <span>
            {status.subject?.tenant_name} ·{' '}
            {status.subject?.display_name || status.subject?.username}
          </span>
          <span>
            {status.state === 'offline' ? '离线授权' : '已授权'}
            {status.offline_until
              ? ` · 有效至 ${new Date(status.offline_until).toLocaleString('zh-CN')}`
              : ''}
          </span>
          <button disabled={busy} onClick={() => void action('verify')}>
            联网验证
          </button>
          <button
            disabled={busy}
            onClick={() => {
              if (window.confirm('退出将停止当前任务和定时执行。确认退出？')) void action('logout')
            }}
          >
            退出登录
          </button>
          {error && <span role="alert">{error}</span>}
        </div>
        <WorkbenchApp
          key={`${status.subject?.tenant_id}:${status.epoch}`}
          storageScope={status.subject?.tenant_id}
        />
      </div>
    )
  }
  return (
    <div className="lq-login-shell">
      <TitleBar brand="灵雀 Lingque" />
      <main className="lq-login-main">
        <section className="lq-login-card" aria-label="灵雀登录">
          <p className="lq-overline">LINGQUE</p>
          <h1>欢迎使用灵雀</h1>
          <p className="lq-subtitle">连接你的工作流，让任务有序执行。</p>
          {status?.authorized ? (
            <>
              <h2>确认旧项目归属</h2>
              <p>
                将以下旧项目归入「{status.subject?.tenant_name}
                」。同租户账号共享，其他租户不可通过灵雀访问。模型配置仍作为本机公共设置保留。
              </p>
              {projects === null ? (
                <p>正在读取待确认项目…</p>
              ) : (
                <ul className="lq-projects">
                  {projects.map(p => (
                    <li key={p.project_id}>
                      <strong>{p.name}</strong>
                      <small>{p.directory}</small>
                    </li>
                  ))}
                </ul>
              )}
              <button disabled={busy || projects === null} onClick={() => void action('claim')}>
                确认归属并进入
              </button>
              <button
                className="lq-secondary"
                disabled={busy}
                onClick={() => void action('logout')}
              >
                退出，换个账号
              </button>
            </>
          ) : status?.subject ? (
            <>
              <p role="status">{status.message}</p>
              <button disabled={busy} onClick={() => void action('verify')}>
                重新联网验证
              </button>
              <button
                className="lq-secondary"
                disabled={busy}
                onClick={() => void action('logout')}
              >
                退出当前登录
              </button>
            </>
          ) : (
            <form onSubmit={submit}>
              <label>
                租户编码
                <input
                  autoComplete="organization"
                  maxLength={64}
                  value={tenant}
                  onChange={e => setTenant(e.target.value)}
                  required
                  disabled={busy}
                />
              </label>
              <label>
                用户名
                <input
                  autoComplete="username"
                  maxLength={64}
                  value={username}
                  onChange={e => setUsername(e.target.value)}
                  required
                  disabled={busy}
                />
              </label>
              <label>
                密码
                <input
                  type="password"
                  autoComplete="current-password"
                  maxLength={128}
                  value={password}
                  onChange={e => setPassword(e.target.value)}
                  required
                  disabled={busy}
                />
              </label>
              <button type="submit" disabled={busy || status === null}>
                {busy ? '正在登录…' : '登录灵雀'}
              </button>
            </form>
          )}
          <p role="alert" className="lq-error">
            {error || (!status?.authorized ? status?.message : '')}
          </p>
          {status === null && error && (
            <button className="lq-secondary" disabled={busy} onClick={() => void action('status')}>
              重新连接登录服务
            </button>
          )}
          <p className="lq-device-note">
            设备额度由同租户所有账号合计使用。额度满时，新设备登录会替换较早活跃的设备。后台撤销会在下次联网验证时生效；离线设备仍受原离线期限约束。
          </p>
        </section>
      </main>
    </div>
  )
}
