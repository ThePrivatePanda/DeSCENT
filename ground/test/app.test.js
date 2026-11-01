// app.js in test/mock/fakedom.js: the requests the page makes of the app, and what it draws
// from the stream. Words and requests only.
const test = require('node:test');
const assert = require('node:assert');
const { dashboard } = require('./mock/fakedom.js');
const { numbered } = require('./mock/fakestream.js');

const VALID = { accel: true, gyro: true, mag: true, quat: true, gps: true, soc: true, env: true, fresh: true };
const unit = (csid, more) => Object.assign({ kind: 'unit', csid, label: String(csid), live: true, firstT: Date.now() - 5000, lastT: Date.now() - 100,
  packets: 12, missed: 0, resets: 0, repeats: 0, intervalMs: 1000, bestRssi: -80, bestSnr: 9, receptions: [{ rx: 'rx1', rssi: -80, snr: 9 }],
  lastValid: {}, saturatedCount: 0, state: 'OK', issues: [],
  latest: { crcOk: true, crcReceived: 0x1234, values: { csid, counter: 12, battery: 3.9, validity: 0xff, ax: 0, ay: 0, az: 9.8 }, valid: VALID, saturated: false } }, more);
const text = (html) => String(html).replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ').trim();

test('the page loads without the old decoder and sends nothing on load', async () => {
  const p = dashboard();
  await p.settle();
  for (const gone of ['packet.js', 'lines.js', 'fleet.js', 'recorder.js', 'exportcsv.js', 'serial.js']) assert.ok(!p.scripts.includes(gone), gone);
  assert.deepStrictEqual(p.posts(), [], 'never POST on load');
  assert.ok(p.calls.some((c) => c.url === '/api/model-settings' && c.method === 'GET'));
  assert.deepStrictEqual(p.net.urls, ['ws://127.0.0.1:8791/api/stream']);
});

test('live: the unit list is drawn from the stream, and a unit the app calls lost beeps and says so', async () => {
  const p = dashboard();
  await p.settle();
  const ws = p.net.last().open();
  const t0 = Date.now() - 5000;   // the run's first packet: the same on every row of the run, as the app sends it
  ws.send(numbered([unit(9, { firstT: t0 }), { kind: 'receiver', id: 'rx1', name: 'Mast', sf: 9, packets: 12, dups: 0, badCrc: 0, errors: 0, lastLine: Date.now() }, { kind: 'snapshot-end' }]));
  await p.every(250);
  const cards = text(p.$('fleet').querySelector('tbody').innerHTML);
  assert.match(cards, /CSID 9 OK/);
  assert.match(cards, /12 pkts/);
  assert.match(text(p.$('summary').innerHTML), /1 unit · 12 packets/);
  assert.ok(p.calls.some((c) => c.url === '/api/history?label=9'), 'the selected unit\'s graph is asked for');
  ws.send(Object.assign(unit(9, { state: 'LOST', firstT: t0 }), { seq: 4 }));
  await p.every(250);
  assert.match(p.$('toast').textContent, /^CSID 9 lost after/);
  assert.match(text(p.$('fleet').querySelector('tbody').innerHTML), /CSID 9 Lost/);
  assert.deepStrictEqual(p.sounds, [[660, null, 0.18], [660, null, 0.18], [660, null, 0.18]], 'three low notes, and nothing for the snapshot');
});

