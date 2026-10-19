/**
 * Laya 决策后端面板 —— 「增强判断模型」页内与 Jev 并列的后端选项。
 *
 * 本组件不渲染自己的 Section：它由 JevSettings 在同一 Section 内按 tab 切换，
 * 复用既有的 .jev-* 样式，避免第二套视觉实现。
 *
 * Laya（github.com/NandhaKishorM/laya）是独立模型，只是恰好兼容 Jev 的
 * /v1/systemone 协议。两者各自独立配置，同一时刻只有一个生效
 * （后端 ModelRegistry::decision_backend：Jev 已配 Key 时优先）。
 */
import { useEffect, useState } from 'react'
import { IconBrushCleaning, IconEye, IconEyeOff, IconPlug } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { FormRow } from '../../ui/PageLayout'
import { CompactModal } from '../layout/CompactModal'
import { useLanguage } from '../../locales'
import {
  clearLayaApiKey,
  getLayaConfig,
  saveLayaConfig,
  testLayaConnection,
  type LayaConnectionStatus,
} from '../lib/api'

const DEFAULT_BASE_URL = 'http://127.0.0.1:8000'
const DEFAULT_TIMEOUT_MS = 10_000
const DEFAULT_MAX_RETRIES = 2

type TFunc = (key: string, ...args: string[]) => string

/** 词条取值：字典命中 → 用字典文案；未命中 → 用中文兜底（与 JevSettings 同模式）。 */
const tr = (t: TFunc, key: string, fallback: string): string => {
  const v = t(key)
  return v === key ? fallback : v
}

