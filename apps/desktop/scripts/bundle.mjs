// 把网页控制台（rust/web）拷成桌面 App 自带的一份（backlog F-11）。
// 目录结构和主机上一样：页面在根上，脚本、样式、图标在 /assets/ 下。
// Service Worker 和 PWA 清单在桌面 App 里用不上，不拷。
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const web = join(here, '..', '..', '..', 'rust', 'web');
const dist = join(here, '..', 'dist');

rmSync(dist, { recursive: true, force: true });
mkdirSync(join(dist, 'assets'), { recursive: true });
for (const name of ['app.js', 'client.js', 'dom.js', 'markdown.js', 'style.css']) {
  cpSync(join(web, 'src', name), join(dist, 'assets', name));
}
for (const name of ['icon.svg', 'icon-192.png']) {
  cpSync(join(web, 'icons', name), join(dist, 'assets', name));
}
const index = readFileSync(join(web, 'index.html'), 'utf8')
  .split('\n')
  .filter((line) => !/rel="(manifest|apple-touch-icon)"|apple-mobile-web-app/.test(line))
  .join('\n');
writeFileSync(join(dist, 'index.html'), index);
console.log(`控制台已拷到 ${dist}`);
