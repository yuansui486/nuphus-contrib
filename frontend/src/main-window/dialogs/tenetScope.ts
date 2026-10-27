import { dirDisplayName, normalizePathKey } from '../chat/sessionGroups'

/** 项目原则的归属项目（弹窗/编辑器只需名称与路径） */
export interface TenetProjectScope {
  name: string
  path: string
}

export const TENET_PRIORITIES = ['critical', 'high', 'medium', 'low'] as const
export type TenetPriority = (typeof TENET_PRIORITIES)[number]

export function normalizeTenetPriority(value: string | null | undefined): TenetPriority {
  return TENET_PRIORITIES.includes(value as TenetPriority) ? (value as TenetPriority) : 'high'
}

export function tenetProjectLabel(
  path: string | null | undefined,
  projects: readonly TenetProjectScope[],
): string | null {
  const key = normalizePathKey(path)
  if (!key) return null
  const hit = projects.find(project => normalizePathKey(project.path) === key)
  return hit?.name.trim() || dirDisplayName(key)
}

export const TENETS_CHANGED_EVENT = 'nuphus:tenets-changed'

export function notifyTenetsChanged() {
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new Event(TENETS_CHANGED_EVENT))
  }
}

/** 后端 get_tenets 返回的最小条目形状（前端只读展示/过滤用） */
export interface TenetRecord {
  id: string
  content: string
  project_path?: string | null
  priority: string
  enforce?: string
  active?: boolean
  created_at?: string
}

/**
 * 作用域过滤：project 缺省 = 通用原则（project_path 为空）；
 * 传入 project = 仅该路径下的项目原则。
 */
export function filterTenetsForScope<T extends { project_path?: string | null }>(
  items: readonly T[],
  project?: TenetProjectScope,
): T[] {
  if (!project) return items.filter(t => !normalizePathKey(t.project_path))
  const key = normalizePathKey(project.path)
  return items.filter(t => normalizePathKey(t.project_path) === key)
}
