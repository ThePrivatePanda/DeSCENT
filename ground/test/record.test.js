// tools/record_stream.js against a fake app: the requests it makes, the file it writes, and
// how it stops.
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { record, main } = require('../tools/record_stream.js');
const { fakeNet, numbered } = require('./mock/fakestream.js');

function rig(o) {
  o = o || {};
  const net = fakeNet();
  const calls = [];
  const st = { id: 'r1', position: 10, lines: 10 };
  const later = (fn) => setTimeout(fn, 5);
  const fetch = async (url, init) => {
    const u = new URL(url);
    calls.push((init.method || 'GET') + ' ' + u.pathname + u.search + (init.body ? ' ' + init.body : ''));
    const p = u.pathname;
    if (p === '/api/replay/open') later(() => net.last().open().send(numbered([{ kind: 'settings', lostAfterMs: 600000 }, { kind: 'unit', csid: 9 }, { kind: 'snapshot-end' }], 0)));
    if (p === '/api/replay/r1/seek') st.position = 0;
    if (p === '/api/replay/r1/play') {
      later(() => {
        if (o.dropDuringPlay) { net.last().drop(); return; }
        net.last().send(numbered([{ kind: 'reset' }, { kind: 'snapshot-end' }, { kind: 'packet', csid: 9 }, { kind: 'unit', csid: 9 }], 3));
        st.position = 10;
      });
    }
    const body = p === '/api/replay/open' || p === '/api/replay/r1' ? Object.assign({}, st) : { ok: true };
    return { ok: true, status: 200, json: async () => body };
  };
  const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'record-')), 'stream.jsonl');
  return { net, calls, out, opts: { base: 'http://127.0.0.1:8791/', path: '2026-10-17/logs/1402-ground.log', out, fetch, WebSocket: net.WebSocket, quietMs: 30, pollMs: 5, stallMs: 2000 } };
}

test('it opens the log as a replay, plays it from the start as fast as it goes, and writes every message in order', async () => {
  const r = rig();
  const got = await record(r.opts);
  assert.deepStrictEqual(got, { count: 7, first: 0, last: 6, id: 'r1' });
  assert.deepStrictEqual(r.net.urls, ['ws://127.0.0.1:8791/api/replay/r1/stream']);
  const posts = r.calls.filter((c) => c.startsWith('POST'));
  assert.deepStrictEqual(posts, [
    'POST /api/replay/open?path=2026-10-17%2Flogs%2F1402-ground.log',
    'POST /api/replay/r1/seek {"position":0}',
    'POST /api/replay/r1/speed {"speed":1000000}',
    'POST /api/replay/r1/play',
    'POST /api/replay/r1/close',
  ]);
  const lines = fs.readFileSync(r.out, 'utf8').split('\n').filter(Boolean).map(JSON.parse);
  assert.deepStrictEqual(lines.map((m) => m.seq), [0, 1, 2, 3, 4, 5, 6]);
  assert.deepStrictEqual(lines.map((m) => m.kind), ['settings', 'unit', 'snapshot-end', 'reset', 'snapshot-end', 'packet', 'unit']);
  assert.strictEqual(r.net.sockets[0].closed, true);
});

test('a socket the app closes during the replay is an error that says so, and the replay is still closed', async () => {
  const r = rig({ dropDuringPlay: true });
  await assert.rejects(record(r.opts), /closed ws:\/\/127\.0\.0\.1:8791\/api\/replay\/r1\/stream during the replay, after 3 messages/);
  assert.strictEqual(r.calls[r.calls.length - 1], 'POST /api/replay/r1/close');
});

test('an app that refuses the open says why', async () => {
  const r = rig();
  r.opts.fetch = async () => ({ ok: false, status: 404, json: async () => ({ ok: false, error: 'no such log' }) });
  await assert.rejects(record(r.opts), /POST \/api\/replay\/open\?path=.*: no such log/);
});

test('the command line wants three arguments', async () => {
  const was = process.stderr.write;
  let said = '';
  process.stderr.write = (s) => { said += s; return true; };
  try { assert.strictEqual(await main([]), 2); } finally { process.stderr.write = was; }
  assert.match(said, /^usage: node tools\/record_stream\.js/);
});
