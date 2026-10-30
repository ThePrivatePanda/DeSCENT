// js/stream.js against test/mock/fakestream.js: the app's stream as messages, no app.
const test = require('node:test');
const assert = require('node:assert');
const S = require('../web/js/stream.js');
const { fakeNet, numbered, loadJsonl, fixtures, HEAD } = require('./mock/fakestream.js');

const VALID = { accel: true, gyro: true, mag: true, quat: true, gps: true, soc: true, env: true, fresh: true };
const values = (more) => Object.assign({ lat: 0, lon: 0, gpsAlt: 1, envAlt: 233.79, ax: 0, ay: 0, az: 9.8, gx: 0, gy: 0, gz: 0,
  mx: 0, my: 0, mz: 0, qi: 0, qj: 0, qk: 0, qr: 1, temp: 21, pressure: 983.9, humidity: 43, counter: 0, csid: 9, battery: 3.98, validity: 0xff }, more);
const unit = (csid, more) => Object.assign({ kind: 'unit', csid, label: String(csid), live: true, firstT: 1000, lastT: 1000, packets: 1,
  missed: 0, resets: 0, repeats: 0, intervalMs: null, bestRssi: -73, bestSnr: 9, receptions: [{ rx: 'rx1', rssi: -73, snr: 9 }],
  lastValid: {}, saturatedCount: 0, state: 'OK', issues: [], latest: { crcOk: true, crcReceived: 0x1234, values: values({ csid }), valid: VALID, saturated: false } }, more);
// csid only: which run it is on comes with the unit rows
const pkt = (csid, t, rx, more) => Object.assign({ kind: 'packet', csid, t, rx,
  rssi: -73, snr: 9, crcOk: true, values: values({ csid, counter: t / 1000 }), valid: VALID, hex: 'H' + t }, more);
const rxMsg = (id, more) => Object.assign({ kind: 'receiver', id, name: 'Mast', sf: 9, packets: 10, dups: 2, badCrc: 1, errors: 0, lastLine: 5000, state: 'open' }, more);

function client(o) {
  const net = fakeNet();
  const seen = { lines: [], packets: [], settings: [], ready: 0 };
  const c = new S.StreamClient('ws://x/api/stream', Object.assign({
    WebSocket: net.WebSocket, setTimeout: net.setTimeout, clearTimeout: net.clearTimeout,
    onLine: (m) => seen.lines.push(m), onPacket: (m) => seen.packets.push(m), onSettings: (m) => seen.settings.push(m),
    onReady: () => seen.ready++, log: (s) => seen.log = s,
  }, o)).start();
  return { c, net, seen, f: c.fleet };
}

test('the snapshot is held back until snapshot-end, then shown whole', () => {
  const { c, net, f } = client();
  assert.strictEqual(net.urls[0], 'ws://x/api/stream');
  const ws = net.last().open();
  ws.send(numbered([unit(9), unit(3, { state: 'STALE' }), rxMsg('rx1')]));
  assert.strictEqual(c.ready, false);
  assert.strictEqual(f.units.size, 0, 'nothing on screen half way through a snapshot');
  ws.send({ kind: 'snapshot-end', seq: 4 });
  assert.strictEqual(c.ready, true);
  assert.deepStrictEqual([...f.units.keys()].sort(), [3, 9]);
  assert.strictEqual(f.receivers.get('rx1').unique, 7, '10 packet lines less 2 duplicates and 1 bad CRC');
});

test('unit rows: the app\'s state and issues, sorted worst first, hidden units left out', () => {
  const { net, f } = client();
  net.last().open().send(numbered([
    unit(9), unit(3, { state: 'LOST', lastT: 0 }), unit(12, { issues: ['low battery'] }), unit(5, { state: 'STALE' }),
    { kind: 'snapshot-end' },
  ]));
  assert.deepStrictEqual(f.rows(2000, 'attention').map((r) => r.unit.csid), [3, 5, 12, 9]);
  assert.deepStrictEqual(f.rows(2000, 'csid').map((r) => r.unit.csid), [3, 5, 9, 12]);
  assert.strictEqual(f.rows(2000, 'csid')[0].age, 2000);
  f.hidden.add(5);
  assert.deepStrictEqual(f.rows(2000, 'csid').map((r) => r.unit.csid), [3, 9, 12]);
  const s = f.summary(2000);
  assert.deepStrictEqual([s.heard, s.active, s.stale, s.lost, s.lowBattery], [3, 2, 0, 1, 1]);
  assert.strictEqual(f.unit('9').csid, 9);
  assert.strictEqual(f.unit('77'), null);
  assert.ok(Number.isNaN(f.unit('9').intervalMs), 'null from JSON reads as NaN, as fleet.js had it');
});

