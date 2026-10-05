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
