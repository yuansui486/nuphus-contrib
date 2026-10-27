import { useCallback, useEffect, useMemo, useState } from 'react'
import { addTenet, deleteTenet, getTenets, updateTenet } from '../lib/api'
import { IconPencil, IconPlus, IconShield, IconTrash2 } from '../../ui/Icons'
import { Button } from '../../ui/Button'
import { useLanguage } from '../../locales'
import {
  TENET_PRIORITIES,
  TENETS_CHANGED_EVENT,
  filterTenetsForScope,
  normalizeTenetPriority,
  notifyTenetsChanged,
  type TenetPriority,
  type TenetProjectScope,
  type TenetRecord,
} from './tenetScope'
import '../../styles/memory-dialogs.css'
import './project-tenets.css'

interface ProjectTenetsPanelProps {
  project: TenetProjectScope
}

/**
 * 项目原则内联编辑器：创建对话 / 创建项目弹窗内直接增删改，不另开嵌套弹窗。
 * 数据按 project.path 过滤，与系统提示词的「当前项目原则」同一归属口径。
 */
export function ProjectTenetsPanel({ project }: ProjectTenetsPanelProps) {
  const { t } = useLanguage()
  const [tenets, setTenets] = useState<TenetRecord[]>([])
  const [loading, setLoading] = useState(true)
  const [adding, setAdding] = useState(false)
  const [content, setContent] = useState('')
  const [addPriority, setAddPriority] = useState<TenetPriority>('high')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editContent, setEditContent] = useState('')
  const [editPriority, setEditPriority] = useState<TenetPriority>('high')
  const [busyId, setBusyId] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const visible = useMemo(() => filterTenetsForScope(tenets, project), [tenets, project])

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const res = await getTenets()
      setTenets(res?.items || [])
    } catch (e) {
      console.error('Failed to load project tenets:', e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    setAdding(false)
    setContent('')
    setEditingId(null)
    void load()
  }, [load, project.path])

  useEffect(() => {
    const onChanged = () => {
      void load()
    }
    window.addEventListener(TENETS_CHANGED_EVENT, onChanged)
    return () => window.removeEventListener(TENETS_CHANGED_EVENT, onChanged)
  }, [load])

  const handleAdd = useCallback(async () => {
    if (!content.trim() || saving) return
    setSaving(true)
    try {
      await addTenet(content.trim(), addPriority, project.path)
      setContent('')
      setAdding(false)
      setAddPriority('high')
      notifyTenetsChanged()
    } catch (e) {
      console.error('Failed to add project tenet:', e)
    } finally {
      setSaving(false)
    }
  }, [addPriority, content, project.path, saving])

  const handleDelete = useCallback(async (id: string) => {
    setBusyId(id)
    try {
      await deleteTenet(id)
      notifyTenetsChanged()
    } catch (e) {
      console.error('Failed to delete project tenet:', e)
    } finally {
      setBusyId(null)
    }
  }, [])

  const startEdit = useCallback((tenet: TenetRecord) => {
    setEditingId(tenet.id)
    setEditContent(tenet.content)
    setEditPriority(normalizeTenetPriority(tenet.priority))
  }, [])

  const cancelEdit = useCallback(() => {
    setEditingId(null)
    setEditContent('')
    setEditPriority('high')
  }, [])

  const saveEdit = useCallback(async () => {
    if (!editingId || !editContent.trim()) return
    setBusyId(editingId)
    try {
      await updateTenet(editingId, editContent.trim(), editPriority)
      cancelEdit()
      notifyTenetsChanged()
    } catch (e) {
      console.error('Failed to update project tenet:', e)
    } finally {
      setBusyId(null)
    }
  }, [cancelEdit, editContent, editPriority, editingId])

  const handlePriorityChange = useCallback(
    async (tenet: TenetRecord, priority: TenetPriority) => {
      if (editingId === tenet.id) {
        setEditPriority(priority)
        return
      }
      setBusyId(tenet.id)
      try {
        await updateTenet(tenet.id, tenet.content, priority)
        notifyTenetsChanged()
      } catch (e) {
        console.error('Failed to update project tenet priority:', e)
      } finally {
        setBusyId(null)
      }
    },
    [editingId],
  )

  return (
    <section className="pt-panel" aria-label={t('memory.tenet.projectTitle')}>
      <div className="pt-head">
        <span className="pt-title">
          <IconShield size={14} />
          {t('memory.tenet.projectTitle')}
        </span>
        <span className="pt-count">{visible.length}</span>
        <Button
          variant="default"
          size="sm"
          onClick={() => setAdding(value => !value)}
          title={t('memory.tenet.addProject')}
        >
          <IconPlus size={13} />
          {t('common.add')}
        </Button>
      </div>
      <div className="pt-guide">{t('memory.tenet.projectGuide', project.name)}</div>

      {adding && (
        <div className="pt-form">
          <span className="td-badge-project">{project.name}</span>
          <textarea
            className="memdlg-textarea"
            rows={3}
            value={content}
            autoFocus
            placeholder={t('memory.tenet.placeholder')}
            aria-label={t('memory.tenet.content')}
            onChange={e => setContent(e.target.value)}
          />
          <div className="pt-priority-row">
            <span className="memdlg-label">{t('memory.tenet.priority')}</span>
            <select
              className="memdlg-select"
              value={addPriority}
              aria-label={t('memory.tenet.priority')}
              onChange={e => setAddPriority(normalizeTenetPriority(e.target.value))}
            >
              {TENET_PRIORITIES.map(priority => (
                <option key={priority} value={priority}>
                  {t(`memory.tenet.priority.${priority}`)}
                </option>
              ))}
            </select>
          </div>
          <div className="pt-form-actions">
            <Button
              variant="default"
              size="sm"
              onClick={() => {
                setAdding(false)
                setContent('')
              }}
            >
              {t('common.cancel')}
            </Button>
            <Button
              variant="primary"
              size="sm"
              loading={saving}
              disabled={!content.trim()}
              onClick={() => void handleAdd()}
            >
              {t('common.save')}
            </Button>
          </div>
        </div>
      )}

      {loading ? (
        <div className="pt-empty">{t('common.loading')}</div>
      ) : visible.length === 0 ? (
        <div className="pt-empty">{t('memory.tenet.projectEmpty')}</div>
      ) : (
        <div className="pt-list">
          {visible.map(tenet => {
            const editing = editingId === tenet.id
            return (
              <div key={tenet.id} className="pt-item">
                <div className="pt-item-head">
                  <div className="td-card-meta">
                    <span className="td-badge-project" title={project.path}>
                      {project.name}
                    </span>
                    <select
                      className="td-priority-select"
                      aria-label={t('memory.tenet.priority')}
                      title={t('memory.tenet.priority')}
                      value={editing ? editPriority : normalizeTenetPriority(tenet.priority)}
                      disabled={busyId === tenet.id}
                      onChange={e =>
                        void handlePriorityChange(tenet, normalizeTenetPriority(e.target.value))
                      }
                    >
                      {TENET_PRIORITIES.map(priority => (
                        <option key={priority} value={priority}>
                          {t(`memory.tenet.priority.${priority}`)}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div className="td-card-actions">
                    {!editing && (
                      <button
                        type="button"
                        className="memdlg-icon-btn"
                        title={t('memory.tenet.edit')}
                        aria-label={t('memory.tenet.edit')}
                        disabled={busyId === tenet.id}
                        onClick={() => startEdit(tenet)}
                      >
                        <IconPencil size={13} />
                      </button>
                    )}
                    <button
                      type="button"
                      className="memdlg-icon-btn danger"
                      title={t('common.delete')}
                      aria-label={t('common.delete')}
                      disabled={busyId === tenet.id}
                      onClick={() => void handleDelete(tenet.id)}
                    >
                      <IconTrash2 size={13} />
                    </button>
                  </div>
                </div>
                {editing ? (
                  <div className="pt-item-edit">
                    <textarea
                      className="memdlg-textarea"
                      rows={3}
                      value={editContent}
                      aria-label={t('memory.tenet.content')}
                      onChange={e => setEditContent(e.target.value)}
                      autoFocus
                    />
                    <div className="pt-form-actions">
                      <Button
                        variant="default"
                        size="sm"
                        disabled={busyId === tenet.id}
                        onClick={cancelEdit}
                      >
                        {t('common.cancel')}
                      </Button>
                      <Button
                        variant="primary"
                        size="sm"
                        loading={busyId === tenet.id}
                        disabled={!editContent.trim()}
                        onClick={() => void saveEdit()}
                      >
                        {t('common.save')}
                      </Button>
                    </div>
                  </div>
                ) : (
                  <span className="pt-item-body">{tenet.content}</span>
                )}
              </div>
            )
          })}
        </div>
      )}
    </section>
  )
}
