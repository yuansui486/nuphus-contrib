import { describe, expect, it } from 'vitest'
import {
  filterTenetsForScope,
  normalizeTenetPriority,
  tenetProjectLabel,
} from '../main-window/dialogs/tenetScope'

const items = [
  { id: 'g', content: '通用', project_path: null, priority: 'high' },
  { id: 'a', content: 'A', project_path: 'E:\\proj\\A', priority: 'high' },
  { id: 'b', content: 'B', project_path: 'E:\\proj\\B', priority: 'high' },
  { id: 'blank', content: '空白', project_path: '  ', priority: 'high' },
]

describe('原则作用域过滤', () => {
  it('缺省项目 → 只显示通用原则（project_path 为空）', () => {
    expect(filterTenetsForScope(items).map(t => t.id)).toEqual(['g', 'blank'])
  })

  it('指定项目 → 只显示该路径的项目原则，忽略尾分隔符', () => {
    expect(
      filterTenetsForScope(items, { name: 'A', path: 'E:\\proj\\A\\' }).map(t => t.id),
    ).toEqual(['a'])
  })

  it('不匹配任何项目 → 空列表，不把通用原则混入项目视图', () => {
    expect(filterTenetsForScope(items, { name: 'C', path: 'E:\\proj\\C' }).map(t => t.id)).toEqual(
      [],
    )
  })
})

describe('原则项目名与优先级', () => {
  const projects = [
    { name: '一号项目', path: 'E:\\proj\\A' },
    { name: '无名称项目', path: 'E:\\proj\\C\\' },
  ]

  it('项目路径优先取书签名称，未命中时回退目录名', () => {
    expect(tenetProjectLabel('E:\\proj\\A\\', projects)).toBe('一号项目')
    expect(tenetProjectLabel('E:\\proj\\Z', projects)).toBe('Z')
    expect(tenetProjectLabel(null, projects)).toBeNull()
    expect(tenetProjectLabel('  ', projects)).toBeNull()
  })

  it('未知优先级回退为 high，合法值原样返回', () => {
    expect(normalizeTenetPriority('low')).toBe('low')
    expect(normalizeTenetPriority('critical')).toBe('critical')
    expect(normalizeTenetPriority('unexpected')).toBe('high')
    expect(normalizeTenetPriority(undefined)).toBe('high')
  })
})
