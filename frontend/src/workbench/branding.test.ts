import { expect, it } from 'vitest'
import { brandSplash } from './branding'
import splash from '../../public/splash.html?raw'
import logo from '../../public/lingque.svg?raw'

it('uses the chosen flow mark without changing splash controls or scripts', () => {
  const branded = brandSplash(splash)
  expect(branded).toContain('src="./lingque.svg"')
  expect(branded).toContain('<title>灵雀 Lingque</title>')
  for (const control of ['id="skipWrap"', 'id="barFill"', 'splash.js?v=']) {
    expect(branded).toContain(control)
  }
  expect(branded).not.toContain('class="nv-eye l"')
  expect(logo).toContain('灵雀 · 逐流')
  expect(logo).toContain('fill="#152C4B"')
})

it('reports incompatible upstream markup instead of silently shipping the wrong brand', () => {
  expect(() => brandSplash('<html>changed</html>')).toThrow('upstream splash logo changed')
})