// The user, 2026-11-01: "beep every time it hears a chipsat. not on every transmission, but
// basically every time it gets the 0 counter packet ... so I also know if a new 'run' started"
test('a run starting is one short chirp: a board heard for the first time, or its counter starting again', async () => {
  const p = dashboard();
  await p.settle();
  const ws = p.net.last().open();
  const t0 = Date.now() - 60000;
  ws.send(numbered([unit(9, { firstT: t0 }), unit(12, { firstT: t0 }), { kind: 'receiver', id: 'rx1', name: 'Mast', sf: 9, packets: 24, dups: 0, badCrc: 0, errors: 0, lastLine: Date.now() }, { kind: 'snapshot-end' }]));
  await p.every(250);
  assert.deepStrictEqual(p.sounds, [], 'the snapshot on load is not news');
  let seq = 10;
  const send = (m) => ws.send(Object.assign(m, { seq: seq++ }));
  // packets and a repeat of the same run from a second receiver: nothing
  send({ kind: 'packet', csid: 9, label: '9', rx: 'rx1', t: Date.now(), rssi: -80, snr: 9, crcOk: true, values: { csid: 9, counter: 13 }, valid: VALID });
  send({ kind: 'packet', csid: 9, label: '9', rx: 'rx2', t: Date.now(), rssi: -85, snr: 7, crcOk: true, dup: true, values: { csid: 9, counter: 13 }, valid: VALID });
  send(unit(9, { firstT: t0, packets: 13, receptions: [{ rx: 'rx1', rssi: -80, snr: 9 }, { rx: 'rx2', rssi: -85, snr: 7 }] }));
  await p.every(250);
  assert.deepStrictEqual(p.sounds, []);
  // a board never heard before
  send(unit(15, { firstT: Date.now() - 500, packets: 1 }));
  await p.every(250);
  assert.deepStrictEqual(p.sounds, [[880, 1320, 0.14]], 'one rising chirp, nothing like the lost alarm');
  await p.every(250);
  assert.equal(p.sounds.length, 1, 'said once');
  // 9 restarts: the app retires its run and sends a new live row with a new first packet time.
  // Its first packet was missed, so the counter seen is 1, and it still counts
  p.sandbox.Date = Date;
  send(unit(9, { label: '9@' + t0, live: false, firstT: t0, state: 'LOST' }));
  send(unit(9, { firstT: Date.now() - 200, packets: 1 }));
  await p.every(250);
  assert.equal(p.sounds.length, 1, 'held off: a second sound within two seconds would be a burst');
  // after the hold-off the restart is heard
  const realNow = Date.now;
  Date.now = () => realNow() + 2500;
  try {
    send(unit(9, { firstT: realNow() - 100, packets: 2 }));
    await p.every(250);
    assert.equal(p.sounds.length, 2, 'the restart chirps');
    assert.deepStrictEqual(p.sounds[1], [880, 1320, 0.14]);
    // two boards up in one pass: one sound
    Date.now = () => realNow() + 6000;
    send(unit(21, { firstT: realNow() + 5000, packets: 1 }));
    send(unit(22, { firstT: realNow() + 5000, packets: 1 }));
    await p.every(250);
    assert.equal(p.sounds.length, 3, 'two new boards in one pass are one chirp');
  } finally { Date.now = realNow; }
});

test('a reconnect\'s snapshot and a replay make no run sound', async () => {
  const p = dashboard();
  await p.settle();
  let ws = p.net.last().open();
  const t0 = Date.now() - 5000;
  ws.send(numbered([unit(9, { firstT: t0 }), { kind: 'snapshot-end' }]));
  await p.every(250);
  // the socket drops; the next connection's snapshot has a board that came up meanwhile
  ws.close();
  await p.every(250);
  ws = p.net.last().open();
  ws.send(numbered([unit(9, { firstT: t0 }), unit(12), { kind: 'snapshot-end' }]));
  await p.every(250);
  assert.deepStrictEqual(p.sounds, [], 'a snapshot is never news, also after a reconnect');
  ws.send(Object.assign(unit(15), { seq: 20 }));
  await p.every(250);
  assert.equal(p.sounds.length, 1);
  // in a replay the live fleet is not listened to
  await p.fire('file-open', 'change', { files: [{ name: 'drop.log', size: 4, text: async () => 'x' }], value: '' });
  await p.settle();
  ws.send(Object.assign(unit(33), { seq: 21 }));
  p.app.view.mode = 'replay';
  await p.every(250);
  p.net.last().open().send(numbered([unit(9, { firstT: t0 }), unit(44), { kind: 'snapshot-end' }]));
  await p.every(250);
  assert.equal(p.sounds.length, 1, 'nothing while replaying');
});

