// A page that is not there, for running a screen's own script in node. It fakes the
// elements a screen paints into and listens on, the shell it registers with, and the app
// behind /api. What a screen paints is kept as text, so a test can say what is in it.
// It cannot tell how anything looks: no layout, no CSS, no focus, no real clicks.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const WEB = path.join(__dirname, '..', '..', 'web', 'js');
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

// hub is the fake app. o.screen is the screen that is up, and can be changed by a test.
function page(scripts, hub, o) {
  const els = {};
  const handlers = {};
  const timers = [];
  const parts = {};
  const heard = [];
  const toasts = [];
  const el = (id) => (els[id] = els[id] || {
    id, html: '', dataset: {}, style: {},
    addEventListener(type, fn) { (handlers[id + ':' + type] = handlers[id + ':' + type] || []).push(fn); },
  });
  const jobs = { pull: null, chip: null, tbeam: null, heard };
  const sandbox = {
    console,
    setInterval: (fn, ms) => { fn.ms = ms; timers.push(fn); return timers.length; },
    clearInterval() {},
    localStorage: { getItem: () => null, setItem() {} },
    DG: {},
  };
  sandbox.window = sandbox;
  sandbox.self = sandbox;
  sandbox.DGApp = {
    hub, live: { receivers: new Map() }, taps: [],
    ui: { $: el, esc, toast: (t) => toasts.push(t), logUrl: (n) => n, flatName: (n) => n, age: () => '', sfSelect: () => '' },
    shell: {
      paint: (e, html) => { e.html = html; },
      mark: (cls, word) => '<span class="mark ' + cls + '">' + esc(word) + '</span>',
      steps: () => '', track: () => '', nameWidth: () => '', jobs,
      onJobs: (fn) => heard.push(fn), pollJobs: async () => {}, renderStrip() {},
      register: (name, part) => { parts[name] = part; },
      current: () => o.screen, seen() {}, show() {},
    },
  };
  vm.createContext(sandbox);
  for (const f of scripts) vm.runInContext(fs.readFileSync(path.join(WEB, f), 'utf8'), sandbox, { filename: f });

  // Something a handler reads its attributes off, standing in for the element pressed.
  const target = (attrs, more) => Object.assign(
    { getAttribute: (n) => (n in attrs ? attrs[n] : null), closest: () => null, blur() {}, tagName: 'BUTTON' }, more || {});
  const settle = () => new Promise((r) => setTimeout(r, 5));
  return {
    parts, jobs, toasts, settle, target, taps: sandbox.DGApp.taps, shell: sandbox.DGApp.shell,
    html: (id) => el(id).html,
    // What a person reads: tags out, entities back, runs of space as one
    text: (id) => el(id).html.replace(/<[^>]*>/g, ' ')
      .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&amp;/g, '&')
      .replace(/\s+/g, ' ').trim(),
    // ev: what else the event carries, like the key of a keydown
    async fire(id, type, attrs, more, ev) {
      for (const fn of handlers[id + ':' + type] || []) await fn(Object.assign({ target: target(attrs || {}, more), preventDefault() {} }, ev));
      await settle();
    },
    // The screen's timer, run by hand as many times as asked.
    async ticks(n) {
      for (let i = 0; i < n; i++) { for (const fn of timers.slice(-1)) await fn(); await settle(); }
    },
    // Every timer set to this many ms, when a page has more than one.
    async every(ms, n) {
      for (let i = 0; i < (n || 1); i++) { for (const fn of timers.filter((f) => f.ms === ms)) await fn(); await settle(); }
    },
    async jobEnded(kind) { for (const fn of heard) fn([kind]); await settle(); },
  };
}

module.exports = { page };
