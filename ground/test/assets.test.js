const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const WEB = path.join(__dirname, '..', 'web');
const html = fs.readFileSync(path.join(WEB, 'index.html'), 'utf8');
const sw = fs.readFileSync(path.join(WEB, 'sw.js'), 'utf8');

// The FILES array in sw.js, read as text: the worker itself only runs in a browser.
function offlineList() {
  const m = /const FILES = \[([\s\S]*?)\];/.exec(sw);
  assert.ok(m, 'sw.js still has a FILES list');
  return [...m[1].matchAll(/'([^']+)'/g)].map((x) => x[1]);
}

function pageLoads() {
  const out = [...html.matchAll(/<script\b[^>]*\bsrc="([^"]+)"/g)].map((m) => m[1]);
  for (const m of html.matchAll(/<link\b[^>]*>/g)) {
    const href = /\bhref="([^"]+)"/.exec(m[0]);
    if (/\brel="stylesheet"/.test(m[0]) && href) out.push(href[1]);
  }
  return out.filter((u) => !/^([a-z]+:|\/\/)/i.test(u));
}

test('every script and stylesheet the page loads is in the offline list', () => {
  const loads = pageLoads();
  // Guards the test itself: a pattern that stopped matching would pass with nothing checked.
  assert.ok(loads.includes('js/app.js') && loads.includes('css/style.css'), 'found the page\'s own files');
  const list = offlineList();
  assert.deepStrictEqual(loads.filter((u) => !list.includes(u)), []);
});

test('everything in the offline list is a file that exists', () => {
  // One missing file fails the whole cache.addAll, and then nothing is kept offline.
  const missing = offlineList().filter((u) => u !== './' && !fs.existsSync(path.join(WEB, u)));
  assert.deepStrictEqual(missing, []);
});

const FONTS = ['figtree-latin-400-normal.woff2', 'figtree-latin-600-normal.woff2',
  'figtree-latin-700-normal.woff2', 'caprasimo-latin-400-normal.woff2'];

test('the four vendored fonts are on disk and in the offline list', () => {
  const list = offlineList();
  for (const f of FONTS) {
    assert.ok(fs.existsSync(path.join(WEB, 'vendor', 'fonts', f)), f + ' on disk');
    assert.ok(list.includes('vendor/fonts/' + f), f + ' in sw.js');
  }
  assert.match(sw, /const CACHE = 'descent-ground-v15'/);
});

test('Plex Sans is gone, Plex Mono stays for hex', () => {
  const css = fs.readFileSync(path.join(WEB, 'css', 'style.css'), 'utf8');
  for (const [name, text] of [['index.html', html], ['style.css', css], ['sw.js', sw]]) {
    assert.doesNotMatch(text, /ibm-plex-sans|Plex Sans/, name);
  }
  assert.match(css, /--font-mono:\s*"Plex Mono"/);
});

