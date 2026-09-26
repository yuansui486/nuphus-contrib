// Regenerate only desktop assets from the chosen, unchanged B / 逐流 SVG.
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
const root = fileURLToPath(new URL('..', import.meta.url));
const generated = join(root, 'target/lingque-icons');
const destination = join(root, 'src-tauri/icons-lingque');
const result = spawnSync(process.execPath, [
  join(root, 'frontend/node_modules/@tauri-apps/cli/tauri.js'), 'icon',
  join(root, 'frontend/public/lingque.svg'), '--output', generated,
], { cwd: root, stdio: 'inherit' });
if (result.status !== 0) process.exit(result.status || 1);
mkdirSync(destination, { recursive: true });
for (const name of ['32x32.png', '128x128.png', '128x128@2x.png', 'icon.png', 'icon.ico', 'icon.icns']) {
  copyFileSync(join(generated, name), join(destination, name));
}
