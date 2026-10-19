import { lazy, Suspense, useEffect, useState } from 'react'
import { useLanguage } from '../../locales'
import { IconPalette, IconX } from '../../ui/Icons'
import { listWorkflows, wfSave } from '../lib/api'
import { scheduleIdle } from '../lib/idle'
import type { WorkflowItem } from '../../core/types'
import './workflow-workbench.css'
import { useCanvasLeaveGuard } from '../workflow-canvas/useCanvasLeaveGuard'

// 画布 = 工作流编辑器单一形态。内置工具页已移出画布、成为独立功能界面
// （入口在桌面工具条末尾），故此处不再按 tab 拆包。
const CanvasPage = lazy(() =>
  import('../workflow-canvas/CanvasPage').then(m => ({ default: m.CanvasPage })),
)

export function CanvasWorkbenchPage({
  onClose,
  workflowId: initialWorkflowId = null,
  replayRunId = null,
  onExitReplay,
}: {
  onClose: () => void
  /** 由列表入口带过来的目标工作流；为 null 时由本页自行挑选最近更新的草稿 */
  workflowId?: string | null
  replayRunId?: string | null
  onExitReplay?: () => void
}) {
  const { t } = useLanguage()
  const [workflowId, setWorkflowId] = useState<string | null>(initialWorkflowId)
  const [loading, setLoading] = useState(!initialWorkflowId)
  const [error, setError] = useState('')
  const navigation = useCanvasLeaveGuard()

  useEffect(() => {
    // 指定了目标工作流 → 直接编辑它，跳过「自选草稿」逻辑
    if (initialWorkflowId) {
      setWorkflowId(initialWorkflowId)
      setError('')
      setLoading(false)
      return
    }
    let cancelled = false
    const openWorkflowEditor = async () => {
      try {
        const workflows = (await listWorkflows()) ?? []
        const draft = workflows
          .filter((item: WorkflowItem) => item.status === 'draft')
          .sort((a: WorkflowItem, b: WorkflowItem) => b.updated_at - a.updated_at)[0]
        if (draft) {
          if (!cancelled) setWorkflowId(draft.id)
          return
        }
        const id = crypto.randomUUID()
        const result = await wfSave({
          id,
          name: '未命名工作流',
          status: 'Draft',
          steps: [],
          doc: null,
          schedule: null,
          run_history: [],
          dry_run: false,
        })
        if (!result?.saved) throw new Error('工作流创建失败')
        if (!cancelled) setWorkflowId(id)
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err))
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    void openWorkflowEditor()
    return () => {
      cancelled = true
    }
  }, [initialWorkflowId])

  // ── 空闲预取默认 tab 的 chunk ──
  // 壳挂载后等浏览器空闲再发请求，不阻塞首屏；其余 tab 不预取，
  // 用户切过去时有 Suspense fallback 兜底反馈。
  useEffect(() => {
    scheduleIdle(() => {
      void import('../workflow-canvas/CanvasPage')
    })
  }, [])

  return (
    <div className="workflow-workbench">
      <header className="workflow-workbench-header">
        <button
          type="button"
          className="workflow-workbench-close"
          onClick={() => void navigation.leave(onClose)}
          aria-label={t('common.close')}
        >
          <IconX size={16} />
        </button>
        <div className="workflow-workbench-title">
          <IconPalette size={16} />
          <span>{t('cmd.canvas')}</span>
        </div>
        <span className="workflow-workbench-drag" data-tauri-drag-region />
        {/* ── 画布主操作组挂载点（撤销 / 重做 / 保存 / 运行）──
            CanvasPage 经 portal 注入：这些是「画布级」动作，归画布 header 所有，
            不该留在它下面那层 .wfc-toolbar 内（2026-10-17 大王定）。
            用挂载点而不是把状态提升到本页：保存/运行/撤销都依赖 CanvasPage 内部
            的 IR 与历史栈，提升会把整张画布的状态机翻出来。 */}
        <div className="workflow-workbench-actions" id="workflow-workbench-actions" />
      </header>
      <main className="workflow-workbench-body">
        {loading && <div className="page-loading">{t('common.loading')}</div>}
        {!loading && error && <div className="error-banner">{error}</div>}
        {!loading && workflowId && (
          <Suspense fallback={<div className="page-loading">{t('common.loading')}</div>}>
            <CanvasPage
              key={workflowId}
              registerLeaveGuard={navigation.register}
              workflowId={workflowId}
              replayRunId={replayRunId}
              onExitReplay={onExitReplay}
              onClose={onClose}
              /* 标题旁下拉切换目标工作流：本页已持有 workflowId 与加载态，
                 直接换 id 即整页换成目标画布（CanvasPage 的加载 effect 依赖 id） */
              onSwitchWorkflow={setWorkflowId}
            />
          </Suspense>
        )}
      </main>
    </div>
  )
}
