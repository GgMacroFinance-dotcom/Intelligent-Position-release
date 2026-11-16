// @ts-check
// WebSocket 客户端：请求/回复按 id 配对，事件广播给订阅者，断线后自动重连。

/** @typedef {import('./types/ServerFrame').ServerFrame} ServerFrame */
/** @typedef {import('./types/Request').Request} Request */
/** @typedef {import('./types/RuntimeEvent').RuntimeEvent} RuntimeEvent */
/** @typedef {'connecting' | 'open' | 'closed'} Status */

export class Client {
  /** @param {string} url */
  constructor(url) {
    this.url = url;
    /** @type {WebSocket | null} */
    this.ws = null;
    this.nextId = 1;
    /** @type {Map<number, {resolve: (v: any) => void, reject: (e: Error) => void}>} */
    this.pending = new Map();
    /** @type {Set<(e: RuntimeEvent) => void>} */
    this.eventListeners = new Set();
    /** @type {Set<(s: Status) => void>} */
    this.statusListeners = new Set();
    /** @type {Set<() => void>} */
    this.lagListeners = new Set();
    this.retryMs = 500;
    /** @type {Status} */
    this.status = 'connecting';
  }

  connect() {
    this.setStatus('connecting');
    const ws = new WebSocket(this.url);
    this.ws = ws;
    ws.onopen = () => {
      this.retryMs = 500;
      this.setStatus('open');
    };
    ws.onmessage = (msg) => this.onFrame(/** @type {ServerFrame} */ (JSON.parse(msg.data)));
    ws.onclose = () => {
      for (const p of this.pending.values()) p.reject(new Error('连接已断开'));
      this.pending.clear();
      this.setStatus('closed');
      setTimeout(() => this.connect(), this.retryMs);
      this.retryMs = Math.min(this.retryMs * 2, 8000);
    };
  }

  /** @param {Status} s */
  setStatus(s) {
    this.status = s;
    for (const fn of this.statusListeners) fn(s);
  }

  /** @param {ServerFrame} frame */
  onFrame(frame) {
    switch (frame.type) {
      case 'reply': {
        const p = typeof frame.id === 'number' ? this.pending.get(frame.id) : undefined;
        if (!p || typeof frame.id !== 'number') return;
        this.pending.delete(frame.id);
        if (frame.ok) p.resolve(frame.data);
        else p.reject(new Error(frame.error ?? '请求失败'));
        return;
      }
      case 'event':
        for (const fn of this.eventListeners) fn(frame.event);
        return;
      case 'lagged':
        for (const fn of this.lagListeners) fn();
        return;
    }
  }

  /**
   * 发请求，返回回复里的 data。
   * @template T
   * @param {Request} request
   * @returns {Promise<T>}
   */
  request(request) {
    const ws = this.ws;
    if (!ws || ws.readyState !== WebSocket.OPEN) return Promise.reject(new Error('尚未连接'));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ id, ...request }));
    });
  }

  /** @param {(e: RuntimeEvent) => void} fn */
  onEvent(fn) {
    this.eventListeners.add(fn);
  }

  /** @param {(s: Status) => void} fn */
  onStatus(fn) {
    this.statusListeners.add(fn);
  }

  /** @param {() => void} fn */
  onLagged(fn) {
    this.lagListeners.add(fn);
  }
}