/** 页内文案表：按当前语言生成，键名稳定。 */
function makeTXT(t: TFunc) {
  return {
    enabledState: tr(t, 'laya.enabledState', '已启用'),
    disabledState: tr(t, 'laya.disabledState', '未启用'),
    offState: tr(t, 'laya.offState', '已关闭'),
    notTestedNote: tr(
      t,
      'laya.notTestedNote',
      '本功能未在开发环境中完成端到端实测（缺少可运行 Laya 的机器），协议对接依据 Laya 源码实现，尚未经过真实模型验证。投入使用前请按实际部署自行验证并按需修改。',
    ),
    exclusiveNote: tr(
      t,
      'laya.exclusiveNote',
      '另请注意：Jev 与 Laya 各自独立配置，同一时刻只有一个生效，Jev 已配置 API Key 时优先生效。',
    ),
    enableLabel: tr(t, 'laya.enableLabel', '启用 Laya 决策后端'),
    enableHint: tr(
      t,
      'laya.enableHint',
      '关闭时不会联系任何 Laya 服务，增强模式仍按既有的 Jev 或主模型逻辑运行。',
    ),
    enableAria: tr(t, 'laya.enableAria', '启用 Laya 决策后端'),
    baseUrlLabel: tr(t, 'laya.baseUrlLabel', '服务地址'),
    baseUrlHint: tr(
      t,
      'laya.baseUrlHint',
      '自托管 Laya 服务地址，默认本机 8000 端口；非本机地址必须使用 HTTPS。',
    ),
    baseUrlAria: tr(t, 'laya.baseUrlAria', 'Laya 服务地址'),
    modelLabel: tr(t, 'laya.modelLabel', '模型'),
    modelHint: tr(
      t,
      'laya.modelHint',
      '可填 english、multilingual、typed-decisions；留空则由 Laya 路由按语言自动选择。',
    ),
    modelPlaceholder: tr(t, 'laya.modelPlaceholder', '留空即自动路由'),
    modelAria: tr(t, 'laya.modelAria', 'Laya 模型'),
    configured: tr(t, 'laya.configured', '已配置'),
    apiKeyHint: tr(
      t,
      'laya.apiKeyHint',
      '自托管 Laya 通常无需 Key，仅当服务端设置了 LAYA_API_KEY 时才需要填写。',
    ),
    apiKeyPlaceholderSet: tr(t, 'laya.apiKeyPlaceholderSet', '已配置；输入新密钥可覆盖'),
    apiKeyPlaceholderEmpty: tr(t, 'laya.apiKeyPlaceholderEmpty', '通常留空'),
    apiKeyAria: tr(t, 'laya.apiKeyAria', 'Laya API Key'),
    showKey: tr(t, 'laya.showKey', '显示'),
    hideKey: tr(t, 'laya.hideKey', '隐藏'),
    showKeyAria: tr(t, 'laya.showKeyAria', '显示 Laya API Key'),
    hideKeyAria: tr(t, 'laya.hideKeyAria', '隐藏 Laya API Key'),
    clearKeyTitle: tr(t, 'laya.clearKeyTitle', '清除已保存的 Laya API Key'),
    clearKeyAria: tr(t, 'laya.clearKeyAria', '清除已保存的 Laya API Key'),
    timeoutLabel: tr(t, 'laya.timeoutLabel', '请求超时（毫秒）'),
    timeoutHint: tr(
      t,
      'laya.timeoutHint',
      '单次请求的最长等待时间，范围 100–120000。CPU 推理需要适当放宽。',
    ),
    timeoutAria: tr(t, 'laya.timeoutAria', 'Laya 请求超时'),
    retriesLabel: tr(t, 'laya.retriesLabel', '最大重试次数'),
    retriesHint: tr(t, 'laya.retriesHint', '只对可重试的临时错误生效，范围 0–10。'),
    retriesAria: tr(t, 'laya.retriesAria', 'Laya 最大重试次数'),
    fallbackLabel: tr(t, 'laya.fallbackLabel', 'Laya 不可用时回退主模型'),
    fallbackHint: tr(
      t,
      'laya.fallbackHint',
      '服务请求失败时，由主模型继续从同一有限候选动作空间选择。',
    ),
    fallbackAria: tr(t, 'laya.fallbackAria', '回退到主模型'),
    saveBtn: tr(t, 'laya.saveBtn', '保存配置'),
    testBtn: tr(t, 'laya.testBtn', '测试连接'),
    clearModalTitle: tr(t, 'laya.clearModalTitle', '清除 API Key'),
    cancel: tr(t, 'laya.cancel', '取消'),
    confirmClear: tr(t, 'laya.confirmClear', '确认清除'),
    clearModalBody: tr(
      t,
      'laya.clearModalBody',
      '确定清除已保存的 Laya API Key？服务地址和模型配置会保留。',
    ),
    // ── 反馈消息（函数型，带 {0} 占位符）──
    connOk: tr(t, 'laya.connOk', '服务可达'),
    connRouted: (m: string) =>
      t('laya.connRouted', m) === 'laya.connRouted'
        ? `服务可达（路由到 ${m}）`
        : t('laya.connRouted', m),
    connStatus: (s: string) =>
      t('laya.connStatus', s) === 'laya.connStatus' ? `连接状态：${s}` : t('laya.connStatus', s),
    errNeedBaseUrl: tr(t, 'laya.errNeedBaseUrl', '请输入 Laya 服务地址'),
    errTimeoutRange: tr(t, 'laya.errTimeoutRange', '请求超时必须介于 100 与 120000 毫秒之间'),
    errRetriesRange: tr(t, 'laya.errRetriesRange', '最大重试次数必须是 0 到 10 之间的整数'),
    errNoConfig: tr(t, 'laya.errNoConfig', '后端未返回 Laya 配置'),
    saveOk: tr(t, 'laya.saveOk', 'Laya 配置已保存'),
    errNoTestResult: tr(t, 'laya.errNoTestResult', '后端未返回连接测试结果'),
    clearOk: tr(t, 'laya.clearOk', 'Laya API Key 已清除'),
  }
}

function connectionMessage(
  result: LayaConnectionStatus | string,
  TXT: ReturnType<typeof makeTXT>,
): string {
  if (typeof result === 'string') return result
  if (result.message) return result.message
  const status = result.status.toLowerCase()
  if (['ok', 'ready', 'available', 'connected'].includes(status)) {
    return result.model ? TXT.connRouted(result.model) : TXT.connOk
  }
  return TXT.connStatus(result.status)
}

function connectionOk(result: LayaConnectionStatus | string): boolean {
  if (typeof result === 'string') {
    return ['ok', 'ready', 'available', 'connected', 'success'].includes(result.toLowerCase())
  }
  return ['ok', 'ready', 'available', 'connected', 'success'].includes(result.status.toLowerCase())
}

