// The Live screen's cards, summary, detail and runs, from js/livecards.js. Words only:
// how they look is the browser's business.
const test = require('node:test');
const assert = require('node:assert');
const L = require('../web/js/lines.js');
const P = require('../web/js/packet.js');
const { Fleet } = require('../web/js/fleet.js');
const V = require('../web/js/livecards.js');
const { page } = require('./mock/fakepage.js');

const BASE = P.decode(P.hexToBytes('00 00 00 00 00 00 00 00 98 BD FF FF 23 5B 00 00 FD FF 00 00 FD FF 01 00 FF FF FF FF AC FF 18 FF D4 FD 4A 06 8E 02 ED 82 56 1A 7A 0A 7F 26 0F 11 04 00 09 6D 6F 09 23')).values;
function line(more, rssi) {
  const v = Object.assign({}, BASE, more);
  const crc = P.crc16(P.encode(v, 0), 53);
  return 'PKT,55,' + P.bytesToHex(P.encode(v, crc)) + ',' + rssi + ',10.00,0';
}
const text = (html) => html.replace(/<[^>]*>/g, ' ')
  .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&amp;/g, '&')
  .replace(/\s+/g, ' ').trim();
const count = (s, re) => (s.match(re) || []).length;

// CSID 9 as in the design frame: 425 packets, 3.98 V, heard by two receivers
function nine() {
  const f = new Fleet();
  for (let c = 0; c < 425; c++) f.ingest(L.parseLine(line({ csid: 9, counter: c, battery: 3.98, validity: 0x6f }, -73)), c * 250, 'A');
  f.ingest(L.parseLine(line({ csid: 9, counter: 424, battery: 3.98, validity: 0x6f }, -88.5)), 424 * 250 + 100, 'B');
  return f;
}

test('a unit card has the words of the frame and no missed count', () => {
  const f = nine();
  const u = f.units.get(9);
  const t = u.lastT + 200;
  const html = V.unitCardHtml(u, f.state(u, t), t - u.lastT, 2, { runs: 3, staleMs: f.staleAfterMs(u), sel: true, cfg: f.cfg });
  const s = text(html);
  for (const w of ['CSID 9', 'OK', '3 runs', '0.2 s ago', '3.98 V', '-73.0 dBm', '425 pkts', '2 of 2 rx']) assert.ok(s.includes(w), w + ' in ' + s);
  assert.doesNotMatch(s, /missed/i);
  assert.match(html, /data-runs="9"/, 'the runs pill opens the Runs tab');
  assert.match(html, /class="ucard card sel"/);
  assert.equal(count(html, /NOT valid/g), 2, '0x6F clears two bits');
});

test('a unit card and its runs pill keep keyboard focus across the 250 ms repaint', () => {
  const f = nine();
  const u = f.units.get(9);
  const html = V.unitCardHtml(u, f.state(u, u.lastT), 0, 2, { runs: 3, staleMs: f.staleAfterMs(u), sel: false, cfg: f.cfg });
  // shell.paint puts focus back on the element with the same data-keep
  assert.match(html, /class="ucard"[^>]*data-keep="card-9"/);
  assert.match(html, /data-runs="9"[^>]*data-keep="runs-9"|data-keep="runs-9"[^>]*data-runs="9"/);
});

test('the issues line counts lost, stale and low battery, or says there are none', () => {
  const f = new Fleet();
  const feed = (csid, battery, from, n) => {
    for (let c = 0; c < n; c++) f.ingest(L.parseLine(line({ csid, counter: c, battery, validity: 0xff }, -80)), from + c * 1000, 'A');
  };
  feed(7, 3.9, 0, 3);              // lost by the end
  feed(3, 3.9, 700000, 3);         // stale
  feed(12, 3.42, 760000, 3);       // low battery, still OK
  const t = 762500;
  assert.equal(V.issuesText(f, t), '1 lost · 1 stale · 1 low battery');
  const ok = new Fleet();
  for (let c = 0; c < 3; c++) ok.ingest(L.parseLine(line({ csid: 1, counter: c, battery: 4.0, validity: 0xff }, -80)), c * 1000, 'A');
  assert.equal(V.issuesText(ok, 2500), 'No issues');
});

test('the summary counts units and packets with one number format', () => {
  const f = { summary: () => ({ heard: 4, packets: 2588 }) };
  assert.equal(text(V.summaryHtml(f, 0)), '4 units · 2,588 packets');
  assert.equal(text(V.summaryHtml({ summary: () => ({ heard: 1, packets: 425 }) }, 0, 318)), '1 unit · 318 lines read');
});

