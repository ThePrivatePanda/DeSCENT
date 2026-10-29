const test = require('node:test');
const assert = require('node:assert');
const N = require('../web/js/serial_native.js');

test('native transport whenever the page came over plain http, from this computer or another screen', () => {
  assert.equal(N.isNativeHost({ protocol: 'http:', hostname: '127.0.0.1' }), true);
  assert.equal(N.isNativeHost({ protocol: 'http:', hostname: 'localhost' }), true);
  assert.equal(N.isNativeHost({ protocol: 'file:', hostname: '' }), false);
  assert.equal(N.isNativeHost({ protocol: 'https:', hostname: 'ground.privatepanda.co' }), false);
  assert.equal(N.isNativeHost({ protocol: 'http:', hostname: '192.168.1.5' }), true);
});

test('a line frame reaches the page as receiver key and text', () => {
  const seen = [];
  const handle = N.makeFrameHandler((rx, text) => seen.push([rx, text]), () => {});
  handle(JSON.stringify({ type: 'line', rx: 'rx1', t: 1790000000000, text: 'HB,1,2,3' }));
  assert.deepStrictEqual(seen, [['rx1', 'HB,1,2,3']]);
});

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

test('junk on the socket is ignored, not thrown', () => {
  const handle = N.makeFrameHandler(() => { throw new Error('should not be called'); }, () => {});
  handle('not json');
  handle(JSON.stringify({ type: 'unknown' }));
});

test('a flash with settings puts every phase rate and log gap on the request', async () => {
  // The bridge names the keys it forwards. The log gaps were missing from that list on
  // 2026-10-17 and the app answered "no waiting log gap given" to a page that had sent one.
  const calls = [];
  const was = globalThis.fetch;
  globalThis.fetch = async (url, init) => { calls.push({ url, init }); return { ok: true, status: 200, json: async () => ({ ok: true }) }; };
  try {
    const o = { uid: 'u', backup: true, embedded: 'flight', csid: '9', sf: 7, dbm: 14 };
    for (let p = 0; p < 4; p++) { o['fixed' + p] = '1000'; o['follow' + p] = '0'; o['soc' + p] = '75,50,35,20'; o['socms' + p] = '1,2,3,4,5'; o['log' + p] = String(50 + p); }
    o.logblind = '200';
    await N.NativeHub.prototype.chipFlash.call({}, o);
  } finally {
    globalThis.fetch = was;
  }
  const q = new URL('http://x' + calls[0].url).searchParams;
  assert.equal(q.get('fixed2'), '1000');
  assert.deepStrictEqual([0, 1, 2, 3].map((p) => q.get('log' + p)), ['50', '51', '52', '53']);
  assert.equal(q.get('logblind'), '200');
  assert.equal(q.get('module'), null, 'no module on a carried image');
});