test('packets build the graph series: one point per packet, one reception per receiver', () => {
  const { net, f, seen } = client();
  const ws = net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  ws.send(numbered([
    pkt(9, 2000, 'rx1', { rssi: -90 }),
    pkt(9, 2000, 'rx2', { rssi: -70, snr: 11 }),    // the same packet at the second receiver
    pkt(9, 3000, 'rx1', { valid: Object.assign({}, VALID, { gps: false }) }),
    pkt(9, 4000, 'rx1', { crcOk: false }),
  ], 3));
  const h = f.unit('9').history;
  assert.deepStrictEqual(h.t, [2, 3]);
  assert.deepStrictEqual(h.rssi, [-70, -73], 'best of the receivers');
  assert.deepStrictEqual(h.snr, [11, 9]);
  assert.deepStrictEqual(h.lat, [0, null], 'not valid is a gap, not a zero');
  assert.deepStrictEqual(h.battery, [3.98, 3.98]);
  for (const [k, col] of Object.entries(h)) assert.strictEqual(col.length, h.t.length, k);
  assert.deepStrictEqual(Object.keys(f.unit('9').rxHistory).sort(), ['rx1', 'rx2']);
  assert.deepStrictEqual(f.unit('9').rxHistory.rx1.rssi, [-90, -73]);
  assert.strictEqual(seen.packets.length, 4, 'the bench hears every packet, bad CRC too');
});

test('the dup flag, when the app sends it, decides what is the same packet', () => {
  const { net, f } = client();
  const ws = net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  ws.send(numbered([pkt(9, 2000, 'rx1'), pkt(9, 2000, 'rx2', { dup: false }), pkt(9, 2600, 'rx1', { dup: true, hex: 'other' })], 3));
  assert.deepStrictEqual(f.unit('9').history.t, [2, 2]);
});

test('a board that restarts: the new run\'s first packet moves off the old run\'s graph', () => {
  const { net, f } = client();
  const ws = net.last().open().send(numbered([unit(9, { firstT: 1000 }), { kind: 'snapshot-end' }]));
  ws.send(numbered([pkt(9, 1000, 'rx1'), pkt(9, 2000, 'rx1'),
    // the app sends the packet that restarted it before the rows that say so
    pkt(9, 3000, 'rx1', { values: values({ counter: 0 }) }),
    unit(9, { label: '9a', live: false, gen: 0, firstT: 1000, lastT: 2000, packets: 2 }),
    unit(9, { firstT: 3000, lastT: 3000, packets: 1 }),
  ], 3));
  assert.deepStrictEqual(f.unit('9a').history.t, [1, 2]);
  assert.deepStrictEqual(f.unit('9').history.t, [3]);
  assert.deepStrictEqual(f.unit('9').history.counter, [0]);
  assert.deepStrictEqual(f.earlierRuns(9).map((u) => u.label), ['9a']);
  assert.deepStrictEqual([...f.allUnits().keys()].sort(), ['9', '9a']);
  assert.strictEqual(f.units.get(9).firstT, 3000);
});

test('the rows of a restart can come first, and then nothing moves', () => {
  const { net, f } = client();
  const ws = net.last().open().send(numbered([unit(9, { firstT: 1000 }), { kind: 'snapshot-end' }]));
  ws.send(numbered([pkt(9, 1000, 'rx1'),
    unit(9, { label: '9a', live: false, gen: 0, firstT: 1000, lastT: 1000 }),
    unit(9, { firstT: 3000, lastT: 3000 }),
    pkt(9, 3000, 'rx1')], 3));
  assert.deepStrictEqual(f.unit('9a').history.t, [1]);
  assert.deepStrictEqual(f.unit('9').history.t, [3]);
});

