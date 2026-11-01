// Enough of a browser to run app.js and the scripts it needs in node: elements that keep
// what is written into them and the handlers put on them, timers run by hand, the app's
// stream from fakestream.js and a fake app behind fetch that records every request. No
// layout, no CSS, no real events; screens.js and the screens are not loaded.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { fakeNet } = require('./fakestream.js');

const WEB = path.join(__dirname, '..', '..', 'web');

function makeEl(tag, id) {
  const handlers = {};
  const el = {
    tagName: String(tag || 'div').toUpperCase(), id: id || '', dataset: {}, style: {}, children: [], options: [],
    innerHTML: '', textContent: '', value: '', hidden: false, disabled: false, checked: false, title: '',
    max: 0, selectedIndex: 0, handlers, attrs: {},
    classList: {
      set: new Set(),
      add(c) { this.set.add(c); }, remove(c) { this.set.delete(c); }, contains(c) { return this.set.has(c); },
      toggle(c, on) { if (on === undefined ? !this.set.has(c) : on) this.set.add(c); else this.set.delete(c); },
    },
    addEventListener(type, fn) { (handlers[type] = handlers[type] || []).push(fn); },
    setAttribute(k, v) { this.attrs[k] = String(v); }, getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; },
    appendChild(c) { this.children.push(c); return c; }, append(...c) { this.children.push(...c); }, prepend(c) { this.children.unshift(c); },
    add(o) { this.options.push(o); }, contains() { return false; }, closest() { return null; },
    focus() {}, blur() {}, click() { if (this.onclick) this.onclick(); },
    querySelector(sel) { this.q = this.q || {}; return (this.q[sel] = this.q[sel] || makeEl('div')); },
    querySelectorAll() { return []; },
    get clientWidth() { return 600; }, get clientHeight() { return 0; },
  };
  return el;
}

