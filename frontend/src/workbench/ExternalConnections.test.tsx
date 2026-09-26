import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { LangProvider } from '../locales'
import { ExternalConnections } from './ExternalConnections'
const clients = vi.hoisted(() => vi.fn())
vi.mock('./api', () => ({ clients }))
const copy = vi.fn().mockResolvedValue(undefined)
const local = {
  status: 'listening',
  available: true,
  executable: 'E:\\应用 Space\\nuphus-workbench-mcp.exe',
  config: {
    mcpServers: {
      'nuphus-workbench': { command: 'E:\\应用 Space\\nuphus-workbench-mcp.exe', args: ['serve'] },
    },
  },
}
beforeEach(() => {
  clients.mockReset()
  copy.mockClear()
  localStorage.setItem('nuphus_language', 'zh')
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: copy } })
})
it('copies an executable configuration without ports and keeps HTTP failure independent', async () => {
  render(
    <LangProvider>
      <ExternalConnections endpoint={{ status: 'failed', message: 'Port occupied', local }} />
    </LangProvider>,
  )
  expect(screen.getByText('本地服务已就绪 · 全部权限')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '复制 MCP 配置' }))
  await waitFor(() => expect(copy).toHaveBeenCalledWith(JSON.stringify(local.config, null, 2)))
  expect(
    screen.getByText('高级接入：HTTP / Streamable HTTP').closest('details'),
  ).not.toHaveAttribute('open')
})
it('tests the installed MCP chain and displays actionable failures', async () => {
  clients.mockResolvedValue({ ok: false, message: 'protocol_mismatch: 请更新工作台' })
  render(
    <LangProvider>
      <ExternalConnections endpoint={{ status: 'listening', local }} />
    </LangProvider>,
  )
  fireEvent.click(screen.getByRole('button', { name: '测试连接' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('protocol_mismatch')
  expect(clients).toHaveBeenCalledWith('check')
})
it('does not offer usable setup when the executable is missing', () => {
  localStorage.setItem('nuphus_language', 'en')
  render(
    <LangProvider>
      <ExternalConnections
        endpoint={{ status: 'listening', local: { ...local, available: false } }}
      />
    </LangProvider>,
  )
  expect(screen.getByRole('button', { name: 'Test connection' })).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Copy MCP configuration' })).toBeDisabled()
  expect(screen.getByText(/MCP executable is missing/)).toBeInTheDocument()
})
