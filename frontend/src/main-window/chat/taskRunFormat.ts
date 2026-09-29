import type { TaskRun } from '../../core/types'

/**
 * taskRunFormat — TaskBubble 面板行与 TaskDetailModal 详情弹窗**共用**的台账展示口径。
 *
 * 同一份数据两处渲染，格式化函数必须只有一份实现：面板行与详情弹窗对同一条目
 * 显示不同串（耗时一格 "4.2s" vs "4.20s"）就是这么漂移出来的。
 */

/** 耗时格式化。null / 未结算 → 空串（调用方据此不渲染该格） */
export function fmtDuration(ms: TaskRun['duration_ms']): string {
  if (ms == null) return ''
  if (ms < 1000) return `${ms}ms`
  const s = ms / 1000
  if (s < 60) return `${s.toFixed(1)}s`
  return `${Math.floor(s / 60)}m${Math.round(s % 60)}s`
}
