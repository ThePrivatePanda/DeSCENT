// js/replay.js: what the replay bar asks the app for, against a fake app that records
// every request.
const test = require('node:test');
const assert = require('node:assert');
const { Replay, stateFrom } = require('../web/js/replay.js');
const { fakeNet, numbered } = require('./mock/fakestream.js');

function app(answers) {
  const calls = [];
  const fetch = async (url, init) => {
    calls.push({ url, method: (init && init.method) || 'GET', body: init ? init.body : undefined });
    const a = typeof answers === 'function' ? answers(url, init) : null;
    if (a) return a;
    if (url.startsWith('/api/replay/open')) return { ok: true, status: 200, json: async () => ({ id: 'r7' }) };
    if (url === '/api/replay/r7') return { ok: true, status: 200, json: async () => ({ position: 120, lines: 400, packets: 33, from: 1790000000000, playing: true }) };
    return { ok: true, status: 200, json: async () => ({ ok: true }) };
  };
  const net = fakeNet();
  const o = { fetch, WebSocket: net.WebSocket, setTimeout: net.setTimeout, clearTimeout: net.clearTimeout,
    location: { protocol: 'http:', host: '127.0.0.1:8765' }, join: (parts) => parts.join('') };
  return { calls, net, o };
}

test('Open in Replay asks the app to open its own file by name, and listens on the replay\'s stream', async () => {
  const { calls, net, o } = app();
  const r = await Replay.open(o, { path: '2026-10-17/logs/descent-2026-10-17.log' });
  assert.deepStrictEqual(calls, [{ url: '/api/replay/open?path=2026-10-17%2Flogs%2Fdescent-2026-10-17.log', method: 'POST', body: null }]);
  assert.strictEqual(r.id, 'r7');
  assert.deepStrictEqual(net.urls, ['ws://127.0.0.1:8765/api/replay/r7/stream'], 'never the live /api/stream');
});

test('files picked on this computer go up in one body with their names and sizes', async () => {
  const { calls, o } = app();
  await Replay.open(o, { files: [{ name: 'a.log', size: 3, body: 'abc' }, { name: 'lab b.txt', size: 2, body: 'de' }] });
  assert.strictEqual(calls[0].url, '/api/replay/open?name=a.log&size=3&name=lab+b.txt&size=2');
  assert.strictEqual(calls[0].body, 'abcde');
  await Replay.open(o, { files: [{ name: 'one.log', size: 1, body: 'x' }] });
  assert.strictEqual(calls[1].body, 'x');
});

test('the replay bar: play, pause, speed and seek are POSTs to the replay, and seek takes the stream again', async () => {
  const { calls, net, o } = app();
  const r = await Replay.open(o, { path: 'x.log' });
  calls.length = 0;
  await r.play();
  await r.speed('600');
  await r.pause();
  await r.seek(250.4);
  assert.deepStrictEqual(calls.map((c) => [c.method, c.url, c.body]), [
    ['POST', '/api/replay/r7/play', null],
    ['POST', '/api/replay/r7/speed', '{"speed":600}'],
    ['POST', '/api/replay/r7/pause', null],
    ['POST', '/api/replay/r7/seek', '{"position":250}'],
  ]);
  assert.strictEqual(net.sockets.length, 2, 'a seek reconnects for a fresh snapshot');
  assert.strictEqual(net.sockets[0].closed, true);
});

test('the bar reads where the replay is from GET /api/replay/{id}', async () => {
  const { calls, o } = app();
  const r = await Replay.open(o, { path: 'x.log' });
  const st = await r.poll();
  assert.deepStrictEqual(calls[1], { url: '/api/replay/r7', method: 'GET', body: undefined });
  assert.deepStrictEqual([st.position, st.lines, st.packets, st.from, st.playing], [120, 400, 33, 1790000000000, true]);
  assert.strictEqual(r.clock(), 1790000000000, 'with no unit yet the clock is the first line');
  r.stream.opened();
  r.stream.handle(JSON.stringify({ kind: 'unit', csid: 9, label: '9', firstT: 1790000000000, lastT: 1790000005000, seq: 1 }));
  assert.strictEqual(r.clock(), 1790000000000, 'still in the snapshot');
  r.stream.handle(JSON.stringify({ kind: 'snapshot-end', seq: 2 }));
  assert.strictEqual(r.clock(), 1790000005000);
  assert.strictEqual(stateFrom({ position: 3, from: { log: 'a/ground.log', raw: 'a/dump.txt' } }).kept.raw, 'a/dump.txt');
});

test('save CSV and save log are the app\'s; close tells the app and stops listening', async () => {
  const { calls, net, o } = app();
  const r = await Replay.open(o, { path: 'x.log' });
  assert.strictEqual(r.csvUrl, '/api/replay/r7/csv');
  assert.strictEqual(r.logUrl, '/api/replay/r7/log');
  await r.close();
  assert.deepStrictEqual(calls[calls.length - 1], { url: '/api/replay/r7/close', method: 'POST', body: undefined });
  net.last().drop();
  assert.deepStrictEqual(net.runTimers(), [], 'no reconnect after close');
});

test('a graph for a replay comes from the replay\'s own history', async () => {
  const series = { t: [1, 2], counter: [5, 6] };
  const { calls, net, o } = app((url) => (url.includes('/history') ? { ok: true, json: async () => series } : null));
  const r = await Replay.open(o, { path: 'x.log' });
  net.last().open().send(numbered([{ kind: 'unit', csid: 9, label: '9a', live: false, firstT: 1000, lastT: 2000 }, { kind: 'snapshot-end' }]));
  assert.strictEqual(await r.stream.wantHistory('9a'), true);
  assert.strictEqual(calls[calls.length - 1].url, '/api/replay/r7/history?label=9a');
  assert.deepStrictEqual(r.fleet.unit('9a').history.counter, [5, 6]);
});

test('an app that refuses says why, and one without replay says so in words', async () => {
  const { o } = app((url) => (url.startsWith('/api/replay/open')
    ? { ok: false, status: 404, json: async () => { throw new Error('no json'); } } : null));
  await assert.rejects(Replay.open(o, { path: 'x.log' }), /this build of the app cannot do that yet/);
  const b = app((url) => (url.startsWith('/api/replay/open') ? { ok: false, status: 400, json: async () => ({ ok: false, error: 'no such log' }) } : null));
  await assert.rejects(Replay.open(b.o, { path: 'x.log' }), /no such log/);
});
