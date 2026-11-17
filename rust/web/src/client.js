// @ts-check
// WebSocket 客户端：请求/回复按 id 配对，事件广播给订阅者，断线后自动重连。
// 重连时带上次收到的事件序号，服务端补发断开期间的事件（docs/ip/m2-spec.md D-29）。

/** @typedef {import('./types/ServerFrame').ServerFrame} ServerFrame */
/** @typedef {import('./types/Request').Request} Request */
/** @typedef {import('./types/RuntimeEvent').RuntimeEvent} RuntimeEvent */
/** @typedef {import('./types/Alert').Alert} Alert */
/** @typedef {'connecting' | 'open' | 'closed'} Status */

/** 这份控制台代码说的 `/ws` 协议版本，和服务端 `PROTOCOL_VERSION` 对应。 */
export const PROTOCOL = 1;

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
    /** @type {Set<(a: Alert) => void>} */
    this.alertListeners = new Set();
    /** @type {Set<(tag: string) => void>} */
    this.dismissListeners = new Set();
    this.retryMs = 500;
    /** @type {Status} */
    this.status = 'connecting';
    /** 服务端本次启动的标识；连上过才有。 @type {string | null} */
    this.epoch = null;
    /** 收到的最后一个事件序号。 */
    this.seq = 0;
    /** 这次连上时服务端是否补发了断开期间的事件。为 false 时要重新拉列表。 */
    this.resumed = false;
    /** 服务端说的协议版本；和 PROTOCOL 不同时页面该刷新了。 @type {number | null} */
    this.serverProtocol = null;
  }

  connect() {
    this.setStatus('connecting');
    let url = this.url;
    if (this.epoch !== null) {
      url += `${url.includes('?') ? '&' : '?'}since=${this.seq}&epoch=${encodeURIComponent(this.epoch)}`;
    }
    const ws = new WebSocket(url);
    this.ws = ws;
    // 收到服务端的 hello 才算连上。
    ws.onopen = () => {
      this.retryMs = 500;
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

  /** 服务端的协议版本和这份代码不同（比如服务升级了，页面还是旧的）。 */
  get outdated() {
    return this.serverProtocol !== null && this.serverProtocol !== PROTOCOL;
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
      case 'hello':
        this.epoch = frame.epoch;
        this.seq = frame.seq;
        this.resumed = frame.resumed;
        this.serverProtocol = frame.protocol;
        this.setStatus('open');
        return;
      case 'event':
        this.seq = frame.seq;
        for (const fn of this.eventListeners) fn(frame.event);
        return;
      case 'lagged':
        for (const fn of this.lagListeners) fn();
        return;
      case 'alert':
        for (const fn of this.alertListeners) fn(frame.alert);
        return;
      case 'dismiss':
        for (const fn of this.dismissListeners) fn(frame.tag);
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

  /** 服务端按这台设备的通知设置筛过的通知（docs/ip/m2-spec.md D-32）。 @param {(a: Alert) => void} fn */
  onAlert(fn) {
    this.alertListeners.add(fn);
  }

  /** 撤掉某个标签的通知。 @param {(tag: string) => void} fn */
  onDismiss(fn) {
    this.dismissListeners.add(fn);
  }
}
