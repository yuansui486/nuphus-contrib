import { useEffect, useId, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLanguage } from '../../locales'
import { Section } from '../../ui/PageLayout'
import { Button } from '../../ui/Button'
import { IconCheck, IconChevronDown, IconExternalLink, IconPlus, IconRefresh } from '../../ui/Icons'
import '../../styles/chatgpt-settings.css'
import {
  getCurrentConfig,
  listModels,
  openExternal,
  setModelSupportsVision,
  switchModel,
  type ModelInfo,
} from '../lib/api'

interface Account {
  account_id: string
  provider: string
  label: string
  logged_in: boolean
}
interface Model {
  slug: string
  display_name: string
}

/** 隐去系统生成的账号 ID，保留用户可识别的账号与工作区名称。 */
function accountLabel(account: Account) {
  const suffix = ` · ${account.account_id}`
  return account.label.endsWith(suffix)
    ? account.label.slice(0, -suffix.length) || account.label
    : account.label
}

export function ChatGPTSettings({
  onModelChanged,
  onModelsChanged,
}: {
  onModelChanged?: () => void
  onModelsChanged?: (models: ModelInfo[]) => void
}) {
  const { lang } = useLanguage()
  const zh = lang.startsWith('zh')
  const visionHintId = useId()
  const [accounts, setAccounts] = useState<Account[]>([])
  const [selected, setSelected] = useState('')
  const [models, setModels] = useState<Model[]>([])
  const [registeredModels, setRegisteredModels] = useState<ModelInfo[]>([])
  const [busy, setBusy] = useState('accounts')
  const [catalogRevision, setCatalogRevision] = useState(0)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [active, setActive] = useState('')
  const generation = useRef(0)
  const account = accounts.find(a => a.account_id === selected)

  useEffect(() => {
    let mounted = true
    Promise.all([invoke<Account[]>('chatgpt_accounts'), getCurrentConfig().catch(() => null)])
      .then(([items, config]) => {
        if (mounted) {
          setAccounts(items)
          setSelected(
            (items.find(a => a.provider === config?.provider) ?? items[0])?.account_id ?? '',
          )
          if (config) setActive(`${config.provider}:${config.model}`)
          setBusy('')
        }
      })
      .catch(e => {
        if (mounted) {
          setError(String(e))
          setBusy('')
        }
      })
    return () => {
      mounted = false
      generation.current++
      void invoke('chatgpt_cancel_login').catch(() => {})
    }
  }, [])

  async function loadModels(id: string) {
    const current = ++generation.current
    setModels([])
    setBusy('models')
    setError('')
    try {
      const list = await invoke<Model[]>('chatgpt_models', { accountId: id })
      const registered = await listModels()
      if (generation.current === current) {
        setModels(list)
        setRegisteredModels(registered ?? [])
        onModelsChanged?.(registered ?? [])
      }
    } catch (e) {
      if (generation.current === current) setError(String(e))
    } finally {
      if (generation.current === current) setBusy('')
    }
  }

  useEffect(() => {
    if (account?.logged_in) void loadModels(account.account_id)
    else {
      generation.current++
      setModels([])
    }
  }, [selected, account?.logged_in, catalogRevision])

  async function login(id?: string) {
    setBusy('login')
    setError('')
    setNotice('')
    const current = ++generation.current
    try {
      const result = await invoke<Account>('chatgpt_login', { accountId: id ?? null })
      const list = await invoke<Account[]>('chatgpt_accounts')
      if (generation.current !== current) return
      setAccounts(list)
      setSelected(result.account_id)
      /** 重新授权可能仍选中同一账号，因此显式刷新目录。 */
      setCatalogRevision(revision => revision + 1)
    } catch (e) {
      if (generation.current === current) setError(String(e))
    } finally {
      if (generation.current === current) setBusy('')
    }
  }

  async function logout() {
    if (!account) return
    setBusy('logout')
    setError('')
    setNotice('')
    try {
      const revoked = await invoke<boolean>('chatgpt_logout', { accountId: account.account_id })
      setAccounts(await invoke<Account[]>('chatgpt_accounts'))
      setModels([])
      setActive('')
      onModelsChanged?.((await invoke<ModelInfo[]>('list_models')) ?? [])
      onModelChanged?.()
      setNotice(
        revoked
          ? zh
            ? '已退出登录。'
            : 'Signed out.'
          : zh
            ? '已在本机退出，但未确认远端撤销。请到 ChatGPT 设置中断开此应用。'
            : 'Signed out locally; remote revocation was not confirmed. Disconnect this app in ChatGPT Settings.',
      )
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy('')
    }
  }

  async function selectModel(model: Model) {
    if (!account) return
    setBusy('switch')
    setError('')
    try {
      await switchModel(model.slug, account.provider, undefined, undefined, 'default')
      setActive(`${account.provider}:${model.slug}`)
      onModelChanged?.()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy('')
    }
  }

  /** 模型目录不声明视觉能力时，复用用户显式设置，按账号和模型共同定位。 */
  async function toggleVision(model: Model, enabled: boolean) {
    if (!account) return
    const current = ++generation.current
    setBusy('vision')
    setError('')
    try {
      await setModelSupportsVision(account.provider, model.slug, enabled)
      if (generation.current !== current) return
      const updated = registeredModels.map(item =>
        item.provider === account.provider && item.id === model.slug
          ? { ...item, supports_vision: enabled }
          : item,
      )
      setRegisteredModels(updated)
      onModelsChanged?.(updated)
    } catch (e) {
      if (generation.current === current) setError(String(e))
    } finally {
      if (generation.current === current) setBusy('')
    }
  }

  return (
    <Section
      className="chatgpt-settings"
      title="ChatGPT"
      description={
        zh
          ? '连接你的 ChatGPT 套餐，在 Nuphus 中使用账号内的模型。'
          : 'Connect your ChatGPT plan to use your account’s models in Nuphus.'
      }
      actions={
        <Button
          variant="ghost"
          size="sm"
          icon={<IconExternalLink size={14} aria-hidden="true" />}
          onClick={() =>
            void openExternal('https://chatgpt.com/#settings/Usage').catch(e => setError(String(e)))
          }
        >
          {zh ? '查看用量' : 'View usage'}
        </Button>
      }
    >
      <div className="chatgpt-account">
        <div className="chatgpt-account-heading">
          <span className="chatgpt-label">{zh ? '账号与工作区' : 'Account & workspace'}</span>
          <span className={`chatgpt-connection${account?.logged_in ? ' is-connected' : ''}`}>
            <span className="chatgpt-connection-dot" aria-hidden="true" />
            {busy === 'accounts'
              ? zh
                ? '加载中'
                : 'Loading'
              : account?.logged_in
                ? zh
                  ? '已连接'
                  : 'Connected'
                : zh
                  ? '未连接'
                  : 'Not connected'}
          </span>
        </div>
        <div className="chatgpt-account-controls">
          {accounts.length > 0 ? (
            <div className="chatgpt-select-wrap">
              <select
                className="chatgpt-account-select"
                aria-label={zh ? 'ChatGPT 账号' : 'ChatGPT account'}
                value={selected}
                disabled={!!busy}
                onChange={e => {
                  setSelected(e.target.value)
                  setNotice('')
                  setError('')
                }}
              >
                {accounts.map(a => (
                  <option key={a.account_id} value={a.account_id}>
                    {accountLabel(a)}
                    {accounts.filter(item => accountLabel(item) === accountLabel(a)).length > 1
                      ? ` · …${a.account_id.slice(-8)}`
                      : ''}
                  </option>
                ))}
              </select>
              <IconChevronDown size={16} aria-hidden="true" />
            </div>
          ) : (
            <p className="chatgpt-account-empty">
              {zh
                ? '登录后即可选择套餐内的可用模型。'
                : 'Sign in to choose the models available on your plan.'}
            </p>
          )}
          <div className="chatgpt-account-actions">
            {!account?.logged_in && (
              <Button
                variant="primary"
                disabled={!!busy}
                onClick={() => void login(account?.account_id)}
              >
                Continue with ChatGPT
              </Button>
            )}
            {accounts.length > 0 && (
              <Button
                disabled={!!busy}
                icon={<IconPlus size={14} aria-hidden="true" />}
                onClick={() => void login()}
              >
                {zh ? '添加账号' : 'Add account'}
              </Button>
            )}
            {account?.logged_in && (
              <Button variant="ghost" disabled={!!busy} onClick={() => void logout()}>
                {zh ? '退出登录' : 'Sign out'}
              </Button>
            )}
          </div>
        </div>
        <p className="chatgpt-plan-note">
          {zh
            ? '此连接使用 ChatGPT 套餐用量；API Key 连接另行计费。'
            : 'This connection uses your ChatGPT plan; API key connections are billed separately.'}
        </p>
      </div>
      {busy && (
        <div className="chatgpt-feedback" role="status">
          <span className="btn-spinner" aria-hidden="true" />
          <span>
            {busy === 'login'
              ? zh
                ? '请在浏览器完成登录与授权…'
                : 'Complete sign-in and consent in your browser…'
              : busy === 'models'
                ? zh
                  ? '正在获取可用模型…'
                  : 'Loading available models…'
                : zh
                  ? '正在更新…'
                  : 'Updating…'}
          </span>
          {busy === 'login' && (
            <Button
              size="sm"
              onClick={() => void invoke('chatgpt_cancel_login').catch(e => setError(String(e)))}
            >
              {zh ? '取消登录' : 'Cancel sign-in'}
            </Button>
          )}
        </div>
      )}
      {error && (
        <div role="alert" className="chatgpt-feedback is-error">
          <span>{error}</span>
          {account?.logged_in && (
            <Button size="sm" disabled={!!busy} onClick={() => void login(account.account_id)}>
              {zh ? '重新授权' : 'Reconnect'}
            </Button>
          )}
        </div>
      )}
      {notice && (
        <p className="chatgpt-feedback" role="status">
          {notice}
        </p>
      )}
      {account?.logged_in && (
        <div className="chatgpt-catalog">
          <div className="chatgpt-catalog-heading">
            <h4>
              {zh ? '可用模型' : 'Available models'}{' '}
              <span className="chatgpt-model-count">{models.length}</span>
            </h4>
            <Button
              variant="ghost"
              size="sm"
              disabled={!!busy}
              icon={<IconRefresh size={14} aria-hidden="true" />}
              onClick={() => void loadModels(account.account_id)}
            >
              {zh ? '刷新模型' : 'Refresh models'}
            </Button>
          </div>
          <p className="chatgpt-catalog-hint" id={visionHintId}>
            {zh
              ? '选择默认对话模型。仅为确认支持图片的模型开启视觉输入。'
              : 'Choose your default model. Enable vision only for models that support images.'}
          </p>
          {models.length > 0 && (
            <div className="chatgpt-model-list">
              <div className="chatgpt-model-columns" aria-hidden="true">
                <span>{zh ? '模型' : 'Model'}</span>
                <span>{zh ? '视觉输入' : 'Vision input'}</span>
                <span>{zh ? '操作' : 'Action'}</span>
              </div>
              <ul aria-label={zh ? '可用模型' : 'Available models'}>
                {models.map(model => {
                  const isActive = active === `${account.provider}:${model.slug}`
                  const supportsVision = registeredModels.some(
                    item =>
                      item.provider === account.provider &&
                      item.id === model.slug &&
                      item.supports_vision,
                  )
                  return (
                    <li
                      key={model.slug}
                      className={`chatgpt-model-row${isActive ? ' is-active' : ''}`}
                    >
                      <div className="chatgpt-model-name">
                        <span>{model.display_name}</span>
                        {isActive && (
                          <span className="chatgpt-active-badge">
                            <IconCheck size={12} aria-hidden="true" />
                            {zh ? '当前模型' : 'Current'}
                          </span>
                        )}
                      </div>
                      <div className="chatgpt-vision-control">
                        <span className="chatgpt-vision-mobile" aria-hidden="true">
                          {zh ? '视觉输入' : 'Vision input'}
                        </span>
                        <button
                          type="button"
                          role="switch"
                          className="switch"
                          aria-label={`${model.display_name} ${zh ? '视觉输入' : 'vision input'}`}
                          aria-describedby={visionHintId}
                          aria-checked={supportsVision}
                          disabled={!!busy}
                          onClick={() => void toggleVision(model, !supportsVision)}
                        />
                      </div>
                      <Button
                        size="sm"
                        variant="ghost"
                        aria-label={`${model.display_name}${isActive ? (zh ? ' · 使用中' : ' · Active') : ''}`}
                        aria-pressed={isActive}
                        disabled={!!busy || isActive}
                        onClick={() => void selectModel(model)}
                      >
                        {isActive ? (zh ? '使用中' : 'Active') : zh ? '设为默认' : 'Use model'}
                      </Button>
                    </li>
                  )
                })}
              </ul>
            </div>
          )}
          {!busy && !error && models.length === 0 && (
            <p className="chatgpt-model-empty">
              {zh
                ? '此账号当前没有可用模型，可刷新或切换账号重试。'
                : 'No models available. Refresh or try another account.'}
            </p>
          )}
        </div>
      )}
    </Section>
  )
}
