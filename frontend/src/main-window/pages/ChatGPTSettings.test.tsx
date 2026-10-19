import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatGPTSettings } from './ChatGPTSettings'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  switchModel: vi.fn(),
  openExternal: vi.fn(),
  listModels: vi.fn(),
  setModelSupportsVision: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('../lib/api', () => ({
  switchModel: mocks.switchModel,
  openExternal: mocks.openExternal,
  listModels: mocks.listModels,
  setModelSupportsVision: mocks.setModelSupportsVision,
  getCurrentConfig: vi.fn(async () => null),
}))
vi.mock('../../locales', () => ({ useLanguage: () => ({ lang: 'en' }) }))

const account = {
  account_id: 'oaiapp_one',
  provider: 'custom-chatgpt-one',
  label: 'one@example.invalid · oaiapp_one',
  logged_in: true,
}

beforeEach(() => {
  vi.clearAllMocks()
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === 'chatgpt_accounts') return [account]
    if (command === 'chatgpt_models')
      return [{ slug: 'account-model', display_name: 'Account model' }]
    if (command === 'chatgpt_logout') return true
    return undefined
  })
  mocks.switchModel.mockResolvedValue('ok')
  mocks.listModels.mockResolvedValue([
    { id: 'account-model', provider: account.provider, supports_vision: false },
    { id: 'account-model', provider: 'custom-chatgpt-other', supports_vision: true },
  ])
  mocks.setModelSupportsVision.mockResolvedValue('ok')
})

describe('ChatGPT settings', () => {
  it('allows vision input to be enabled for the exact account and model', async () => {
    const modelsChanged = vi.fn()
    render(<ChatGPTSettings onModelsChanged={modelsChanged} />)
    const toggle = await screen.findByRole('switch', { name: 'Account model vision input' })
    expect(toggle).not.toBeChecked()
    fireEvent.click(toggle)
    await waitFor(() => expect(toggle).toBeChecked())
    expect(mocks.setModelSupportsVision).toHaveBeenCalledWith(
      'custom-chatgpt-one',
      'account-model',
      true,
    )
    expect(modelsChanged).toHaveBeenLastCalledWith(
      expect.arrayContaining([
        expect.objectContaining({ provider: account.provider, supports_vision: true }),
        expect.objectContaining({ provider: 'custom-chatgpt-other', supports_vision: true }),
      ]),
    )
  })

  it('loads saved vision settings again after refreshing the account catalog', async () => {
    mocks.listModels.mockResolvedValue([
      { id: 'account-model', provider: account.provider, supports_vision: true },
    ])
    render(<ChatGPTSettings />)
    expect(await screen.findByRole('switch', { name: 'Account model vision input' })).toBeChecked()
    fireEvent.click(screen.getByRole('button', { name: 'Refresh models' }))
    await waitFor(() => expect(mocks.listModels).toHaveBeenCalledTimes(2))
    expect(await screen.findByRole('switch', { name: 'Account model vision input' })).toBeChecked()
  })

  it('preserves the saved vision setting when persistence fails', async () => {
    mocks.setModelSupportsVision.mockRejectedValueOnce(new Error('Could not save model settings'))
    render(<ChatGPTSettings />)
    const toggle = await screen.findByRole('switch', { name: 'Account model vision input' })
    fireEvent.click(toggle)
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not save model settings')
    expect(toggle).not.toBeChecked()
    expect(toggle).toBeEnabled()
  })

  it('uses the account catalog and switches the exact provider/model pair', async () => {
    const changed = vi.fn()
    render(<ChatGPTSettings onModelChanged={changed} />)
    const model = await screen.findByRole('button', { name: 'Account model' })
    expect(mocks.invoke).toHaveBeenCalledWith('chatgpt_models', { accountId: 'oaiapp_one' })
    fireEvent.click(model)
    await waitFor(() => expect(changed).toHaveBeenCalledOnce())
    expect(mocks.switchModel).toHaveBeenCalledWith(
      'account-model',
      'custom-chatgpt-one',
      undefined,
      undefined,
      'default',
    )
    expect(screen.getByRole('button', { name: 'Account model · Active' })).toBeInTheDocument()
  })

  it('shows denied authorization without losing the saved account', async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'chatgpt_accounts') return [{ ...account, logged_in: false }]
      if (command === 'chatgpt_login') throw new Error('ChatGPT authorization was declined')
      return undefined
    })
    render(<ChatGPTSettings />)
    await screen.findByRole('combobox')
    fireEvent.click(screen.getByRole('button', { name: 'Continue with ChatGPT' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('authorization was declined')
    expect(screen.getByRole('combobox')).toHaveValue('oaiapp_one')
    expect(mocks.invoke).toHaveBeenCalledWith('chatgpt_login', { accountId: 'oaiapp_one' })
    expect(mocks.invoke).not.toHaveBeenCalledWith('chatgpt_models', expect.anything())
  })

  it('distinguishes local sign-out from confirmed remote revocation', async () => {
    let signedOut = false
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === 'chatgpt_accounts') return [{ ...account, logged_in: !signedOut }]
      if (command === 'chatgpt_models')
        return [{ slug: 'account-model', display_name: 'Account model' }]
      if (command === 'chatgpt_logout') {
        signedOut = true
        return false
      }
      return undefined
    })
    render(<ChatGPTSettings />)
    await screen.findByRole('button', { name: 'Account model' })
    fireEvent.click(screen.getByRole('button', { name: 'Sign out' }))
    expect(await screen.findByText(/remote revocation was not confirmed/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Account model' })).not.toBeInTheDocument()
    expect(screen.getByRole('combobox')).toHaveValue('oaiapp_one')
  })
})
