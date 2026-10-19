/**
 * inputFocus 单测 —— 「入栏/追加后聚焦输入框、光标置末尾」的语义钉死。
 * 两条链路（append-to-chat / 引用入栏）都经此实现，回归即双链路同时回归。
 */
import { describe, expect, it } from 'vitest'
import { focusInputAtEnd } from './inputFocus'

describe('focusInputAtEnd', () => {
  it('聚焦输入框并把光标移到末尾', () => {
    const ta = document.createElement('textarea')
    ta.value = '已输入的追问'
    document.body.appendChild(ta)

    focusInputAtEnd(ta)

    expect(document.activeElement).toBe(ta)
    expect(ta.selectionStart).toBe(ta.value.length)
    expect(ta.selectionEnd).toBe(ta.value.length)
    ta.remove()
  })

  it('空输入框：焦点在手、光标归零（不抛）', () => {
    const ta = document.createElement('textarea')
    document.body.appendChild(ta)

    focusInputAtEnd(ta)

    expect(document.activeElement).toBe(ta)
    expect(ta.selectionStart).toBe(0)
    ta.remove()
  })

  it('null 引用安全（组件已卸载等场景 no-op）', () => {
    expect(() => focusInputAtEnd(null)).not.toThrow()
  })
})
