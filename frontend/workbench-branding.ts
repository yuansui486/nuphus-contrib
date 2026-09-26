import { readFile, writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import type { Plugin } from 'vite'
import { brandSplash } from './src/workbench/branding'

// Transform the upstream splash at build/dev time; retain its download/progress logic.
export function lingqueBranding(): Plugin {
  let root = ''
  let output = ''
  return {
    name: 'lingque-branding',
    configResolved(config) {
      root = config.root
      output = resolve(root, config.build.outDir)
    },
    transformIndexHtml(html, context) {
      if (!context.filename.endsWith('index.html')) return html
      return html
        .replace(/<title>[^<]*<\/title>/, '<title>灵雀 Lingque</title>')
        .replace(
          'type="image/x-icon" href="/nuphus.ico"',
          'type="image/svg+xml" href="./lingque.svg"',
        )
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url?.split('?')[0] !== '/splash.html') return next()
        void readFile(resolve(root, 'public/splash.html'), 'utf8')
          .then(html => {
            res.setHeader('Content-Type', 'text/html; charset=utf-8')
            res.setHeader('Cache-Control', 'no-store')
            res.end(brandSplash(html))
          })
          .catch(next)
      })
    },
    async writeBundle() {
      const path = resolve(output, 'splash.html')
      await writeFile(path, brandSplash(await readFile(path, 'utf8')))
    },
  }
}