test('receivers: counts as sent, last heard from their lines, and what their lines say they are', () => {
  const { net, f, seen } = client();
  const ws = net.last().open().send(numbered([rxMsg('rx1', { lastLine: 'HB,1,2,3' }), rxMsg('rx2'),
    { kind: 'line', rx: 'rx2', t: 4000, text: 'old line in the snapshot' }, { kind: 'snapshot-end' }]));
  assert.strictEqual(f.receivers.get('rx1').lastT, null, 'a lastLine that is not a time is not taken as one');
  assert.strictEqual(seen.lines.length, 0, 'snapshot lines do not reach the taps');
  ws.send(numbered([
    { kind: 'line', rx: 'rx1', t: 6000, text: 'PKT,55,00,-80,9,0' },
    { kind: 'line', rx: 'rx2', t: 6100, text: '#DG,SRC,v1,kind=flash,from=CS9,time_source=anchor' },
  ], 5));
  assert.strictEqual(f.receivers.get('rx1').lastT, 6000);
  assert.strictEqual(f.rxSource.get('rx1'), 'raw');
  assert.strictEqual(f.sources.get('rx2').kind, 'flash');
  assert.deepStrictEqual(seen.lines.map((m) => m.text.slice(0, 3)), ['PKT', '#DG']);
  assert.deepStrictEqual(f.lines.get('rx2').map((l) => l.t), [4000, 6100]);
  f.clearReceivers();
  assert.strictEqual(f.receivers.get('rx2').unique, 0);
  ws.send({ kind: 'receiver', id: 'rx2', packets: 13, dups: 2, badCrc: 1, seq: 7 });
  assert.strictEqual(f.receivers.get('rx2').unique, 3, 'counted from the Clear');
});

test('a gap in seq is counted and the stream goes on', () => {
  const { net, f, c, seen } = client();
  const ws = net.last().open().send({ kind: 'snapshot-end', seq: 1 });
  ws.send({ kind: 'unit', ...unit(9), seq: 2 });
  ws.send({ kind: 'unit', ...unit(3), seq: 6 });
  ws.send({ kind: 'unit', ...unit(4), seq: 7 });
  assert.strictEqual(c.gaps, 3);
  assert.match(seen.log, /3 message\(s\) missing before seq 6/);
  assert.deepStrictEqual([...f.units.keys()].sort(), [3, 4, 9]);
});

test('a dropped stream comes back with backoff and a new snapshot replaces the old one', () => {
  const { net, f, c, seen } = client();
  let ws = net.last().open().send(numbered([unit(9), unit(3), { kind: 'snapshot-end' }]));
  ws.send(pkt(9, 2000, 'rx1', { seq: 4 }));
  ws.drop();
  assert.deepStrictEqual(net.runTimers(), [500]);
  net.last().drop();   // the app is still down
  assert.deepStrictEqual(net.runTimers(), [1000]);
  net.last().drop();
  assert.deepStrictEqual(net.runTimers(), [2000]);
  assert.strictEqual(net.sockets.length, 4);
  ws = net.last().open();
  // seq starts again on a new connection
  ws.send(numbered([unit(9, { packets: 5 }), { kind: 'snapshot-end' }]));
  assert.deepStrictEqual([...f.units.keys()], [9], 'unit 3 is not in the app any more');
  assert.strictEqual(f.unit('9').packets, 5);
  assert.deepStrictEqual(f.unit('9').history.t, [2], 'the same run keeps its graph across a reconnect');
  assert.strictEqual(c.gaps, 0);
  assert.strictEqual(seen.ready, 2);
  ws.drop();
  assert.deepStrictEqual(net.runTimers(), [500], 'a good snapshot resets the wait');
  c.stop();
  net.last().drop();
  assert.deepStrictEqual(net.runTimers(), [], 'stopped means stopped');
});

test('a replay stream does not carry graphs over a new snapshot', () => {
  const { net, f, c } = client({ keepHistory: false });
  net.last().open().send(numbered([unit(9), pkt(9, 2000, 'rx1'), { kind: 'snapshot-end' }]));
  assert.deepStrictEqual(f.unit('9').history.t, [2], 'packets in a snapshot make the graph');
  c.reconnect();
  net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  assert.deepStrictEqual(f.unit('9').history.t, []);
});