// Every custom property the page uses is set on :root, so a token renamed in the rewrite
// cannot leave a rule or a chart reading an empty string.
test('every var() in the css and every --name the js reads is defined on :root', () => {
  const cssDir = path.join(WEB, 'css');
  const style = fs.readFileSync(path.join(cssDir, 'style.css'), 'utf8');
  const root = /(^|\n):root\s*\{([\s\S]*?)\n\}/.exec(style);
  assert.ok(root, 'style.css has a :root block');
  const defined = new Set([...root[2].matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
  const used = new Set();
  for (const f of fs.readdirSync(cssDir)) {
    for (const m of fs.readFileSync(path.join(cssDir, f), 'utf8').matchAll(/var\((--[\w-]+)/g)) used.add(m[1]);
  }
  const jsDir = path.join(WEB, 'js');
  for (const f of fs.readdirSync(jsDir)) {
    const js = fs.readFileSync(path.join(jsDir, f), 'utf8');
    for (const m of js.matchAll(/(?:['"]|var\()(--[\w-]+)/g)) used.add(m[1]);
  }
  assert.ok(used.has('--rule') && used.has('--sans') && used.size > 15, 'found the names in use');
  // charts.js builds '--series-' + n
  const missing = [...used].filter((n) => (n.endsWith('-')
    ? ![1, 2, 3, 4, 5, 6, 7, 8].every((i) => defined.has(n + i)) : !defined.has(n)));
  assert.deepStrictEqual(missing, []);
});

test('the names the js reads through getComputedStyle are plain values', () => {
  const style = fs.readFileSync(path.join(WEB, 'css', 'style.css'), 'utf8');
  const names = ['--rule', '--rule-strong', '--ink', '--ink-2', '--ink-3', '--sunk', '--panel', '--accent', '--critical', '--sans', '--mono']
    .concat([1, 2, 3, 4, 5, 6, 7, 8].map((i) => '--series-' + i));
  for (const n of names) {
    const all = [...style.matchAll(new RegExp('(?:^|[\\s;{])' + n + '\\s*:\\s*([^;]+);', 'g'))].map((m) => m[1].trim());
    assert.ok(all.length, n + ' is set');
    for (const v of all) assert.doesNotMatch(v, /color-mix|var\(/, n + ' = ' + v);
  }
});

// The ids of the 1.2.2 page, and which of them started hidden or disabled.
const IDS_122 = ('view-chipsats view-tbeams view-live view-data view-replay btn-connect receivers strip strip-rx ' +
  'strip-log strip-log-name strip-log-save strip-log-new strip-job strip-build strip-release strip-release-version ' +
  'strip-release-link file-open btn-settings btn-theme ' +
  'replay rp-play rp-speed rp-pos rp-time rp-save ' +
  'rp-csv rp-raw rp-exit rp-file dash sel-unit btn-pause latest clear-what cl-toggle btn-clear clear-opts ' +
  'cl-units cl-sel cl-rx cl-hidden cl-log summary sort btn-hide btn-restore fleet fleet-empty graph-title tabs window ' +
  'charts screen-chipsats cs-look cs-list cs-sheet screen-tbeams tb-ident tb-ident-stop tb-list tb-sheet screen-data ' +
  'data-where data-look data-table data-empty data-detail settings settings-close settings-form built settings-reset toast').split(' ');
const HIDDEN_122 = ['view-chipsats', 'view-tbeams', 'view-data', 'strip', 'strip-job', 'strip-build', 'strip-release',
  'file-open', 'replay', 'rp-raw', 'clear-opts', 'screen-chipsats', 'tb-ident-stop',
  'screen-tbeams', 'screen-data', 'data-empty', 'data-detail', 'settings', 'toast'];
const DISABLED_122 = ['view-replay'];
// 1.4: the browser's own recording, the crash autosave and Web Serial went with the page's
// decoder; the app records every line and Save a copy in the strip downloads it.
const GONE_14 = ('mode recpanel rec-state rec-start rec-pause rec-stop rec-info btn-save ' +
  'recover recover-text recover-open recover-download recover-discard unsupported').split(' ');

const tagOf = (id) => {
  const m = new RegExp('<[a-z][a-z0-9]*\\b[^>]*\\bid="' + id + '"[^>]*>').exec(html);
  return m ? m[0] : null;
};

test('every id of the 1.2.2 page and every id the js asks for is in index.html', () => {
  const jsDir = path.join(WEB, 'js');
  const asked = new Set();
  const painted = new Set();
  for (const f of fs.readdirSync(jsDir)) {
    const js = fs.readFileSync(path.join(jsDir, f), 'utf8');
    for (const m of js.matchAll(/(?:\$|getElementById)\('([\w-]+)'\)/g)) asked.add(m[1]);
    // ids a script writes into the page itself, like #cs-bench
    for (const m of js.matchAll(/id="([\w-]+)"/g)) painted.add(m[1]);
  }
  assert.ok(asked.has('fleet') && asked.has('settings-form'), 'found the ids in the js');
  const missing = IDS_122.concat([...asked]).filter((id) => !painted.has(id) && !tagOf(id));
  assert.deepStrictEqual([...new Set(missing)], []);
  assert.deepStrictEqual(HIDDEN_122.filter((id) => !/\shidden[\s>]/.test(tagOf(id) || '')), [], 'still hidden at load');
  assert.deepStrictEqual(DISABLED_122.filter((id) => !/\sdisabled[\s>]/.test(tagOf(id) || '')), [], 'still disabled at load');
  assert.deepStrictEqual(GONE_14.filter((id) => tagOf(id) || asked.has(id) || painted.has(id)), [], 'gone from the page and the js');
});

test('app.js reaches into #fleet tbody at load, so #fleet is still a table with one', () => {
  assert.match(html, /<table\b[^>]*\bid="fleet"[\s\S]*?<tbody>[\s\S]*?<\/table>/);
  assert.match(tagOf('sel-unit') || '', /^<select\b/);
});

test('the fake page draws a mark the way screens.js does', () => {
  const { page } = require('./mock/fakepage');
  const js = (f) => fs.readFileSync(path.join(WEB, 'js', f), 'utf8');
  const markSrc = /const mark = (\(cls, word\) => [^;]+);/.exec(js('screens.js'));
  const escSrc = /const esc = (\(s\) => [^\n]+);\n/.exec(js('app.js'));
  assert.ok(markSrc && escSrc, 'found mark in screens.js and esc in app.js');
  const real = new Function('esc', 'return ' + markSrc[1])(new Function('return ' + escSrc[1])());
  const fake = page([], {}, {}).shell.mark;
  for (const [cls, word] of [['good', 'OK'], ['bad', 'Lost <3 & "so"'], ['off', 'quiet for 12 s'], ['warn', '']]) {
    assert.strictEqual(fake(cls, word), real(cls, word));
  }
});

// The ring that marks a control the build would replace is on a span round it, but the phase
// card carries the same class, so a bare .changed rule in chipsats.css restyles the card too
// (2026-10-30: it turned the card into an inline-flex row on the first keystroke)
test('chipsats.css styles the changed ring on span.changed only', () => {
  const css = fs.readFileSync(path.join(WEB, 'css', 'chipsats.css'), 'utf8');
  const bare = [...css.matchAll(/^[^{]*?(?:^|[\s,>])\.changed\b[^{]*\{/gm)].map((m) => m[0].trim());
  assert.deepStrictEqual(bare, [], 'selectors that reach every .changed, the phase card included');
});

test('every literal mark() class in the js is one of good, off, warn, bad', () => {
  const dir = path.join(WEB, 'js');
  const bad = [];
  let n = 0;
  for (const f of fs.readdirSync(dir).filter((x) => x.endsWith('.js'))) {
    for (const m of fs.readFileSync(path.join(dir, f), 'utf8').matchAll(/\bmark\(\s*(['"])([^'"]*)\1\s*,/g)) {
      n++;
      if (!['good', 'off', 'warn', 'bad'].includes(m[2])) bad.push(f + ': ' + m[2]);
    }
  }
  // a pattern that stopped matching would pass with nothing checked
  assert.ok(n > 20, 'found ' + n + ' calls');
  assert.deepStrictEqual(bad, []);
});

// sw.js run in a vm with a fake network, cache and timer; no browser needed.
function worker(net, cached) {
  const vm = require('node:vm');
  const handlers = {};
  const timers = [];
  const put = [];
  const sandbox = {
    URL, Promise, location: { origin: 'https://x.test' },
    self: { addEventListener: (n, f) => { handlers[n] = f; } },
    fetch: net,
    setTimeout: (f, ms, v) => { timers.push({ f: () => f(v), ms }); },
    caches: { open: async () => ({ put: async (req, res) => { put.push(res); } }), match: async () => cached },
  };
  vm.runInNewContext(sw, sandbox);
  const ask = (method, url) => {
    let out; const e = { request: { method, url }, respondWith: (p) => { out = p; } };
    handlers.fetch(e);
    return out;
  };
  return { ask, timers, put };
}
const later = () => new Promise((r) => setImmediate(r));

test('sw.js: a network that answers is served and cached', async () => {
  const res = { clone: () => 'copy', body: 'net' };
  const w = worker(async () => res, 'old');
  assert.strictEqual(await w.ask('GET', 'https://x.test/index.html'), res);
  await later();
  assert.deepStrictEqual(w.put, ['copy']);
});

test('sw.js: after 3 s the cached copy is served, and the late network is still cached', async () => {
  let answer;
  const res = { clone: () => 'copy' };
  const w = worker(() => new Promise((r) => { answer = r; }), 'old');
  const p = w.ask('GET', 'https://x.test/index.html');
  assert.strictEqual(w.timers[0].ms, 3000);
  w.timers[0].f();
  assert.strictEqual(await p, 'old');
  answer(res);
  await later();
  assert.deepStrictEqual(w.put, ['copy']);
});

test('sw.js: a failed fetch serves the cache; with no cache the network error stands', async () => {
  assert.strictEqual(await worker(async () => { throw new Error('offline'); }, 'old').ask('GET', 'https://x.test/'), 'old');
  await assert.rejects(worker(async () => { throw new Error('offline'); }, undefined).ask('GET', 'https://x.test/'), /offline/);
});

test('sw.js: a stalled network with nothing cached waits for the network; other requests are left alone', async () => {
  let answer;
  const res = { clone: () => 'copy' };
  const w = worker(() => new Promise((r) => { answer = r; }), undefined);
  const p = w.ask('GET', 'https://x.test/index.html');
  w.timers[0].f();
  await later();
  answer(res);
  assert.strictEqual(await p, res);
  assert.strictEqual(w.ask('POST', 'https://x.test/api'), undefined);
  assert.strictEqual(w.ask('GET', 'https://elsewhere.test/a.js'), undefined);
});
