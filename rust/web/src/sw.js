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
const CACHE = 'ip-console-v2';

const SHELL = [
  '/',
  '/assets/app.js',
  '/assets/client.js',
  '/assets/dom.js',
  '/assets/markdown.js',
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

/** @typedef {ExtendableEvent & { notification: Notification }} NotificationEvent */
/** @typedef {ExtendableEvent & { data: { json(): any } | null }} PushEvent */

// 主机推来的通知（backlog F-10）：页面关着也能收到。内容已在主机上按这台设备的设置筛过、脱过敏，
// 这里照着弹。同一个标签的通知会被新的一条替换，“已在别处处理”就是这样把旧的盖掉的。
sw.addEventListener('push', (/** @type {PushEvent} */ event) => {
  /** @type {{ title?: string, body?: string, tag?: string, thread_id?: string, silent?: boolean }} */
  let data = {};
  try {
    data = event.data?.json() ?? {};
  } catch {
    /* 不是 JSON 也照样弹一条，浏览器要求每次推送都显示通知 */
  }
  event.waitUntil(
    sw.registration.showNotification(data.title ?? 'intelligent-position', {
      body: data.body ?? '',
      tag: data.tag,
      icon: '/assets/icon-192.png',
      data: { thread_id: data.thread_id ?? null },
      silent: !!data.silent,
    }),
  );
});

// 点了控制台弹的通知（backlog F-9）：有开着的控制台就切过去并告诉它打开哪个对话，没有就开一个新窗口。
sw.addEventListener('notificationclick', (/** @type {NotificationEvent} */ event) => {
  event.notification.close();
  const threadId = event.notification.data?.thread_id ?? null;
  event.waitUntil(
    sw.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((/** @type {any[]} */ list) => {
      const open = list[0];
      if (open) {
        open.postMessage({ type: 'open-alert', thread_id: threadId });
        return open.focus();
      }
      return sw.clients.openWindow(threadId ? `/#/chat/${encodeURIComponent(threadId)}` : '/#/inbox');
    }),
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
