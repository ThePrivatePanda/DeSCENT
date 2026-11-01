const test = require('node:test');
const assert = require('node:assert');
const N = require('../web/js/serial_native.js');

test('nothing in the dashboard reaches the network', () => {
  // The field has no internet. A CDN font or script added later must fail here.
  const fs = require('node:fs');
  const path = require('node:path');
  const dir = path.join(__dirname, '..', 'web');
  const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]);
  const offenders = [];
  for (const f of walk(dir)) {
    if (!/\.(html|js|css)$/.test(f) || f.includes(path.join('web', 'vendor'))) continue;
    const text = fs.readFileSync(f, 'utf8');
    for (const m of text.matchAll(/(?:src|href)\s*=\s*["']https?:\/\/[^"']+/g)) offenders.push(f + ': ' + m[0]);
  }
  assert.deepStrictEqual(offenders, []);
});

test('a flash with settings puts every rate slot on the request', async () => {
  // The bridge names the keys it forwards. The log gaps were missing from that list on
  // 2026-10-17 and the app answered "no waiting log gap given" to a page that had sent one.
  const calls = [];
  const was = globalThis.fetch;
  globalThis.fetch = async (url, init) => { calls.push({ url, init }); return { ok: true, status: 200, json: async () => ({ ok: true }) }; };
  try {
    const o = { uid: 'u', backup: true, embedded: 'flight', csid: '9', sf: 7, dbm: 14, layout: 4 };
    for (let p = 0; p < 4; p++) { o['fixed' + p] = String(1000 + p); o['follow' + p] = '0'; o['soc' + p] = '75,50,35,20'; o['socms' + p] = '1,2,3,4,5'; }
    await N.NativeHub.prototype.chipFlash.call({}, o);
  } finally {
    globalThis.fetch = was;
  }
  const q = new URL('http://x' + calls[0].url).searchParams;
  assert.equal(q.get('layout'), '4');
  assert.deepStrictEqual([0, 1, 2, 3].map((p) => q.get('fixed' + p)), ['1000', '1001', '1002', '1003']);
  assert.equal(q.get('socms3'), '1,2,3,4,5');
  assert.equal(q.get('module'), null, 'no module on a carried image');
});