export function LayaBackendPanel() {
  const { t } = useLanguage()
  const TXT = makeTXT(t)
  const [baseUrl, setBaseUrl] = useState(DEFAULT_BASE_URL)
  const [model, setModel] = useState('')
  const [enabled, setEnabled] = useState(false)
  const [apiKey, setApiKey] = useState('')
  const [hasKey, setHasKey] = useState(false)
  const [showKey, setShowKey] = useState(false)
  const [timeoutMs, setTimeoutMs] = useState(String(DEFAULT_TIMEOUT_MS))
  const [maxRetries, setMaxRetries] = useState(String(DEFAULT_MAX_RETRIES))
  const [fallbackToPrimaryModel, setFallbackToPrimaryModel] = useState(true)

  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [showClearConfirm, setShowClearConfirm] = useState(false)
  const [feedback, setFeedback] = useState<{ ok: boolean; message: string } | null>(null)

  useEffect(() => {
    let alive = true
    void getLayaConfig()
      .then(config => {
        if (!alive || !config) return
        setBaseUrl(config.base_url || DEFAULT_BASE_URL)
        setModel(config.model || '')
        setEnabled(Boolean(config.enabled))
        setHasKey(Boolean(config.has_key))
        setTimeoutMs(String(config.timeout_ms ?? DEFAULT_TIMEOUT_MS))
        setMaxRetries(String(config.max_retries ?? DEFAULT_MAX_RETRIES))
        setFallbackToPrimaryModel(config.fallback_to_primary_model !== false)
      })
      .catch(() => {
        // 读取失败不打断首次配置：默认值本身就是可用的初始状态
        // （本机 8000、无 Key、10s 超时）。真的用不了会在保存或测试时暴露，
        // 那时用户已经明确表达了意图，报错才有意义。
      })
    return () => {
      alive = false
    }
  }, [])

  const validate = (): boolean => {
    if (!baseUrl.trim()) {
      setFeedback({ ok: false, message: TXT.errNeedBaseUrl })
      return false
    }
    const timeout = Number(timeoutMs)
    if (!Number.isFinite(timeout) || timeout < 100 || timeout > 120_000) {
      setFeedback({ ok: false, message: TXT.errTimeoutRange })
      return false
    }
    const retries = Number(maxRetries)
    if (!Number.isInteger(retries) || retries < 0 || retries > 10) {
      setFeedback({ ok: false, message: TXT.errRetriesRange })
      return false
    }
    return true
  }

  const save = async (): Promise<void> => {
    setFeedback(null)
    if (!validate()) return
    setSaving(true)
    try {
      const config = await saveLayaConfig({
        apiKey: apiKey.trim() || undefined,
        baseUrl: baseUrl.trim(),
        model: model.trim(),
        enabled,
        timeoutMs: Number(timeoutMs),
        maxRetries: Number(maxRetries),
        fallbackToPrimaryModel,
      })
      if (!config) throw new Error(TXT.errNoConfig)
      setHasKey(Boolean(config.has_key))
      setApiKey('')
      setFeedback({ ok: true, message: TXT.saveOk })
    } catch (error) {
      setFeedback({ ok: false, message: String(error) })
    } finally {
      setSaving(false)
    }
  }

  const test = async (): Promise<void> => {
    setFeedback(null)
    if (!validate()) return
    setTesting(true)
    try {
      const result = await testLayaConnection()
      if (!result) throw new Error(TXT.errNoTestResult)
      setFeedback({ ok: connectionOk(result), message: connectionMessage(result, TXT) })
    } catch (error) {
      setFeedback({ ok: false, message: String(error) })
    } finally {
      setTesting(false)
    }
  }

  const clear = async (): Promise<void> => {
    setClearing(true)
    try {
      await clearLayaApiKey()
      setHasKey(false)
      setApiKey('')
      setShowClearConfirm(false)
      setFeedback({ ok: true, message: TXT.clearOk })
    } catch (error) {
      setFeedback({ ok: false, message: String(error) })
    } finally {
      setClearing(false)
    }
  }

  return (
    <>
      <div className="decision-settings-summary">
        <span className={`decision-settings-dot${enabled ? ' is-ready' : ''}`} />
        <div>
          <strong>{enabled ? TXT.enabledState : TXT.disabledState}</strong>
          <p>{TXT.notTestedNote}</p>
          <p>{TXT.exclusiveNote}</p>
        </div>
      </div>

      <FormRow
        label={TXT.enableLabel}
        hint={TXT.enableHint}
        control={
          <label className="decision-fallback-toggle">
            <input
              type="checkbox"
              checked={enabled}
              disabled={loading}
              onChange={event => setEnabled(event.target.checked)}
              aria-label={TXT.enableAria}
            />
            <span>{enabled ? TXT.enabledState : TXT.offState}</span>
          </label>
        }
      />

      <FormRow
        stacked
        label={TXT.baseUrlLabel}
        hint={TXT.baseUrlHint}
        control={
          <input
            className="compact-input"
            value={baseUrl}
            disabled={loading}
            onChange={event => setBaseUrl(event.target.value)}
            aria-label={TXT.baseUrlAria}
          />
        }
      />

      <FormRow
        stacked
        label={TXT.modelLabel}
        hint={TXT.modelHint}
        control={
          <input
            className="compact-input"
            value={model}
            disabled={loading}
            onChange={event => setModel(event.target.value)}
            placeholder={TXT.modelPlaceholder}
            aria-label={TXT.modelAria}
          />
        }
      />

      <FormRow
        stacked
        label={
          <span className="models-field-label">
            <IconPlug size={12} className="icon-prefix" /> API Key
            {hasKey && <span className="model-badge label-badge">{TXT.configured}</span>}
          </span>
        }
        hint={TXT.apiKeyHint}
        control={
          <div className="models-key-row">
            <div className="models-key-field">
              <input
                className="compact-input"
                type={showKey ? 'text' : 'password'}
                value={apiKey}
                disabled={loading}
                onChange={event => setApiKey(event.target.value)}
                placeholder={hasKey ? TXT.apiKeyPlaceholderSet : TXT.apiKeyPlaceholderEmpty}
                aria-label={TXT.apiKeyAria}
              />
              <button
                type="button"
                className="models-key-eye"
                onClick={() => setShowKey(value => !value)}
                tabIndex={-1}
                title={showKey ? TXT.showKey : TXT.hideKey}
                aria-label={showKey ? TXT.hideKeyAria : TXT.showKeyAria}
              >
                {showKey ? <IconEyeOff size={13} /> : <IconEye size={13} />}
              </button>
            </div>
            {hasKey && (
              <button
                type="button"
                className="models-key-clear"
                onClick={() => setShowClearConfirm(true)}
                disabled={clearing}
                title={TXT.clearKeyTitle}
                aria-label={TXT.clearKeyAria}
              >
                <IconBrushCleaning size={13} />
              </button>
            )}
          </div>
        }
      />

      <div className="decision-policy-grid">
        <FormRow
          stacked
          label={TXT.timeoutLabel}
          hint={TXT.timeoutHint}
          control={
            <input
              className="compact-input"
              type="number"
              min={100}
              max={120000}
              step={1000}
              value={timeoutMs}
              disabled={loading}
              onChange={event => setTimeoutMs(event.target.value)}
              aria-label={TXT.timeoutAria}
            />
          }
        />

        <FormRow
          stacked
          label={TXT.retriesLabel}
          hint={TXT.retriesHint}
          control={
            <input
              className="compact-input"
              type="number"
              min={0}
              max={10}
              step={1}
              value={maxRetries}
              disabled={loading}
              onChange={event => setMaxRetries(event.target.value)}
              aria-label={TXT.retriesAria}
            />
          }
        />
      </div>

      <FormRow
        label={TXT.fallbackLabel}
        hint={TXT.fallbackHint}
        control={
          <label className="decision-fallback-toggle">
            <input
              type="checkbox"
              checked={fallbackToPrimaryModel}
              disabled={loading}
              onChange={event => setFallbackToPrimaryModel(event.target.checked)}
              aria-label={TXT.fallbackAria}
            />
            <span>{fallbackToPrimaryModel ? TXT.enabledState : TXT.offState}</span>
          </label>
        }
      />

      <div className="decision-settings-actions">
        <Button variant="primary" size="sm" loading={saving} onClick={() => void save()}>
          <IconPlug size={13} />
          {TXT.saveBtn}
        </Button>
        <Button variant="ghost" size="sm" loading={testing} onClick={() => void test()}>
          <IconPlug size={13} />
          {TXT.testBtn}
        </Button>
        {feedback && (
          <span
            role="status"
            className={
              feedback.ok
                ? 'decision-settings-feedback is-ok'
                : 'decision-settings-feedback is-error'
            }
          >
            {feedback.message}
          </span>
        )}
      </div>

      <CompactModal
        open={showClearConfirm}
        onClose={() => setShowClearConfirm(false)}
        title={TXT.clearModalTitle}
        size="sm"
        footer={
          <>
            <Button variant="ghost" size="sm" onClick={() => setShowClearConfirm(false)}>
              {TXT.cancel}
            </Button>
            <Button variant="danger" size="sm" loading={clearing} onClick={() => void clear()}>
              {TXT.confirmClear}
            </Button>
          </>
        }
      >
        <p>{TXT.clearModalBody}</p>
      </CompactModal>
    </>
  )
}
