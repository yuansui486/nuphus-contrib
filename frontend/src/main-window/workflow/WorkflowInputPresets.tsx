import { useId, useState } from 'react'
import type { WorkflowInputKind, WorkflowInputSpec } from '../../core/types'
import { useLanguage } from '../../locales'
import { Button } from '../../ui/Button'
import type { WorkflowInputEditValue, WorkflowInputsState } from './WorkflowInputsForm'

interface PresetField {
  type: WorkflowInputKind
  value: WorkflowInputEditValue
}

interface InputPreset {
  name: string
  fields: Record<string, PresetField>
}

/** Versioned, per-workflow storage; editing the form never writes to storage. */
function storageKey(workflowId: string): string {
  return `nuphus:workflow-input-presets:v1:${workflowId}`
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Reconcile saved fields with the current declaration before applying or writing them. */
function currentFields(fields: unknown, specs: WorkflowInputSpec[]): Record<string, PresetField> {
  if (!isRecord(fields)) return {}
  return Object.fromEntries(
    specs.flatMap(spec => {
      if (spec.sensitive || !Object.prototype.hasOwnProperty.call(fields, spec.name)) return []
      const field = fields[spec.name]
      const type = spec.type ?? 'string'
      if (
        !isRecord(field) ||
        field.type !== type ||
        typeof field.value !== (type === 'boolean' ? 'boolean' : 'string')
      ) {
        return []
      }
      return [[spec.name, { type, value: field.value as WorkflowInputEditValue }]]
    }),
  )
}

function readPresets(workflowId: string): InputPreset[] {
  const raw = localStorage.getItem(storageKey(workflowId))
  if (raw === null) return []
  const parsed: unknown = JSON.parse(raw)
  if (!Array.isArray(parsed)) throw new Error('Invalid workflow input presets')
  const names = new Set<string>()
  return parsed.flatMap(preset => {
    if (
      !isRecord(preset) ||
      typeof preset.name !== 'string' ||
      !preset.name.trim() ||
      !isRecord(preset.fields) ||
      names.has(preset.name)
    ) {
      return []
    }
    names.add(preset.name)
    return [{ name: preset.name, fields: preset.fields as Record<string, PresetField> }]
  })
}

interface WorkflowInputPresetsProps {
  workflowId: string
  specs: WorkflowInputSpec[]
  state: WorkflowInputsState
}

/** Both run entry points use this explicit opt-in for non-sensitive input presets. */
export function WorkflowInputPresets({ workflowId, specs, state }: WorkflowInputPresetsProps) {
  const { t } = useLanguage()
  const id = useId()
  const [snapshot, setSnapshot] = useState(() => {
    try {
      return { presets: readPresets(workflowId), error: '' }
    } catch {
      return { presets: [] as InputPreset[], error: 'workflow.presets.readError' }
    }
  })
  const [selected, setSelected] = useState('')
  const [name, setName] = useState('')
  const [notice, setNotice] = useState('')
  const canSave = specs.every(spec => spec.sensitive || !state.errors[spec.name])

  function persist(presets: InputPreset[]): boolean {
    try {
      const sanitized = presets.map(preset => ({
        name: preset.name,
        fields: currentFields(preset.fields, specs),
      }))
      if (sanitized.length) localStorage.setItem(storageKey(workflowId), JSON.stringify(sanitized))
      else localStorage.removeItem(storageKey(workflowId))
      setSnapshot({ presets: sanitized, error: '' })
      return true
    } catch {
      setSnapshot(prev => ({ ...prev, error: 'workflow.presets.writeError' }))
      setNotice('')
      return false
    }
  }

  function save() {
    const trimmed = name.trim()
    if (!trimmed || !canSave) return
    if (snapshot.presets.some(preset => preset.name === trimmed)) {
      setSnapshot(prev => ({ ...prev, error: 'workflow.presets.duplicate' }))
      setNotice('')
      return
    }
    const fields = Object.fromEntries(
      specs
        .filter(spec => !spec.sensitive)
        .map(spec => [spec.name, { type: spec.type ?? 'string', value: state.values[spec.name] }]),
    )
    if (persist([...snapshot.presets, { name: trimmed, fields }])) {
      setSelected(trimmed)
      setName('')
      setNotice('workflow.presets.saved')
    }
  }

  function apply(value: string) {
    setSelected(value)
    setNotice('')
    const preset = snapshot.presets.find(item => item.name === value)
    if (!preset) return
    const fields = currentFields(preset.fields, specs)
    state.applyPreset(
      Object.fromEntries(Object.entries(fields).map(([key, field]) => [key, field.value])),
    )
  }

  return (
    <div className="wcf-presets">
      <label className="wcf-input-label" htmlFor={`${id}-select`}>
        {t('workflow.presets.label')}
      </label>
      <div className="wcf-presets-row">
        <select
          id={`${id}-select`}
          className="wcf-input-control"
          value={selected}
          onChange={event => apply(event.target.value)}
        >
          <option value="">{t('workflow.presets.choose')}</option>
          {snapshot.presets.map(preset => (
            <option key={preset.name} value={preset.name}>
              {preset.name}
            </option>
          ))}
        </select>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={!selected}
          onClick={() => {
            if (persist(snapshot.presets.filter(preset => preset.name !== selected))) {
              setSelected('')
              setNotice('workflow.presets.deleted')
            }
          }}
        >
          {t('workflow.presets.delete')}
        </Button>
      </div>
      <div className="wcf-presets-row">
        <input
          className="wcf-input-control"
          aria-label={t('workflow.presets.name')}
          placeholder={t('workflow.presets.name')}
          value={name}
          onChange={event => setName(event.target.value)}
        />
        <Button type="button" size="sm" disabled={!name.trim() || !canSave} onClick={save}>
          {t('workflow.presets.save')}
        </Button>
      </div>
      <div className="wcf-input-hint">{t('workflow.presets.hint')}</div>
      {snapshot.error && (
        <div className="wcf-input-error" role="alert">
          {t(snapshot.error)}
        </div>
      )}
      {notice && (
        <div className="wcf-input-hint" role="status">
          {t(notice)}
        </div>
      )}
    </div>
  )
}