test('settings from the app reach the fleet and the drawer, in a snapshot or after', () => {
  const { net, f, seen } = client();
  const ws = net.last().open().send(numbered([{ kind: 'settings', lostAfterMs: 120000, lowBatteryV: 3.6 }, { kind: 'snapshot-end' }]));
  assert.strictEqual(seen.settings.length, 1);
  assert.strictEqual(f.cfg.model.lostAfterMs, 120000);
  assert.strictEqual(f.cfg.lowBatteryV, 3.6, 'under the old name for livecards.js');
  ws.send({ kind: 'settings', accelSatMs2: 70, seq: 3 });
  assert.strictEqual(f.cfg.saturationMps2, 70);
  assert.strictEqual(f.cfg.model.lowBatteryV, 3.6, 'a message with one key changes one key');
  assert.strictEqual(seen.settings.length, 2);
});

test('joining and deleting runs on this page, and they stay joined after a reconnect', () => {
  const { net, f } = client();
  const rows = () => [unit(9, { label: '9a', live: false, gen: 0, firstT: 1000, lastT: 2000, packets: 2, missed: 1 }),
    unit(9, { label: '9b', live: false, gen: 1, firstT: 3000, lastT: 3000, packets: 1 }),
    unit(9, { firstT: 4000, lastT: 5000, packets: 2 })];
  net.last().open().send(numbered([...rows(), pkt(9, 4000, 'rx1'), { kind: 'snapshot-end' }]));
  f.setHistory('9a', { t: [1, 2], counter: [0, 1] });
  assert.strictEqual(f.mergeRun('9a'), '9b');
  assert.strictEqual(f.unit('9b').packets, 3);
  assert.strictEqual(f.unit('9b').resets, 1);
  assert.strictEqual(f.unit('9b').firstT, 1000);
  assert.deepStrictEqual(f.unit('9b').history.t, [1, 2]);
  assert.strictEqual(f.unit('9a'), null);
  assert.ok(f.clearUnit('9b'));
  assert.deepStrictEqual(f.earlierRuns(9), []);
  assert.strictEqual(f.total, 2);
  net.last().drop(); net.runTimers();
  net.last().open().send(numbered([...rows(), { kind: 'snapshot-end' }]));
  assert.deepStrictEqual([...f.allUnits().keys()], ['9'], 'the app still sends 9a and 9b; this page has joined and deleted them');
  f.clearUnits();
  assert.strictEqual(f.units.size, 0);
  net.last().send(unit(9, { firstT: 4000, packets: 3, seq: 9 }));
  assert.strictEqual(f.unit('9').packets, 3, 'the run transmitting now comes back with its next row');
});

test('junk on the socket is ignored; ports and flash frames go to the hub', () => {
  const ports = []; const flash = [];
  const { net, f } = client({ onPorts: () => ports.push(1), onFlash: (m) => flash.push(m.pct) });
  const ws = net.last().open();
  ws.send('not json', JSON.stringify(null), { kind: 'unknown', seq: 1 }, { type: 'ports' }, { type: 'flash', pct: 40 }, { kind: 'snapshot-end', seq: 2 });
  assert.deepStrictEqual([ports.length, flash], [1, [40]]);
  assert.strictEqual(f.units.size, 0);
});

function fetcher(answers) {
  const asked = [];
  const fetch = async (url) => {
    asked.push(url);
    const label = new URL(url, 'http://x').searchParams.get('label');
    const a = answers[label];
    return a ? { ok: true, json: async () => a } : { ok: false, status: 404, json: async () => ({}) };
  };
  return { fetch, asked, historyUrl: (label) => '/api/history?label=' + encodeURIComponent(label) };
}

