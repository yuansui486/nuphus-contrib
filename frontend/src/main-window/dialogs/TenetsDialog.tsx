import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  addTenet,
  deleteTenet,
  getProjectBookmarks,
  getProjectDir,
  getTenets,
  updateTenet,
} from '../lib/api'
import { IconPencil, IconPlus, IconShield, IconTrash2, IconX } from '../../ui/Icons'
import { Button, IconButton } from '../../ui/Button'
import { useLanguage } from '../../locales'
import {
  TENET_PRIORITIES,
  TENETS_CHANGED_EVENT,
  filterTenetsForScope,
  normalizeTenetPriority,
  notifyTenetsChanged,
  tenetProjectLabel,
  type TenetPriority,
  type TenetProjectScope,
  type TenetRecord,
} from './tenetScope'
import '../../styles/memory-dialogs.css'

interface TenetsDialogProps {
  onClose: () => void
  /** 缺省 = 管理全部原则；传入项目 = 仅管理该项目的原则 */
  project?: TenetProjectScope
}

/**
 * 教导原则管理弹窗：guide 行 + 原则卡片列表 + 新增子弹窗。
 * 弹窗骨架复用全局 modal-* 体系；数据直连后端 getTenets / addTenet / updateTenet / deleteTenet。
 */
export function TenetsDialog({ onClose, project }: TenetsDialogProps) {
  const { t } = useLanguage()
  const [tenets, setTenets] = useState<TenetRecord[]>([])
  const [projects, setProjects] = useState<TenetProjectScope[]>([])
  const [loading, setLoading] = useState(true)
  const [addOpen, setAddOpen] = useState(false)
  const [content, setContent] = useState('')
  const [addPriority, setAddPriority] = useState<TenetPriority>('high')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editContent, setEditContent] = useState('')
  const [editPriority, setEditPriority] = useState<TenetPriority>('high')
  const [busyId, setBusyId] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const visible = useMemo(
    () => (project ? filterTenetsForScope(tenets, project) : tenets),
    [project, tenets],
  )
  const title = project
    ? t('memory.tenet.projectTitleWithName', project.name)
    : t('memory.tenet.allTitle')
  const guide = project ? t('memory.tenet.projectGuide', project.name) : t('memory.tenet.allGuide')

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const [res, bookmarks, active] = await Promise.all([
        getTenets(),
        getProjectBookmarks().catch(() => []),
        getProjectDir().catch(() => null),
      ])
      setTenets(res?.items || [])
      const known: TenetProjectScope[] = (bookmarks || [])
        .filter(bookmark => bookmark?.path)
        .map(bookmark => ({ name: bookmark.name || '', path: bookmark.path }))
      if (active?.path && !known.some(item => item.path === active.path)) {
        known.push({ name: active.name || active.path, path: active.path })
      }
      setProjects(known)
    } catch (e) {
      console.error('Failed to load tenets:', e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    const onChanged = () => {
      void load()
    }
    window.addEventListener(TENETS_CHANGED_EVENT, onChanged)
    return () => window.removeEventListener(TENETS_CHANGED_EVENT, onChanged)
  }, [load])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (addOpen) setAddOpen(false)
        else if (editingId) setEditingId(null)
        else onClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [addOpen, editingId, onClose])

  const handleAdd = useCallback(async () => {
    if (!content.trim()) return
    setSaving(true)
    try {
      await addTenet(content.trim(), addPriority, project?.path)
      setAddOpen(false)
      setContent('')
      setAddPriority('high')
      notifyTenetsChanged()
    } catch (e) {
      console.error('Failed to add tenet:', e)
    } finally {
      setSaving(false)
    }
  }, [addPriority, content, project?.path])

  const handleDelete = useCallback(async (id: string) => {
    setBusyId(id)
    try {
      await deleteTenet(id)
      notifyTenetsChanged()
    } catch (e) {
      console.error('Failed to delete tenet:', e)
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
      console.error('Failed to update tenet:', e)
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
        console.error('Failed to update tenet priority:', e)
      } finally {
        setBusyId(null)
      }
    },
    [editingId],
  )

  const scopeName = useCallback(
    (tenet: TenetRecord) =>
      tenetProjectLabel(tenet.project_path, projects) ?? t('memory.tenet.generalTitle'),
    [projects, t],
  )

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal-content"
        role="dialog"
        aria-label={title}
        onClick={e => e.stopPropagation()}
      >
        <div className="modal-header">
          <span className="modal-title">{title}</span>
          <IconButton variant="modal-close" label={t('common.close')} onClick={onClose}>
            <IconX size={16} />
          </IconButton>
        </div>

        <div className="modal-body">
          <div className="memdlg-intro">
            <span className="memdlg-intro-icon" aria-hidden>
              <IconShield size={16} />
            </span>
            <span className="memdlg-intro-text">
              <span className="memdlg-intro-title">{title}</span>
              <span className="memdlg-guide-text">{guide}</span>
            </span>
            <Button variant="primary" size="sm" onClick={() => setAddOpen(true)}>
              <IconPlus size={14} />
              {t('common.add')}
            </Button>
          </div>

          {loading ? (
            <div className="memdlg-empty">{t('common.loading')}</div>
          ) : visible.length === 0 ? (
            <div className="memdlg-empty">
              {project ? t('memory.tenet.projectEmpty') : t('memory.noTenets')}
            </div>
          ) : (
            <div className="memdlg-list">
              {visible.map(tenet => {
                const editing = editingId === tenet.id
                return (
                  <div key={tenet.id} className="td-card">
                    <div className="td-card-header">
                      <div className="td-card-meta">
                        <span className="td-badge-project" title={tenet.project_path || undefined}>
                          {scopeName(tenet)}
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
                            <IconPencil size={14} />
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
                          <IconTrash2 size={14} />
                        </button>
                      </div>
                    </div>
                    {editing ? (
                      <div className="td-card-edit">
                        <textarea
                          className="memdlg-textarea"
                          rows={4}
                          value={editContent}
                          aria-label={t('memory.tenet.content')}
                          onChange={e => setEditContent(e.target.value)}
                          autoFocus
                        />
                        <div className="td-card-edit-actions">
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
                      <div className="td-card-body">{tenet.content}</div>
                    )}
                  </div>
                )
              })}
            </div>
          )}
        </div>
      </div>

      {addOpen && (
        <div className="modal-overlay" style={{ zIndex: 1100 }} onClick={() => setAddOpen(false)}>
          <div
            className="modal-content memdlg-modal-sm"
            role="dialog"
            aria-label={t('memory.tenet.add')}
            onClick={e => e.stopPropagation()}
          >
            <div className="modal-header">
              <span className="modal-title">{t('memory.tenet.add')}</span>
              <IconButton
                variant="modal-close"
                label={t('common.close')}
                onClick={() => setAddOpen(false)}
              >
                <IconX size={16} />
              </IconButton>
            </div>
            <div className="modal-body memdlg-form-body">
              <div className="memdlg-form-group">
                <span className="memdlg-label">{t('memory.tenet.scope')}</span>
                <span className="td-badge-project td-badge-project--form">
                  {project ? project.name : t('memory.tenet.generalTitle')}
                </span>
              </div>
              <div className="memdlg-form-group">
                <label className="memdlg-label">
                  {t('memory.tenet.content')}
                  <em className="memdlg-req">*</em>
                </label>
                <textarea
                  className="memdlg-textarea"
                  rows={5}
                  value={content}
                  onChange={e => setContent(e.target.value)}
                  placeholder={t('memory.tenet.placeholder')}
                  autoFocus
                />
              </div>
              <div className="memdlg-form-group">
                <label className="memdlg-label" htmlFor="tenet-priority">
                  {t('memory.tenet.priority')}
                </label>
                <select
                  id="tenet-priority"
                  className="memdlg-select"
                  value={addPriority}
                  onChange={e => setAddPriority(normalizeTenetPriority(e.target.value))}
                >
                  {TENET_PRIORITIES.map(priority => (
                    <option key={priority} value={priority}>
                      {t(`memory.tenet.priority.${priority}`)}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            <div className="memdlg-footer">
              <Button variant="default" onClick={() => setAddOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button
                variant="primary"
                loading={saving}
                disabled={!content.trim()}
                onClick={handleAdd}
              >
                {t('common.save')}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