// Review focus 1
test('no packets yet: 0 units, no stray CSID, a connected receiver with 0 packets', () => {
  const f = new Fleet();
  assert.equal(text(V.summaryHtml(f, 0)), '0 units · 0 packets');
  assert.equal(V.fleetHtml(f, 0, { sort: 'attention', sel: null }), '');
  const none = text(V.detailHtml(null));
  assert.match(none, /Select a unit/);
  assert.doesNotMatch(none, /CSID/);
  const st = V.receiverState(null, { status: 'open' }, 1000);
  const row = text(V.receiverRowHtml({ key: 'rx1', name: 'Mast', id: '', packets: 0, word: st.word, cls: st.cls, index: 0 }));
  assert.match(row, /Mast/);
  assert.match(row, /0 pkts/);
  assert.match(row, /nothing heard/);
});

// Review focus 2
test('a unit with every validity bit clear: every group says not valid, values are dashes', () => {
  const f = new Fleet();
  for (let c = 0; c < 3; c++) f.ingest(L.parseLine(line({ csid: 5, counter: c, validity: 0 }, -80)), c * 1000, 'A');
  const u = f.units.get(5);
  const html = V.detailHtml(u, { state: 'OK', t: 2100, run: 1, runs: 1, rx: [{ name: 'Mast', rssi: -80, snr: 10 }], cfg: f.cfg });
  assert.equal(count(html, /kicker bad">Not valid/g), 6, 'GPS, Environment, Accelerometer, Gyroscope, Magnetometer, Orientation');
  assert.doesNotMatch(html, /NaN|undefined/);
  const s = text(html);
  for (const w of ['Latitude — °', 'Temperature — °C', 'X — m/s²', 'real —', 'Cell voltage — V']) assert.ok(s.includes(w), w + ' in ' + s);
  const card = V.unitCardHtml(u, 'OK', 100, 1, { runs: 1, staleMs: f.staleAfterMs(u), cfg: f.cfg });
  assert.equal(count(card, /NOT valid/g), 8);
  assert.match(text(card), /— V/);
});

test('the detail head says when, which run, and offers the join', () => {
  const f = nine();
  const u = f.units.get(9);
  const t = u.lastT + 200;
  const s = text(V.detailHtml(u, { state: 'OK', t, run: 3, runs: 3, rx: [], cfg: f.cfg }));
  assert.match(s, /^CSID 9 OK Last packet \d\d:\d\d:\d\d · 0\.2 s ago · run 3 of 3 join runs/);
  // the validity letters and the mask in hex, on their own line under the head
  assert.match(V.detailHtml(u, { state: 'OK', t, run: 3, runs: 3, rx: [], cfg: f.cfg }), /<div class="validity-line"><span class="bits">(<span[^>]*>[A-Z]<\/span>){8}<\/span><span class="hex">0x6F<\/span><\/div><\/div>/);
  assert.match(V.detailHtml(u, { state: 'OK', t, run: 1, runs: 1, rx: [], cfg: f.cfg }), /^(?![\s\S]*join runs)/);
  assert.match(text(V.detailHtml(u, { state: 'OK', t, run: 2, runs: 3, rx: [], cfg: f.cfg, replay: true })), /Packet at \d\d:\d\d:\d\d · run 2 of 3/);
});

// Review focus 3
test('thirty runs of one board: the pill says 30 runs and the list has one card', () => {
  const f = new Fleet();
  let t = 0;
  for (let run = 0; run < 30; run++) {
    for (let c = 0; c < 3; c++) f.ingest(L.parseLine(line({ csid: 4, counter: c, validity: 0xff }, -80)), (t += 1000), 'A');
  }
  assert.equal(f.earlierRuns(4).length, 29);
  const html = V.fleetHtml(f, t + 100, { sort: 'attention', sel: '4' });
  assert.equal(count(html, /<tr\b/g), 1);
  assert.match(text(html), /30 runs/);
});

test('deleting a run takes a second click that says what goes', () => {
  const f = new Fleet();
  for (const c of [0, 1, 2, 0, 1]) f.ingest(L.parseLine(line({ csid: 9, counter: c, validity: 0xff }, -80)), f.total * 1000 + 1000, 'A');
  const runs = f.earlierRuns(9).concat([f.units.get(9)]);
  const first = V.runsHtml(9, runs, { sel: '9', ask: null });
  assert.match(first, /data-drop="9a"/);
  assert.doesNotMatch(first, /data-dropgo/);
  const asked = V.runsHtml(9, runs, { sel: '9', ask: '9a' });
  assert.match(text(asked), /Delete run 9a\? Its 3 packets go from this page/);
  assert.match(asked, /data-dropgo="9a"/);
  assert.match(asked, /data-dropno/);
  assert.match(text(asked), /Join into the next run/);
});

test('the module loads into a page and registers as DGLive', () => {
  // fakepage does not hand its window back, so this only shows it runs there
  assert.doesNotThrow(() => page(['packet.js', 'livecards.js'], {}, {}));
});