test('a page that opens late asks the app for the selected unit\'s graph, and the stream extends it', async () => {
  const h = fetcher({ 9: { t: [1, 2, 3], counter: [1, 2, 3], rssi: [-80, -81, null], snr: [9, 9, 9], battery: [3.9, 3.9, 3.9] } });
  const { net, f, c } = client(h);
  assert.strictEqual(c.wantHistory('9'), null, 'not before the snapshot');
  const ws = net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  // a packet arrives while the ask is out
  const asking = c.wantHistory('9');
  assert.strictEqual(c.wantHistory('9'), asking, 'one ask at a time');
  ws.send(pkt(9, 3000, 'rx1', { seq: 3 }), pkt(9, 4000, 'rx1', { seq: 4 }));
  assert.strictEqual(await asking, true);
  assert.deepStrictEqual(h.asked, ['/api/history?label=9']);
  const u = f.unit('9');
  assert.deepStrictEqual(u.history.t, [1, 2, 3, 4], 'the app\'s points, then the stream\'s newer one');
  assert.deepStrictEqual(u.history.counter, [1, 2, 3, 4]);
  assert.deepStrictEqual(u.history.lat, [null, null, null, 0], 'a series the app did not send is gaps');
  for (const [k, col] of Object.entries(u.history)) assert.strictEqual(col.length, 4, k);
  assert.strictEqual(c.wantHistory('9'), null, 'asked once per run');
  ws.send(pkt(9, 5000, 'rx1', { seq: 5 }));
  assert.deepStrictEqual(f.unit('9').history.t, [1, 2, 3, 4, 5]);
  // after a reconnect the selected unit is asked for again
  ws.drop(); net.runTimers();
  net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  await c.wantHistory('9');
  assert.strictEqual(h.asked.length, 2);
  assert.deepStrictEqual(f.unit('9').history.t, [1, 2, 3, 4, 5]);
});

test('the history answer carries every receiver\'s series for the Radio tab, and the stream adds to them', async () => {
  const h = fetcher({ 9: {
    history: { t: [1, 2], counter: [1, 2], rssi: [-70, -71], snr: [9, 9] },
    rxHistory: { rx1: { t: [1, 2], rssi: [-80, -81], snr: [8, 8] }, rx2: { t: [1, 2], rssi: [-70, -71], snr: [9, 9] } },
  } });
  const { net, f, c } = client(h);
  const ws = net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  const asking = c.wantHistory('9');
  // receptions while the ask is out: one the app already had, one newer, one from a third receiver
  ws.send(pkt(9, 2000, 'rx1', { seq: 3 }), pkt(9, 3000, 'rx1', { seq: 4, rssi: -82 }), pkt(9, 3000, 'rx3', { seq: 5, rssi: -90, dup: true }));
  await asking;
  const rh = f.unit('9').rxHistory;
  assert.deepStrictEqual(Object.keys(rh).sort(), ['rx1', 'rx2', 'rx3']);
  assert.deepStrictEqual(rh.rx1.t, [1, 2, 3]);
  assert.deepStrictEqual(rh.rx1.rssi, [-80, -81, -82]);
  assert.deepStrictEqual(rh.rx2.rssi, [-70, -71]);
  assert.deepStrictEqual(rh.rx3.rssi, [-90], 'a receiver heard only since the ask stays');
  assert.deepStrictEqual(f.unit('9').history.t, [1, 2, 3]);
});

test('a reset on the live socket clears what is shown at once, and the snapshot after it is the whole state', () => {
  const { net, f, c, seen } = client();
  const ws = net.last().open().send(numbered([unit(9), unit(3), { kind: 'snapshot-end' }]));
  ws.send(pkt(9, 2000, 'rx1', { seq: 4 }), { kind: 'line', rx: 'rx1', t: 2000, text: 'HB,1,2,3', seq: 5 });
  ws.send({ kind: 'reset', seq: 6 });
  assert.strictEqual(f.units.size, 0);
  assert.strictEqual(f.lines.size, 0);
  ws.send({ kind: 'unit', ...unit(9), seq: 7 }, { kind: 'line', rx: 'rx1', t: 2100, text: 'in the snapshot', seq: 8 });
  assert.strictEqual(f.units.size, 0, 'held until snapshot-end');
  ws.send({ kind: 'snapshot-end', seq: 9 });
  assert.deepStrictEqual([...f.units.keys()], [9]);
  assert.deepStrictEqual(f.unit('9').history.t, [], 'no graph kept over a reset, even live');
  assert.strictEqual(seen.lines.length, 1, 'the snapshot\'s line is not news to the taps');
  assert.strictEqual(c.gaps, 0, 'seq goes on across a reset on the same socket');
  assert.strictEqual(net.sockets.length, 1);
});

