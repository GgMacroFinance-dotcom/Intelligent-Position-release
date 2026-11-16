// @ts-check
// Service Worker：让控制台能“安装”成桌面和手机上的应用（backlog F-2）。
//
// 只缓存控制台自己的静态文件（页面、脚本、样式、图标），不缓存任何数据：数据都走 /ws，
// Service Worker 管不到 WebSocket。取文件一律先走网络，拿到就更新缓存；断网时才用缓存，
// 所以主机升级后打开 App 就是新版本，不会卡在旧代码上。

// tsconfig 只带 dom 的类型，这里补上用到的几个 Service Worker 类型。
/** @typedef {Event & { waitUntil(p: Promise<unknown>): void }} ExtendableEvent */
/** @typedef {ExtendableEvent & { request: Request, respondWith(r: Promise<Response>): void }} FetchEvent */
/** @type {any} */
const sw = self;

/** 改了缓存的文件列表时把版本号加一，旧缓存会在 activate 时删掉。 */
const CACHE = 'ip-console-v1';

const SHELL = [
  '/',
  '/assets/app.js',
  '/assets/client.js',
  '/assets/dom.js',
  '/assets/style.css',
  '/assets/icon.svg',
  '/assets/icon-192.png',
  '/manifest.webmanifest',
];

sw.addEventListener('install', (/** @type {ExtendableEvent} */ event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((c) => c.addAll(SHELL))
      .then(() => sw.skipWaiting()),
  );
});

sw.addEventListener('activate', (/** @type {ExtendableEvent} */ event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => sw.clients.claim()),
  );
});

sw.addEventListener('fetch', (/** @type {FetchEvent} */ event) => {
  const req = event.request;
  const url = new URL(req.url);
  // 只管本站的 GET 静态文件；/ws、/mcp、/healthz 和别的站点都不碰。
  if (req.method !== 'GET' || url.origin !== sw.location.origin) return;
  const isShell = url.pathname === '/' || url.pathname.startsWith('/assets/') || url.pathname === '/manifest.webmanifest';
  if (!isShell) return;
  // 按路径缓存，不带查询串：地址带 ?token= 时令牌不进缓存的键。
  const key = url.pathname;
  event.respondWith(
    fetch(req)
      .then((res) => {
        if (res.ok) {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put(key, copy));
        }
        return res;
      })
      .catch(() => caches.match(key).then((hit) => hit ?? Response.error())),
  );
});
