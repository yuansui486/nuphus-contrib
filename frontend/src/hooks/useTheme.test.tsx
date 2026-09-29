import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { readBaseTheme, skinRestoreSource, ThemeProvider, useTheme, type ThemeId } from './useTheme'

vi.mock('../main-window/lib/plugin-apps', () => ({ themeSnapshotSave: vi.fn(async () => {}) }))

beforeEach(() => localStorage.clear())
afterEach(() => {
  cleanup()
  localStorage.clear()
  vi.restoreAllMocks()
})

function CurrentTheme() {
  return <span>{useTheme().theme}</span>
}

it('defaults Lingque to light both before paint and after mounting', () => {
  expect(readBaseTheme('light')).toBe('light')
  render(
    <ThemeProvider defaultTheme="light">
      <CurrentTheme />
    </ThemeProvider>,
  )
  expect(screen.getByText('light')).toBeInTheDocument()
  expect(document.documentElement).toHaveAttribute('data-theme', 'light')
  expect(localStorage.getItem('nuphus_theme')).toBeNull()
})

it.each<ThemeId>(['dark', 'light', 'tech'])('preserves the saved %s preference', theme => {
  localStorage.setItem('nuphus_theme', theme)
  expect(readBaseTheme('light')).toBe(theme)
  render(
    <ThemeProvider defaultTheme="light">
      <CurrentTheme />
    </ThemeProvider>,
  )
  expect(document.documentElement).toHaveAttribute('data-theme', theme)
})

it('keeps the original edition default unchanged', () => {
  expect(readBaseTheme()).toBe('dark')
  render(
    <ThemeProvider>
      <CurrentTheme />
    </ThemeProvider>,
  )
  expect(screen.getByText('dark')).toBeInTheDocument()
})

it.each([undefined, 'C:/theme/skin.png'])(
  'preserves an active custom theme and its skin with the light edition default (%s)',
  skin => {
    localStorage.setItem(
      'nuphus_custom_themes',
      JSON.stringify([{ id: 'saved', name: 'Saved', base: 'tech', overrides: {}, skin }]),
    )
    localStorage.setItem('nuphus_custom_active', 'saved')
    function RestoredTheme() {
      const { theme, customTheme } = useTheme()
      return (
        <output data-testid="restored" data-skin={skinRestoreSource(customTheme, 'preset.png')}>
          {theme}
        </output>
      )
    }
    render(
      <ThemeProvider defaultTheme="light">
        <RestoredTheme />
      </ThemeProvider>,
    )
    expect(screen.getByTestId('restored')).toHaveTextContent('tech')
    expect(screen.getByTestId('restored')).toHaveAttribute('data-skin', skin ?? '')
  },
)

it('uses the edition fallback for invalid or unavailable storage', () => {
  localStorage.setItem('nuphus_theme', 'invalid')
  expect(readBaseTheme('light')).toBe('light')
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('unavailable')
  })
  expect(readBaseTheme('light')).toBe('light')
})
