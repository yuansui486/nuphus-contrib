import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { readBaseTheme, ThemeProvider, useTheme, type ThemeId } from './useTheme'

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

it('uses the edition fallback for invalid or unavailable storage', () => {
  localStorage.setItem('nuphus_theme', 'invalid')
  expect(readBaseTheme('light')).toBe('light')
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('unavailable')
  })
  expect(readBaseTheme('light')).toBe('light')
})
