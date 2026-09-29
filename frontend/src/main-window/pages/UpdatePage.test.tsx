import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { getVersion } from '@tauri-apps/api/app'
import { getChangelog } from '../lib/api'
import { UpdatePage } from './UpdatePage'

// Tauri 侧依赖全部桩掉：本测试只验证「版本与更新」页面的渲染与 CHANGELOG 段落切分，
// 不触达真实 updater / IPC（jsdom 里没有 Tauri 运行时）。
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn(async () => null) }))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.2.16') }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn(async () => {}) }))
vi.mock('../lib/api', () => ({ getChangelog: vi.fn() }))

// 夹具：结构与真实 CHANGELOG.md 一致（`## [版本] - 日期` / `### 小节` / 单行条目）
const CHANGELOG = `# Changelog

## [Unreleased]

### Added
- 开发中的改动。

## [0.2.16] - 2026-09-19

### Added
- **模型刷新与官方清单同步**：点「刷新」后对齐官方 /v1/models。

### Fixed
- **DeepSeek 内置模型清单对齐**（#31）。

## [0.2.15] - 2026-09-16

### Fixed
- 上一轮的旧改动。
`

const mockedGetVersion = vi.mocked(getVersion)
const mockedGetChangelog = vi.mocked(getChangelog)

function arrange(version: string, changelog: string) {
  mockedGetVersion.mockResolvedValue(version)
  mockedGetChangelog.mockResolvedValue(changelog)
}

describe('UpdatePage 本版更新内容', () => {
  it('切出当前版本段落并渲染（含小节与条目，不含其它版本内容）', async () => {
    arrange('0.2.16', CHANGELOG)
    render(<UpdatePage />)

    expect(await screen.findByText('模型刷新与官方清单同步')).toBeInTheDocument()
    expect(screen.getByText('本版更新内容')).toBeInTheDocument()
    expect(screen.getByText('v0.2.16')).toBeInTheDocument()

    // 小节和列表保持结构，条目标题渲染为粗体。
    expect(screen.getByText('Added')).toBeInTheDocument()
    expect(screen.getByText('Fixed')).toBeInTheDocument()
    expect(screen.getByText('模型刷新与官方清单同步').tagName).toBe('STRONG')
    expect(screen.getByText('DeepSeek 内置模型清单对齐').tagName).toBe('STRONG')
    expect(screen.getAllByRole('listitem').map(item => item.textContent)).toEqual([
      '模型刷新与官方清单同步：点「刷新」后对齐官方 /v1/models。',
      'DeepSeek 内置模型清单对齐（#31）。',
    ])

    // 相邻版本的内容不进入本版区块
    expect(screen.queryByText('开发中的改动。')).toBeNull()
    expect(screen.queryByText('上一轮的旧改动。')).toBeNull()
  })

  it('渲染行内代码、强调和链接，保留代码字符与可复制的路径', async () => {
    arrange(
      '1.0.0',
      [
        '## [1.0.0]',
        '### **修复** `task_id`',
        '- **派发状态**：`agent_dispatch` 不再停在',
        '  `dispatched`；*说明*、~~旧行为~~，参见 [文档](https://example.com/docs)。',
        '- 代码 `**literal** <tag> _value_` 不应再次解析。',
        '- 路径 `src/runtime/workflow_agent.rs` 和 src/config/model.rs 保持文字。',
      ].join('\n'),
    )
    const { container } = render(<UpdatePage />)

    expect((await screen.findByText('dispatched')).tagName).toBe('CODE')
    expect(screen.getByText('agent_dispatch').tagName).toBe('CODE')
    expect(screen.getByText('task_id').tagName).toBe('CODE')
    expect(screen.getByText('修复').tagName).toBe('STRONG')
    expect(screen.getByText('派发状态').tagName).toBe('STRONG')
    expect(screen.getByText('说明').tagName).toBe('EM')
    expect(screen.getByText('旧行为').tagName).toBe('DEL')
    const literal = screen.getByText('**literal** <tag> _value_')
    expect(literal.tagName).toBe('CODE')
    expect(literal.children).toHaveLength(0)
    expect(screen.getByText('src/runtime/workflow_agent.rs').tagName).toBe('CODE')
    expect(container.querySelector('[data-file-path]')).toBeNull()
    const link = screen.getByRole('link', { name: '文档' })
    expect(link).toHaveAttribute('href', 'https://example.com/docs')
    expect(link).toHaveAttribute('target', '_blank')
    expect(link).toHaveAttribute('rel', 'noopener noreferrer')
    expect(screen.getAllByRole('listitem')[0]).toHaveTextContent(
      '派发状态：agent_dispatch 不再停在 dispatched；说明、旧行为，参见 文档。',
    )
  })

  it('HTML 与危险链接按文字显示，未闭合反引号不丢失', async () => {
    arrange(
      '1.0.0',
      [
        '## [1.0.0]',
        '- [脚本链接](javascript:alert) [数据链接](data:text/html,payload)',
        '- <img src=x onerror=alert(1)> <script>alert(1)</script>',
        '- 未闭合 `dispatched',
      ].join('\n'),
    )
    const { container } = render(<UpdatePage />)

    await screen.findByText('脚本链接')
    expect(screen.getByText('数据链接')).toBeInTheDocument()
    expect(screen.queryAllByRole('link')).toHaveLength(0)
    expect(container.querySelector('img, script')).toBeNull()
    expect(
      screen.getByText('<img src=x onerror=alert(1)> <script>alert(1)</script>'),
    ).toBeInTheDocument()
    expect(screen.getByText('未闭合 `dispatched')).toBeInTheDocument()
  })

  it('CHANGELOG 中找不到当前版本 → 显示空态，不报错、不空白', async () => {
    arrange('9.9.9', CHANGELOG)
    render(<UpdatePage />)

    expect(await screen.findByText('暂无本版变更记录')).toBeInTheDocument()
    expect(screen.queryByText('Added')).toBeNull()
  })

  it('版本段落为空（无条目）→ 同样按空态处理', async () => {
    arrange('1.0.0', '## [1.0.0] - 2026-01-01\n\n### Added\n\n')
    render(<UpdatePage />)

    expect(await screen.findByText('暂无本版变更记录')).toBeInTheDocument()
  })

  it('读取命令失败 → 显示「无法读取变更记录」，不抛异常', async () => {
    mockedGetVersion.mockResolvedValue('0.2.16')
    mockedGetChangelog.mockRejectedValue(new Error('boom'))
    render(<UpdatePage />)

    expect(await screen.findByText('无法读取变更记录')).toBeInTheDocument()
  })

  it('既有检查更新流程不受影响（无新版本 → 提示已是最新）', async () => {
    arrange('0.2.16', CHANGELOG)
    render(<UpdatePage />)

    await screen.findByText('本版更新内容')
    fireEvent.click(screen.getByRole('button', { name: /检查更新/ }))

    await waitFor(() => expect(screen.getByText('当前已是最新版本')).toBeInTheDocument())
  })
})
