import type { TransportKind } from '../lib/api'

/** 模型协议覆盖；未覆盖时沿用服务商默认路由。 */
export function ModelProtocolSelect({
  value,
  disabled,
  onChange,
  label,
  defaultLabel,
}: {
  value?: TransportKind | null
  disabled?: boolean
  onChange: (value: TransportKind | null) => void
  label: string
  defaultLabel: string
}) {
  return (
    <select
      className="model-protocol-select"
      aria-label={label}
      value={value ?? ''}
      disabled={disabled}
      onChange={event => onChange((event.target.value || null) as TransportKind | null)}
    >
      <option value="">{defaultLabel}</option>
      <option value="chat-completions">Chat Completions</option>
      <option value="responses">Responses</option>
      <option value="anthropic">Anthropic Messages</option>
    </select>
  )
}