// o: {search, model, replays: {id: state}, answer(url, init)} -> the page and its handles
function dashboard(o) {
  o = Object.assign({ search: '', model: {} }, o);
  const net = fakeNet();
  const els = {};
  const $ = (id) => (els[id] = els[id] || makeEl('div', id));
  for (const id of ['rp-speed']) $(id).value = '60';
  for (const id of ['cl-units', 'cl-rx', 'cl-hidden']) $(id).checked = true;
  const calls = [];
  const downloads = [];
  const intervals = [];
  const stored = {};
  const model = Object.assign({ lostAfterMs: 600000, staleIntervals: 3, staleMinMs: 5000, lowBatteryV: 3.5, weakRssiDbm: -115, accelSatMs2: 75, dedupeWindowMs: 1500 }, o.model);
  const json = (j, status) => ({ ok: (status || 200) < 400, status: status || 200, json: async () => j, text: async () => JSON.stringify(j), blob: async () => ({ size: 1 }) });
  const fetch = async (url, init) => {
    const method = (init && init.method) || 'GET';
    calls.push({ url: String(url), method, body: init ? init.body : undefined });
    const own = o.answer && o.answer(String(url), init);
    if (own) return own;
    if (url === '/api/model-settings') {
      if (method === 'POST') Object.assign(model, JSON.parse(init.body));
      return json(model);
    }
    if (url === '/api/ports') return json({ ports: [], boards: [], log: '/data/descent-x.log' });
    if (String(url).startsWith('/api/replay/open')) return json({ id: 'r1' });
    if (url === '/api/replay/r1') return json(o.replay || { position: 10, lines: 10, packets: 3, from: 1790000000000, playing: false });
    if (url === '/api/replay/r1/csv') return { ok: true, status: 200, text: async () => 'time_ms,csid\n1,9\n2,9\n' };
    return json({ ok: true });
  };
  const document = {
    getElementById: $,
    querySelector: () => makeEl('div'),
    querySelectorAll: () => [],
    createElement: (tag) => makeEl(tag),
    addEventListener() {},
    activeElement: null,
    documentElement: { dataset: {} },
    body: { dataset: {} },
  };
  class FakeBlob { constructor(parts, opt) { this.parts = parts; this.type = opt && opt.type; } }
  // Every tone the page starts: [frequency Hz, wave, length s, how the level moves: 'eased' or 'hard']
  const sounds = [];
  class FakeAudio {
    constructor() { this.currentTime = 0; this.state = 'running'; this.destination = {}; }
    createGain() {
      const g = { ramps: 0, connect: () => g, gain: { setValueAtTime() {}, exponentialRampToValueAtTime() { g.ramps++; } } };
      this.lastGain = g;
      return g;
    }
    createOscillator() {
      const ctx = this;
      const o = { hz: 0, at: 0, type: 'sine', connect: () => o, frequency: { value: 0 } };
      o.frequency.setValueAtTime = (hz, at) => { o.hz = hz; o.at = at; };
      o.frequency.exponentialRampToValueAtTime = () => { throw new Error('a tone does not slide'); };
      o.start = (at) => { o.at = at; };
      o.stop = (at) => { sounds.push([o.hz, o.type, Math.round((at - o.at) * 100) / 100, ctx.lastGain && ctx.lastGain.ramps ? 'eased' : 'hard']); };
      return o;
    }
  }
  const sandbox = {
    console, document, fetch, WebSocket: net.WebSocket, Blob: FakeBlob, URLSearchParams, Intl, Math, JSON, Date, Number,
    location: { protocol: 'http:', host: '127.0.0.1:8791', search: o.search },
    localStorage: { getItem: (k) => (k in stored ? stored[k] : null), setItem: (k, v) => { stored[k] = v; } },
    URL: { createObjectURL: (b) => { downloads.push(b); return 'blob:' + downloads.length; }, revokeObjectURL() {} },
    setInterval: (fn, ms) => { intervals.push({ fn, ms }); return intervals.length; },
    clearInterval() {},
    setTimeout: (fn, ms) => (ms >= 500 ? net.setTimeout(fn, ms) : setTimeout(fn, 0)),
    clearTimeout() {},
    requestAnimationFrame: (fn) => setTimeout(fn, 0),
    performance: { now: () => Date.now() },
    ResizeObserver: class { observe() {} },
    getComputedStyle: () => ({ getPropertyValue: () => '#000000', overflowY: 'visible', gridTemplateColumns: '1fr', rowGap: '0' }),
    matchMedia: () => ({ matches: false }),
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    addEventListener() {},
    uPlot: class { constructor(opts, data) { this.data = data; this.width = 600; this.height = 190; this.cursor = { idx: null }; } setData(d) { this.data = d; } setSize() {} },
    AudioContext: FakeAudio,
  };
  sandbox.window = sandbox;
  sandbox.self = sandbox;
  vm.createContext(sandbox);
  const html = fs.readFileSync(path.join(WEB, 'index.html'), 'utf8');
  const scripts = [...html.matchAll(/<script src="js\/([^"]+)"/g)].map((m) => m[1]);
  // the dashboard's own chain, up to app.js; the screens after it need the shell
  for (const f of scripts.slice(0, scripts.indexOf('app.js') + 1)) {
    vm.runInContext(fs.readFileSync(path.join(WEB, 'js', f), 'utf8'), sandbox, { filename: f });
  }
  const settle = () => new Promise((r) => setTimeout(r, 5));
  return {
    $, els, calls, net, downloads, stored, model, settle, sandbox, scripts, sounds, app: sandbox.DGApp,
    posts: () => calls.filter((c) => c.method === 'POST'),
    // every timer of that period, once
    async every(ms) { for (const t of intervals.filter((x) => x.ms === ms)) await t.fn(); await settle(); },
    async fire(id, type, target) {
      const el = $(id);
      // a target with its own tagName is another element, like an input inside a form
      const t = target && target.tagName ? target : Object.assign(el, target || {});
      for (const fn of el.handlers[type] || []) await fn({ target: t, preventDefault() {} });
      await settle();
    },
  };
}

module.exports = { dashboard, makeEl };
