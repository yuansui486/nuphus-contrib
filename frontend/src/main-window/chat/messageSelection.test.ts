/**
 * 选中文字 → 引用追问 的纯逻辑单测
 *
 * 直 import 真实实现（不复刻判定），避免「测试通过而线上行为漂移」。
 * DOM 相关断言只钉 `.message-content` 这一个契约——消息气泡正文的类名由
 * ChatPanel 渲染，改类名必须同步这里。
 */
import { afterEach, describe, expect, it } from 'vitest'
import {
  buildQuoteRef,
  isSelectableInBubble,
  QUOTE_MAX_CHARS,
  quoteId,
  truncateQuote,
} from './messageSelection'

afterEach(() => {
  document.body.innerHTML = ''
})

describe('truncateQuote', () => {
  it('未超上限 → 原样返回，仅 trim', () => {
    expect(truncateQuote('  引用这段话  ')).toEqual({ text: '引用这段话', truncated: false })
  })

  it('空串 / 纯空白 → 空 text 且不标截断（可用性交由 buildQuoteRef 判）', () => {
    expect(truncateQuote('')).toEqual({ text: '', truncated: false })
    expect(truncateQuote('   \n\t ')).toEqual({ text: '', truncated: false })
  })

  it('正好等于上限 → 不标截断', () => {
    const exact = 'a'.repeat(QUOTE_MAX_CHARS)
    expect(truncateQuote(exact)).toEqual({ text: exact, truncated: false })
  })

  it('超上限 → 截到 max 且标 truncated（不让模型把片段当全文）', () => {
    const r = truncateQuote('b'.repeat(QUOTE_MAX_CHARS + 10))
    expect(r.truncated).toBe(true)
    expect(r.text).toHaveLength(QUOTE_MAX_CHARS)
  })

  it('自定义 max 生效', () => {
    expect(truncateQuote('一二三四五', 3)).toEqual({ text: '一二三', truncated: true })
  })
})

describe('quoteId', () => {
  it('同内容同 id（引用同一段时不重复入栏）', () => {
    expect(quoteId('同一段话')).toBe(quoteId('同一段话'))
  })

  it('不同内容不同 id（否则 addReference 的去重键会吞掉第二段引用）', () => {
    expect(quoteId('第一段')).not.toBe(quoteId('第二段'))
  })

  it('前缀 q + base36，且空串也有稳定 id', () => {
    expect(quoteId('abc')).toMatch(/^q[0-9a-z]+$/)
    expect(quoteId('')).toBe(quoteId(''))
  })
})

describe('isSelectableInBubble', () => {
  /** 造一个与 ChatPanel 同构的气泡：.message-bubble > .message-content.user > p */
  function bubble(): { root: HTMLElement; content: HTMLElement; header: HTMLElement } {
    const root = document.createElement('div')
    root.className = 'message-bubble user'
    const header = document.createElement('div')
    header.className = 'message-header'
    const content = document.createElement('div')
    content.className = 'message-content user'
    const p = document.createElement('p')
    p.textContent = '正文'
    content.appendChild(p)
    root.append(header, content)
    document.body.appendChild(root)
    return { root, content, header }
  }

  it('null → false', () => {
    expect(isSelectableInBubble(null)).toBe(false)
  })

  it('正文内的元素 → true', () => {
    const { content } = bubble()
    expect(isSelectableInBubble(content.querySelector('p'))).toBe(true)
  })

  it('正文内的文本节点 → true（Selection.anchorNode 常是文本节点）', () => {
    const { content } = bubble()
    expect(isSelectableInBubble(content.querySelector('p')!.firstChild)).toBe(true)
  })

  it('气泡内但不在正文（头部 / 时间戳）→ false', () => {
    const { header } = bubble()
    expect(isSelectableInBubble(header)).toBe(false)
  })

  it('消息区之外（侧栏 / 输入框）→ false', () => {
    const outside = document.createElement('textarea')
    document.body.appendChild(outside)
    expect(isSelectableInBubble(outside)).toBe(false)
  })
})

describe('buildQuoteRef', () => {
  it('空选区 → null（不弹浮条、不入引用栏）', () => {
    expect(buildQuoteRef('   ')).toBeNull()
  })

  it('短文本 → label 等于原文，id 由内容派生', () => {
    const ref = buildQuoteRef('  第二段建议  ')
    expect(ref).not.toBeNull()
    expect(ref!.type).toBe('quote')
    expect(ref!.label).toBe('第二段建议')
    expect(ref!.id).toBe(quoteId('第二段建议'))
  })

  it('超长文本 → label 带截断标记，id 基于截断后的文本', () => {
    const ref = buildQuoteRef('c'.repeat(QUOTE_MAX_CHARS + 5))
    expect(ref!.label.endsWith('…（已截断）')).toBe(true)
    expect(ref!.label.startsWith('c'.repeat(QUOTE_MAX_CHARS))).toBe(true)
    expect(ref!.id).toBe(quoteId('c'.repeat(QUOTE_MAX_CHARS)))
  })

  it('两次引用同一段 → id 一致，addReference 去重后只入栏一次', () => {
    expect(buildQuoteRef('同一段')!.id).toBe(buildQuoteRef('同一段')!.id)
  })
})
