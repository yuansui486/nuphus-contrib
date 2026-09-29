import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { TaskBubble } from './TaskBubble'
import type { TaskRun } from '../../core/types'

/**
 * 回归钉：任务面板「点一行弹出该任务详情」。
 *
 * 钉住三件事：
 *  1. 入口在**行上**（不是二级菜单/溢出入口）：panel 内任一行点击即弹；
 *  2. 弹窗内容 = 标题 / 状态 / 元信息 / markdown 摘要——summary 走既有
 *     MarkdownContent（渲染成 DOM 元素，不是把源码当纯文本拍出来）；
 *  3. summary 缺位（null / 空串 / 纯空白 / running 未结算）时正文有兜底文案，
 *     绝不出现空正文区。
 *
 * 另钉关闭闭环：遮罩点击、Esc、关闭按钮三条路径都把弹窗收掉，且面板本体
 * （header / progress / 列表）不随之消失。
 */

vi.mock('./PreviewOverlay', () => ({ PreviewOverlay: () => null }))

function run(over: Partial<TaskRun> = {}): TaskRun {
  return {
    run_id: 'run-1',
    title: '整理本周会议纪要',
    task: '## 任务\n整理本周会议纪要并输出 Markdown',
    goal_type: 'file_operation',
    origin: null,
    attempt: 1,
    state: 'completed',
    started_at: 1000,
    settled_at: 9000,
    duration_ms: 8200,
    ok: true,
    summary: null,
    ...over,
  }
}

function mountPanel(runs: TaskRun[]) {
  const onClose = vi.fn()
  render(<TaskBubble visible runs={runs} onClose={onClose} />)
  return { onClose }
}

/** 面板里的行（整行即详情入口） */
const rowOf = (title: string) => screen.getByRole('button', { name: title })

afterEach(() => {
  cleanup()
})

