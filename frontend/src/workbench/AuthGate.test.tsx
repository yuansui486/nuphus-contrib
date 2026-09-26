import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import AuthGate, { type AuthStatus } from './AuthGate'
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  handler: null as null | ((event: { payload: AuthStatus }) => void),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((_name, handler) => {
    mocks.handler = handler
    return Promise.resolve(() => {})
  }),
}))
vi.mock('../main-window/layout/TitleBar', () => ({ TitleBar: () => <div>灵雀标题栏</div> }))
vi.mock('./WorkbenchApp', () => ({
  default: ({ storageScope }: { storageScope: string }) => <div>业务工作台 {storageScope}</div>,
}))
const signedOut: AuthStatus = {
  state: 'signed_out',
  authorized: false,
  message: '请登录灵雀',
  subject: null,
  policy: null,
  offline_until: null,
  epoch: null,
}
const online: AuthStatus = {
  state: 'online',
  authorized: true,
  message: '已授权',
  subject: { id: 'u1', tenant_id: 't1', tenant_name: '测试企业', username: 'test' },
  policy: { concurrent_device_limit: 2, active_session_count: 1 },
  offline_until: '2030-01-01T00:00:00Z',
  epoch: 'e1',
}
beforeEach(() => {
  mocks.invoke.mockReset()
  mocks.handler = null
  mocks.invoke.mockImplementation((_command, args) =>
    Promise.resolve(args?.action === 'status' ? signedOut : []),
  )
})
async function fill() {
  await screen.findByRole('button', { name: '登录灵雀' })
  await waitFor(() => expect(screen.getByRole('button', { name: '登录灵雀' })).toBeEnabled())
  fireEvent.change(screen.getByLabelText('租户编码'), { target: { value: 'tenant' } })
  fireEvent.change(screen.getByLabelText('用户名'), { target: { value: 'test' } })
  fireEvent.change(screen.getByLabelText('密码'), { target: { value: ' test password ' } })
}
it('does not mount business before product authorization', async () => {
  render(<AuthGate />)
  await fill()
  expect(screen.queryByText(/业务工作台/)).not.toBeInTheDocument()
  expect(mocks.invoke).not.toHaveBeenCalledWith('workbench_call', expect.anything())
})
it('submits once without changing password and clears it after failure', async () => {
  let rejectLogin: (error: unknown) => void = () => {}
  mocks.invoke.mockImplementation((_command, args) =>
    args?.action === 'login'
      ? new Promise((_resolve, reject) => {
          rejectLogin = reject
        })
      : Promise.resolve(signedOut),
  )
  render(<AuthGate />)
  await fill()
  const form = screen.getByLabelText('密码').closest('form')!
  fireEvent.submit(form)
  fireEvent.submit(form)
  expect(mocks.invoke.mock.calls.filter(([, args]) => args?.action === 'login')).toHaveLength(1)
  expect(mocks.invoke).toHaveBeenCalledWith('lingque_auth', {
    action: 'login',
    tenantCode: 'tenant',
    username: 'test',
    password: ' test password ',
  })
  await act(async () => rejectLogin({ message: '该租户尚未开通灵雀' }))
  expect(await screen.findByText('该租户尚未开通灵雀')).toBeVisible()
  expect(screen.getByLabelText('密码')).toHaveValue('')
})
it('requires explicit legacy project ownership confirmation', async () => {
  let claimed = false
  mocks.invoke.mockImplementation((_command, args) => {
    if (args?.action === 'status') return Promise.resolve(online)
    if (args?.action === 'claim') claimed = true
    if (args?.action === 'unclaimed')
      return Promise.resolve(
        claimed ? [] : [{ project_id: 'p1', name: '原工作流', directory: 'E:/test' }],
      )
    return Promise.resolve({})
  })
  render(<AuthGate />)
  expect(await screen.findByText('原工作流')).toBeVisible()
  expect(screen.queryByText(/业务工作台/)).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '确认归属并进入' }))
  expect(await screen.findByText('业务工作台 t1')).toBeVisible()
  expect(mocks.invoke).toHaveBeenCalledWith('lingque_auth', {
    action: 'claim',
    expectedEpoch: 'e1',
    projectIds: ['p1'],
  })
})
it('keeps offline business available but unmounts immediately on revoked authorization', async () => {
  mocks.invoke.mockImplementation((_command, args) =>
    Promise.resolve(args?.action === 'status' ? { ...online, state: 'offline' } : []),
  )
  render(<AuthGate />)
  expect(await screen.findByText('业务工作台 t1')).toBeVisible()
  expect(screen.getByText(/离线授权 · 有效至/)).toBeVisible()
  act(() => mocks.handler?.({ payload: { ...signedOut, message: '设备登录已失效' } }))
  expect(screen.queryByText(/业务工作台/)).not.toBeInTheDocument()
  expect(screen.getByText('设备登录已失效')).toBeVisible()
})
it('requires validation rather than showing a password form for an expired cached session', async () => {
  mocks.invoke.mockImplementation(() =>
    Promise.resolve({ ...online, authorized: false, state: 'expired', message: '离线授权已到期' }),
  )
  render(<AuthGate />)
  expect(await screen.findByRole('button', { name: '重新联网验证' })).toBeVisible()
  expect(screen.queryByLabelText('密码')).not.toBeInTheDocument()
})