test('the replay bar drives the replay in the app', async () => {
  const p = dashboard();
  await p.settle();
  await p.fire('file-open', 'change', { files: [{ name: 'drop.log', size: 4, text: async () => 'x' }], value: '' });
  const open = p.posts()[0];
  assert.strictEqual(open.url, '/api/replay/open?name=drop.log&size=4');
  assert.deepStrictEqual(p.net.urls.slice(-1), ['ws://127.0.0.1:8791/api/replay/r1/stream']);
  assert.strictEqual(p.$('replay').hidden, false);
  assert.ok(p.calls.some((c) => c.url === '/api/replay/r1' && c.method === 'GET'), 'where it is, asked');
  p.net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  await p.every(250);
  assert.match(p.$('toast').textContent, /^Opened drop\.log: 10 lines, 3 packets/);
  assert.match(text(p.$('rp-file').innerHTML), /line 10 of 10 · 3 packets/);
  const before = p.posts().length;
  // at the end, so Play starts again from the top
  await p.fire('rp-play', 'click');
  assert.deepStrictEqual(p.posts().slice(before).map((c) => c.url + ' ' + (c.body || '')), [
    '/api/replay/r1/speed {"speed":60}', '/api/replay/r1/seek {"position":0}', '/api/replay/r1/play ']);
  await p.fire('rp-speed', 'change', { value: '600' });
  await p.fire('rp-pos', 'change', { value: '4' });
  assert.deepStrictEqual(p.posts().slice(-2).map((c) => c.url + ' ' + c.body), ['/api/replay/r1/speed {"speed":600}', '/api/replay/r1/seek {"position":4}']);
  await p.fire('rp-csv', 'click');
  assert.ok(p.calls.some((c) => c.url === '/api/replay/r1/csv'));
  assert.match(p.$('toast').textContent, /Saved 2 rows to drop\.csv/);
  assert.ok(!p.calls.some((c) => c.url.startsWith('/api/stream')), 'the live stream is never asked to replay');
  await p.fire('rp-exit', 'click');
  assert.strictEqual(p.posts().slice(-1)[0].url, '/api/replay/r1/close');
  assert.strictEqual(p.$('replay').hidden, true);
});

test('Open in Replay names the app\'s own file', async () => {
  const p = dashboard();
  await p.settle();
  await p.app.openServerLog('pull-1/ground.log', { dir: '/data/pull-1' });
  assert.strictEqual(p.posts()[0].url, '/api/replay/open?path=pull-1%2Fground.log');
  assert.strictEqual(p.$('rp-raw').hidden, false, 'a pull has its dump beside it');
});

test('the drawer: a status field posts to the app, a graph field stays in this browser', async () => {
  const p = dashboard({ model: { lostAfterMs: 120000 } });
  await p.settle();
  const input = (key, value) => ({ tagName: 'INPUT', type: 'number', value, dataset: { key } });
  await p.fire('settings-form', 'change', input('lostMin', '3'));
  assert.deepStrictEqual(p.posts().map((c) => c.url + ' ' + c.body), ['/api/model-settings {"lostAfterMs":180000}']);
  assert.strictEqual(p.$('toast').textContent, 'Lost after silence of (min): 3');
  await p.fire('settings-form', 'change', input('maxPoints', '500'));
  assert.strictEqual(p.posts().length, 1, 'no request for a display setting');
  assert.strictEqual(JSON.parse(p.stored['dg-settings-v1']).maxPoints, 500);
  assert.ok(!('lostAfterMs' in JSON.parse(p.stored['dg-settings-v1'])));
  // another screen changes one; it arrives on the stream
  p.net.last().open().send(numbered([{ kind: 'snapshot-end' }, { kind: 'settings', lowBatteryV: 3.7 }]));
  await p.settle();
  assert.strictEqual(p.app.model.values.lowBatteryV, 3.7);
  assert.strictEqual(p.app.live.cfg.lowBatteryV, 3.7);
});