describe('任务面板 → 任务详情弹窗', () => {
  it('点面板任一行 → 弹出该任务的标题/状态/元信息/markdown 摘要', async () => {
    mountPanel([
      run({
        title: '整理本周会议纪要',
        state: 'completed',
        attempt: 2,
        goal_type: 'file_operation',
        origin: { plan_path: 'C:/p/.plan.md', task_no: 3 },
        duration_ms: 8200,
        summary: [
          '# 已整理 3 份纪要',
          '',
          '输出到 `notes/` 目录',
          '',
          '- 周一：排期确认',
          '- 周三：评审会议',
        ].join('\n'),
      }),
    ])

    // 点开前没有弹窗
    expect(screen.queryByText('交付内容')).not.toBeInTheDocument()

    fireEvent.click(rowOf('整理本周会议纪要'))

    // 标题 + 状态 + 元信息（第几次执行 / 耗时 / goal_type / 计划归属）。
    // 查询限定在弹窗内：标题在面板行与弹窗各出现一次（面板是原文案）
    const modal = within(document.querySelector('.pm-content') as HTMLElement)
    expect(modal.getByText('整理本周会议纪要')).toBeInTheDocument()
    expect(modal.getByText('完成')).toBeInTheDocument()
    expect(modal.getByText('第 2 次执行')).toBeInTheDocument()
    expect(modal.getByText('8.2s')).toBeInTheDocument()
    expect(modal.getByText('file_operation')).toBeInTheDocument()
    expect(modal.getByText('计划#3')).toBeInTheDocument()

    // 派发正文按 markdown 渲染（点开就知道这笔派了什么）
    const taskMd = document.querySelector('.task-detail-task') as HTMLElement
    expect(taskMd).not.toBeNull()
    expect(taskMd.querySelectorAll('.markdown-h').length).toBe(1)
    expect(taskMd.textContent).toContain('整理本周会议纪要并输出 Markdown')

    // 交付全文按 markdown 渲染（标题/行内代码/列表都是真 DOM 元素，不是源码文本）
    const md = document.querySelector('.task-detail-delivery') as HTMLElement
    expect(md).not.toBeNull()
    expect(md.querySelectorAll('.markdown-h').length).toBe(1)
    expect(md.textContent).toContain('notes/')
    expect(md.querySelectorAll('.markdown-li').length).toBe(2)
    // 渲染出来的 DOM 里不应残留 markdown 源码记号
    expect(md.textContent).not.toContain('# 已整理')
    expect(md.textContent).not.toContain('`notes/`')
  })

  it('弹窗挂在 body 上（portal），不受面板 backdrop-filter 劫持定位基准', () => {
    mountPanel([run({ title: 'opod', summary: 'ok' })])
    fireEvent.click(rowOf('opod'))

    const wrapper = document.querySelector('.pm-wrapper') as HTMLElement
    expect(wrapper).not.toBeNull()
    // portal 到 body：父节点不是 .task-track（否则 fixed 会被面板的 blur 劫持）
    expect(wrapper.parentElement).toBe(document.body)
    // 复用 planner 弹窗的骨架类
    expect(document.querySelector('.pm-backdrop')).not.toBeNull()
    expect(document.querySelector('.pm-content')).not.toBeNull()
    expect(document.querySelector('.pm-header')).not.toBeNull()
    expect(document.querySelector('.planner-body')).not.toBeNull()
  })

  it('键盘可用：行聚焦后 Enter / Space 都能打开详情', () => {
    mountPanel([run({ title: '键盘入口', summary: 'x' })])
    const row = rowOf('键盘入口')

    expect(row).toHaveAttribute('tabindex', '0')
    expect(row).toHaveAttribute('role', 'button')

    fireEvent.keyDown(row, { key: 'Enter' })
    const modal = within(document.querySelector('.pm-content') as HTMLElement)
    expect(modal.getByText('交付内容')).toBeInTheDocument()

    fireEvent.keyDown(window, { key: 'Escape' })
    fireEvent.keyDown(row, { key: ' ' })
    // Esc 只收起弹窗；Space 重新打开后仍是同一条目的内容
    expect(
      within(document.querySelector('.pm-content') as HTMLElement).getByText('键盘入口'),
    ).toBeInTheDocument()
  })

  it.each([
    ['summary 为 null', null],
    ['summary 为空串', ''],
    ['summary 全是空白', '   \n  '],
    ['running 未结算（后端只在该态写 null）', null],
  ])('正文兜底：%s', (_label, summary) => {
    mountPanel([
      run({
        title: '没有交付的任务',
        summary,
        // 隔离变量：不留派发正文，专测交付区兜底
        task: '',
        state: summary === null ? 'running' : 'completed',
      }),
    ])
    fireEvent.click(rowOf('没有交付的任务'))

    const modal = within(document.querySelector('.pm-content') as HTMLElement)
    // running → 「执行中」；终态空交付 → 「没有留下交付内容」——两档不许混
    expect(
      modal.getByText(
        summary === null ? '任务执行中，完成后在此显示交付内容' : '该任务没有留下交付内容',
      ),
    ).toBeInTheDocument()
    // 兜底态不渲染 markdown 容器（不会出现空 markdown 盒子）
    expect(document.querySelector('.task-detail-markdown')).toBeNull()
  })

  // 关窗三条路径逐条独立验证：每条的起点都是「弹窗开着」，
  // 且都只该收起弹窗、面板原样不动
  it.each([
    [
      '点遮罩',
      (node: HTMLElement) => fireEvent.click(node.querySelector('.pm-backdrop') as HTMLElement),
    ],
    ['按 Esc', () => fireEvent.keyDown(window, { key: 'Escape' })],
    [
      '点关闭钮',
      (node: HTMLElement) => fireEvent.click(within(node).getByRole('button', { name: '关闭' })),
    ],
  ])('%s → 收起弹窗且面板不受影响', async (_label, closeModal) => {
    const { onClose } = mountPanel([run({ title: '可关的任务', summary: '结论' })])
    fireEvent.click(rowOf('可关的任务'))
    expect(screen.getByText('交付内容')).toBeInTheDocument()

    closeModal(document.querySelector('.pm-wrapper') as HTMLElement)

    // 卸载发生在 250ms 退场动画之后（先出得去，再摘 DOM）
    await waitFor(() => expect(screen.queryByText('交付内容')).not.toBeInTheDocument())

    // 面板 header / progress / 列表仍在，且关面板的回调没被误触发
    expect(screen.getByText('待命')).toBeInTheDocument()
    expect(rowOf('可关的任务')).toBeInTheDocument()
    expect(onClose).not.toHaveBeenCalled()
  })

  it('关严后弹窗内容不留 DOM（关的是这一条，不是把面板一起收掉）', async () => {
    mountPanel([run({ title: '会被关掉的任务', summary: '结论' })])
    fireEvent.click(rowOf('会被关掉的任务'))
    fireEvent.keyDown(window, { key: 'Escape' })

    await waitFor(() => {
      expect(document.querySelector('.pm-wrapper')).toBeNull()
      expect(document.querySelector('.pm-content')).toBeNull()
      expect(document.querySelector('.pm-backdrop')).toBeNull()
    })
  })

  it('interrupted 是独立状态文案，不与 failed 混', () => {
    mountPanel([
      run({ title: '被打断的任务', state: 'interrupted', ok: false, summary: '进程重启前未结算' }),
    ])
    fireEvent.click(rowOf('被打断的任务'))

    const modal = within(document.querySelector('.pm-content') as HTMLElement)
    expect(modal.getByText('中断')).toBeInTheDocument()
    expect(modal.queryByText('失败')).not.toBeInTheDocument()
    expect(modal.getByText('进程重启前未结算')).toBeInTheDocument()
  })
})