test('the history answer as the app sends it: the flat series with rxHistory beside it', async () => {
  const h = fetcher({ 9: { t: [1, 2], counter: [1, 2], rssi: [-70, -71], snr: [9, 9], battery: [3.9, 3.8],
    rxHistory: { rx1: { t: [1, 2], rssi: [-80, -81], snr: [8, 8] } } } });
  const { net, f, c } = client(h);
  net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  assert.strictEqual(await c.wantHistory('9'), true);
  const u = f.unit('9');
  assert.deepStrictEqual(u.history.battery, [3.9, 3.8]);
  assert.ok(!('rxHistory' in u.history), 'rxHistory is not a graph series');
  assert.deepStrictEqual(u.rxHistory.rx1.rssi, [-80, -81]);
});

test('the head of the app\'s stream for 1402-ground.log: what the page holds at each step', () => {
  const msgs = loadJsonl(HEAD);
  const end = msgs.findIndex((m) => m.kind === 'snapshot-end');
  const { net, f, c, seen } = client({ keepHistory: false });
  const ws = net.last().open();
  ws.send(msgs.slice(0, end + 1));
  // the snapshot as the app keeps it at the end of the log
  const rows = new Set(msgs.slice(0, end).filter((m) => m.kind === 'unit').map((m) => m.label));
  assert.deepStrictEqual(new Set(f.allUnits().keys()), rows);
  assert.strictEqual(f.receivers.size, new Set(msgs.slice(0, end).filter((m) => m.kind === 'receiver').map((m) => m.id)).size);
  const rx2 = f.receivers.get('rx2');
  assert.deepStrictEqual([rx2.packets, rx2.unique, rx2.badCrc, rx2.radioErrors, rx2.id], [174, 173, 0, 112, '73C434']);
  assert.strictEqual(f.rxSource.get('rx2'), 'raw', 'the receiver row says its format');
  for (const l of f.lines.values()) assert.ok(l.length <= S.LINES_KEPT);
  assert.ok(f.lines.get('rx2').length > 0, 'the snapshot\'s lines fill the raw boxes');
  assert.strictEqual(seen.lines.length, 0, 'and are not news to the taps');
  assert.strictEqual(f.cfg.model.lostAfterMs, 600000);
  // the replay seeks back to the start: everything goes, and the log plays again
  ws.send(msgs.slice(end + 1));
  assert.strictEqual(c.gaps, 0);
  const played = msgs.slice(end + 1).filter((m) => m.kind === 'packet');
  assert.ok(played.length > 0);
  const points = [...f.allUnits().values()].reduce((a, u) => a + u.history.t.length, 0);
  assert.strictEqual(points, played.filter((m) => m.crcOk && !m.dup).length, 'one graph point per new packet with a good CRC');
  const heard = [...f.allUnits().values()].reduce((a, u) => a + Object.values(u.rxHistory).reduce((b, h) => b + h.t.length, 0), 0);
  assert.strictEqual(heard, played.filter((m) => m.crcOk).length, 'one reception per packet message');
  assert.ok(f.allUnits().size < rows.size, 'only the units heard so far after the seek');
});

test('a history ask the app cannot answer leaves the graph as the stream drew it', async () => {
  const h = fetcher({});
  const { net, f, c } = client(h);
  const ws = net.last().open().send(numbered([unit(9), { kind: 'snapshot-end' }]));
  ws.send(pkt(9, 3000, 'rx1', { seq: 3 }));
  assert.strictEqual(await c.wantHistory('9'), false);
  assert.deepStrictEqual(f.unit('9').history.t, [3]);
  assert.strictEqual(c.wantHistory('77'), null, 'no row, no ask');
  assert.strictEqual(c.wantHistory('9'), null, 'not asked again on the next redraw');
  assert.strictEqual(h.asked.length, 1);
});

test('the app\'s recorded streams load and end their snapshot', () => {
  assert.ok(fixtures().includes(HEAD), 'the head of 1402-ground is in test/data');
  for (const file of fixtures()) {
    const msgs = loadJsonl(file);
    const { net, f, c } = client({ keepHistory: false });
    net.last().open().send(msgs);
    assert.ok(c.ready, file + ' has a snapshot-end');
    assert.strictEqual(c.gaps, 0, file + ' has no seq gaps');
    for (const u of f.allUnits().values()) {
      assert.ok(u.label && Number.isFinite(u.csid), file + ': unit ' + u.label);
      for (const [k, col] of Object.entries(u.history)) assert.strictEqual(col.length, u.history.t.length, file + ' ' + u.label + ' ' + k);
    }
  }
});
