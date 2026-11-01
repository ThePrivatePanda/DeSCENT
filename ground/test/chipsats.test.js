// The ChipSats screen's own script, run against test/mock/fakepage.js. This checks what it
// paints and what it asks the app for. It says nothing about how the page looks.
const test = require('node:test');
const assert = require('node:assert');
const { page } = require('./mock/fakepage.js');

const UID9 = '2032374555435010002C002E';
const UID12 = '0038002A5553500920393301';
const PROBE = { serial: '002A00', kind: 'STLink V3', access: true };
const CS9 = { uid: UID9, name: 'CS9', module: 'le', lastSeen: 1, firmware: 'dump' };
const CS12 = { uid: UID12, name: 'CS12', module: 'le', lastSeen: 2, firmware: 'flight', firmwareModule: 'le' };
const RATE = (fixedMs, more) => Object.assign({ fixedMs, follow: false, percent: [75, 50, 35, 20], intervalMs: [10000, 30000, 60000, 120000, 180000] }, more || {});
const FLAT = (ms, more) => RATE(ms, Object.assign({ intervalMs: [ms, ms, ms, ms, ms] }, more || {}));
const block = (o) => Object.assign({ layoutVersion: 4, csid: 9, module: 'le', sf: 7, dbm: 14, setBy: 'build', txIntervalMs: 10000, transmit: [RATE(60000), RATE(10000)], log: [FLAT(1000), FLAT(50)], olderSlots: [], reservedClear: true }, o);
// untouched defaults with no block on the board, ground logs every 10 s
// the fixture block's rates as the app is asked for them (slots: ground tx, air tx, ground log, air log)
const BLOCK_RATES = { layout: 4 };
[60000, 10000, 1000, 50].forEach((fixed, i) => {
  BLOCK_RATES['fixed' + i] = String(fixed); BLOCK_RATES['follow' + i] = '0';
  BLOCK_RATES['soc' + i] = '75,50,35,20';
  BLOCK_RATES['socms' + i] = i < 2 ? '10000,30000,60000,120000,180000' : Array(5).fill(fixed).join(',');
});
const readOf = (uid, firmware, settings) => ({ ok: true, uid, flashBytes: 262144, firmware, settings, settingsError: null, probe: PROBE });
const DUMP = readOf(UID9, { kind: 'dump', current: true, id: '25118bdfafab456c' }, null);
// What this browser remembers of CS9 (the fixture block, as the boxes hold it), seeded as o.storage
const REMEMBERED = { csid: '9', sf: 7, dbm: 14 };
[['p0', 60, 1], ['p1', 10, 0.05]].forEach(([p, tx, log]) => {
  REMEMBERED[p + 's'] = String(tx); REMEMBERED[p + 'l'] = String(log); REMEMBERED[p + 'f'] = false;
  [75, 50, 35, 20].forEach((v, k) => { REMEMBERED[p + 'soc' + k] = String(v); });
  [10, 30, 60, 120, 180].forEach((v, k) => { REMEMBERED[p + 'ms' + k] = String(v); REMEMBERED[p + 'lms' + k] = String(log); });
});
const KEPT = () => ({ ['dg-draft-' + UID9]: JSON.stringify(REMEMBERED) });
const FLIGHT = readOf(UID9, { kind: 'flight', module: 'le', id: 'c4fb204d8f79e725' }, block());

// The screen with a fake app behind it. o.read is what a read answers, o.under what
// answers the id check (the board that was read, unless a test says otherwise).
async function chipsats(o) {
  o = Object.assign({ screen: 'chipsats', chips: [CS9], probes: [PROBE], read: DUMP, logs: [], reads: 0, peeks: 0 }, o);
  const hub = {
    native: true, ports: [], boards: o.boards || [],
    chips: async () => o.chips, chipProbes: async () => o.probes, logs: async () => o.logs, radio: async () => null,
    health: async () => o.health || null,
    chipRead: async () => { o.reads++; if (o.readFails) throw new Error('no board answered'); return o.read; },
    chipPeek: async () => { o.peeks++; return o.under === undefined ? o.read.uid : o.under; },
    chipFlash: async (q) => { o.flashed = q; },
    chipForget: async (uid) => { o.forgot = uid; o.chips = o.chips.filter((c) => c.uid !== uid); },
    pull: async (port, keepFsw, more) => { o.pulled = { port, keepFsw, more }; },
    pullStop: async () => { o.stops = (o.stops || 0) + 1; },
  };
  if (o.fsw !== undefined) {
    hub.fsw = async () => o.fsw;
    hub.health = async () => { o.healths = (o.healths || 0) + 1; return o.health || null; };
    hub.radio = async () => { o.radios = (o.radios || 0) + 1; return null; };
  }
  const p = page(['boardmodel.js', 'datamodel.js', 'chipsats.js'], hub, o);
  await p.settle();
  p.parts.chipsats.show();
  await p.settle();
  return Object.assign(p, { o, sheet: () => p.html('cs-sheet'), list: () => p.html('cs-list'), words: () => tidy(p.text('cs-sheet')) });
}
// text() spaces out every tag, so "<b>CS9</b>." reads "CS9 ."
const tidy = (t) => t.replace(/ ([,.;:?!)])/g, '$1');
// Each value struck through, in the order it first appears, with how often: once beside
// its control and once in the Write step, so a strike missing from either shows
function struck(html) {
  const n = new Map();
  for (const s of html.match(/<span class="[^"]*\bwas\b[^"]*">[^<]*<\/span>/g) || []) {
    const v = s.replace(/<[^>]*>/g, '');
    n.set(v, (n.get(v) || 0) + 1);
  }
  return [...n];
}

test('a value on the board is struck through only when the build would replace it', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  assert.deepStrictEqual(struck(p.sheet()), []);
  await p.fire('screen-chipsats', 'change', { 'data-build': 'sf' }, { tagName: 'SELECT', value: '12' });
  assert.deepStrictEqual(struck(p.sheet()), [['SF7', 2]]);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1s' }, { tagName: 'INPUT', value: '0.5' });
  assert.deepStrictEqual(struck(p.sheet()), [['SF7', 2], ['10 s', 2]]);
  // The log gap is its own box, in the same row.
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1l' }, { tagName: 'INPUT', value: '0.1' });
  assert.match(p.sheet(), /data-build="p1l" value="0.1"/);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p0l' }, { tagName: 'INPUT', value: '10' });
  assert.deepStrictEqual(struck(p.sheet()), [['SF7', 2], ['1 s', 2], ['0.05 s', 2], ['10 s', 2]]);
});

// 2026-10-31: a one-press write put 255, SF7 and 14 dBm on a real board, because a board on the
// dump firmware has no block and the draft fell back to defaults. The user: "I'd rather have the
// values empty ... than some default".
test('a board with no settings block and nothing remembered starts with every box empty, and the write refuses naming them', async () => {
  const p = await chipsats({});
  const h = p.sheet();
  assert.match(h, /The dump firmware has no radio settings\./);
  assert.match(h, /data-build="csid" value=""/);
  for (const k of ['p0s', 'p0l', 'p1s', 'p1l']) assert.match(h, new RegExp('data-build="' + k + '" value=""'), k);
  assert.doesNotMatch(h, /value="7" selected|value="14" selected/, 'nothing picked');
  assert.match(h, /<select data-build="sf"><option value="" selected disabled hidden>—<\/option>/);
  assert.match(h, /data-buildembed="1"[^>]*disabled/);
  assert.match(p.words(), /Fill in the board id, the spreading factor, the transmit power and 4 boxes in Phases first\./);
  assert.match(p.words(), /Build with board —, LE, SF—, — dBm/);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed, undefined, 'nothing goes to the board');
  await p.fire('screen-chipsats', 'input', { 'data-build': 'csid' }, { tagName: 'INPUT', value: '9' });
  await p.fire('screen-chipsats', 'change', { 'data-build': 'sf' }, { tagName: 'SELECT', value: '7' });
  await p.fire('screen-chipsats', 'change', { 'data-build': 'dbm' }, { tagName: 'SELECT', value: '14' });
  assert.match(p.words(), /Fill in 4 boxes in Phases first\./);
  for (const [k, v] of [['p0s', '60'], ['p0l', '1'], ['p1s', '10']]) await p.fire('screen-chipsats', 'input', { 'data-build': k }, { tagName: 'INPUT', value: v });
  assert.match(p.words(), /Fill in 1 box in Phases first\./);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1l' }, { tagName: 'INPUT', value: '0.05' });
  assert.match(p.sheet(), /data-buildembed="1"(?![^>]*disabled)[^>]*>/);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  const f = p.o.flashed;
  assert.equal(f.csid, '9'); assert.equal(f.sf, 7); assert.equal(f.dbm, 14);
  assert.deepStrictEqual([f.fixed0, f.fixed1, f.fixed2, f.fixed3], ['60000', '10000', '1000', '50']);
  assert.doesNotMatch(JSON.stringify(f), /NaN|null/, 'no blank became a number');
  // the switch is off: the band table the app insists on is the typed gap in every band, and the
  // firmware's own thresholds, which it does not read while the switch is off
  assert.equal(f.socms0, '60000,60000,60000,60000,60000');
  assert.equal(f.socms3, '50,50,50,50,50');
  assert.equal(f.soc0, '75,50,35,20');
});

test('the id 255 every board is built with is refused in words, as the other refusals are', async () => {
  const p = await chipsats({ read: readOf(UID9, FLIGHT.firmware, block({ csid: 255 })), chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  assert.match(p.sheet(), /data-buildembed="1"[^>]*disabled/);
  assert.match(p.words(), /Build and flash CS9 Set a board id first\./);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed, undefined);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'csid' }, { tagName: 'INPUT', value: '12' });
  assert.match(p.sheet(), /data-buildembed="1"(?![^>]*disabled)[^>]*>/);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed.csid, '12');
});

test('a block read off a board is remembered, and fills its boxes when it comes back with no block, after a reload too', async () => {
  const first = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  const kept = JSON.parse(first.storage['dg-draft-' + UID9]);
  assert.equal(kept.csid, '9'); assert.equal(kept.p0s, '60'); assert.equal(kept.p1lms4, '0.05');
  assert.equal(kept.uid, undefined);
  // the same browser, the page reloaded, the board back on the dump firmware after a pull
  const p = await chipsats({ read: DUMP, storage: first.storage });
  const h = p.sheet();
  assert.match(h, /data-build="csid" value="9"/);
  assert.match(h, /data-build="p1l" value="0.05"/);
  assert.match(h, /<option value="7" selected>SF7/);
  assert.match(h, /data-buildembed="1"(?![^>]*disabled)[^>]*>/);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed.csid, '9');
  assert.equal(p.o.flashed.socms0, '10000,30000,60000,120000,180000');
});

test('a flash that ends well remembers the draft it wrote, a flash that fails does not', async () => {
  const p = await chipsats({ storage: KEPT() });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'csid' }, { tagName: 'INPUT', value: '21' });
  await p.fire('screen-chipsats', 'change', { 'data-build': 'sf' }, { tagName: 'SELECT', value: '11' });
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed.csid, '21');
  p.jobs.chip = { what: 'flash', uid: UID9, running: false, steps: ['x'], error: 'did not answer afterwards', read: null };
  await p.jobEnded('chip');
  assert.equal(JSON.parse(p.storage['dg-draft-' + UID9]).csid, '9', 'a failed write changes nothing');
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  p.jobs.chip = { what: 'flash', uid: UID9, running: false, steps: ['x'], error: null, read: null };
  await p.jobEnded('chip');
  const kept = JSON.parse(p.storage['dg-draft-' + UID9]);
  assert.equal(kept.csid, '21'); assert.equal(kept.sf, 11); assert.equal(kept.p0s, '60');
});

test('each phase has one table: a switch and thresholds for both columns, a log and a transmit box per row', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  let h = p.sheet();
  assert.match(p.words(), /\bPhases\b/);
  assert.equal((h.match(/data-build="p\ds"/g) || []).length, 2, 'two phases, ground and air');
  assert.match(p.words(), /\bGround\b.*\bAir\b/);
  assert.doesNotMatch(h, /Falling|Landed|Waiting/);
  assert.match(h, /data-build="p0s" value="60"/);
  assert.match(h, /data-build="p1s" value="10"/);
  assert.doesNotMatch(h, /data-build="p2/);
  assert.match(h, /data-build="p0l" value="1"/);
  assert.match(h, /data-build="p1l" value="0.05"/);
  assert.doesNotMatch(h, /No altitude source|blind|data-build="lb"/);
  assert.match(h, /30 m on GPS/);
  assert.match(h, /0 writes nothing, not even the time anchors/);
  assert.match(h, /0 is every reading/);
  assert.match(h, /On the ground the readings mostly stay in the board's RAM, and only one per ground log gap reaches flash\./);
  assert.match(h, /the air's battery bands apply only while the board logs as air/);
  assert.equal((h.match(/Set by battery level \(changes both columns\)/g) || []).length, 2, 'a switch per phase');
  assert.equal((h.match(/>Always</g) || []).length, 2, 'off, one row for each');
  assert.doesNotMatch(h, /Gauge unread<|data-build="p\dsoc0"|data-build="p\dlms0"/, 'no bands while nothing follows');
  assert.match(p.words(), /Battery Log every Transmit every On the board Always/);
  assert.deepStrictEqual(struck(h), []);
  // Ground on the battery: its bands appear, the air's do not, five rows and the gauge unread.
  await p.fire('screen-chipsats', 'change', { 'data-build': 'p0f' }, { tagName: 'INPUT', type: 'checkbox', checked: true });
  h = p.sheet();
  assert.doesNotMatch(h, /data-build="p1soc0"/, 'only the ground has bands');
  assert.match(h, /data-build="p0soc0" value="75"/);
  assert.match(h, /data-build="p0ms4" value="180"/);
  assert.match(h, /data-build="p0lms4" value="1"/);
  assert.match(p.words(), /Above[^]*To[^]*To[^]*To[^]*Below 20 %[^]*Gauge unread/);
  assert.equal((h.match(/>Always</g) || []).length, 1);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p0soc1' }, { tagName: 'INPUT', value: '60' });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p0ms4' }, { tagName: 'INPUT', value: '300' });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p0lms4' }, { tagName: 'INPUT', value: '20' });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1s' }, { tagName: 'INPUT', value: '5' });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1l' }, { tagName: 'INPUT', value: '0' });
  h = p.sheet();
  // the rates are the tables' and the Write step's rows, not a sentence of their own since 2026-10-31
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  const f = p.o.flashed;
  assert.equal(f.layout, 4);
  // ground transmit, then ground log: one flag and one set of thresholds, each its own gaps
  for (const i of [0, 2]) {
    assert.equal(f['follow' + i], '1');
    assert.equal(f['soc' + i], '75,60,35,20');
  }
  assert.equal(f.fixed0, '60000');
  assert.equal(f.socms0, '10000,30000,60000,120000,300000');
  assert.equal(f.fixed2, '1000');
  assert.equal(f.socms2, '1000,1000,1000,1000,20000');
  for (const i of [1, 3]) {
    assert.equal(f['follow' + i], '0');
    assert.equal(f['soc' + i], '75,50,35,20');
  }
  assert.equal(f.fixed1, '5000');
  assert.equal(f.fixed3, '0');
  assert.equal(f.logblind, undefined);
  assert.equal(f.log0, undefined);
});

test('ticking the switch does not change logging until a box is edited, whatever the log box says', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  // the bands stay at 0.05 s on a board that never used them, so this is the case that needs seeding
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1l' }, { tagName: 'INPUT', value: '0.2' });
  await p.fire('screen-chipsats', 'change', { 'data-build': 'p1f' }, { tagName: 'INPUT', type: 'checkbox', checked: true });
  const h = p.sheet();
  for (let k = 0; k < 5; k++) assert.match(h, new RegExp('data-build="p1lms' + k + '" value="0.2"'));
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed.fixed3, '200');
  assert.equal(p.o.flashed.socms3, '200,200,200,200,200');
  assert.equal(p.o.flashed.follow3, '1');
  assert.equal(p.o.flashed.follow1, '1', 'the transmit column has the same switch');
});

test('the write step lists what changes row by row, and beside a 0 the box says what it means', async () => {
  const chips = [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })];
  const p = await chipsats({ read: FLIGHT, chips });
  await p.fire('screen-chipsats', 'change', { 'data-build': 'p1f' }, { tagName: 'INPUT', type: 'checkbox', checked: true });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1lms4' }, { tagName: 'INPUT', value: '5' });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1lms0' }, { tagName: 'INPUT', value: '0' });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p0l' }, { tagName: 'INPUT', value: '0' });
  const w = p.words();
  assert.match(w, /Air, below 20 %, log 0\.05 s → 5 s/);
  assert.match(w, /Air, above 75 %, log 0\.05 s → every reading/);
  assert.match(w, /Air, log by battery level off → on/);
  assert.match(w, /Air, transmit by battery level off → on/);
  assert.match(w, /Air, to 50 %, transmit 10 s → 30 s/);
  assert.match(w, /Ground, always, log 1 s → nothing written/);
  assert.doesNotMatch(w, /Air, to 50 %, log/, 'a row that gives the same gap is not a change');
  assert.match(p.sheet(), /<span class="zero">every reading<\/span>/);
  assert.match(p.sheet(), /<span class="zero">nothing written<\/span>/);
  // A band threshold moved is one change, the page writes it to both columns.
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1soc3' }, { tagName: 'INPUT', value: '25' });
  assert.equal((p.words().match(/Air, threshold 4 20 % → 25 %/g) || []).length, 1);
});

test('a board whose log and transmit columns differ is told so and nothing more', async () => {
  const apart = block({ log: [FLAT(1000, { follow: true }), FLAT(50, { percent: [70, 55, 30, 10] })] });
  const p = await chipsats({ read: readOf(UID9, { kind: 'flight', module: 'le', id: 'c4fb204d8f79e725' }, apart),
    chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  const w = p.words();
  assert.match(w, /On the board the log follows the battery and transmit does not\./);
  assert.match(w, /On the board the thresholds differ: log 70, 55, 30, 10 %, transmit 75, 50, 35, 20 %\./);
});

test('a board on firmware from before the two phases shows its other slots and is replaced in both', async () => {
  const older = [{ phase: 1, name: 'falling', rate: RATE(1000), logMs: 50 }];
  const p = await chipsats({ read: readOf(UID9, { kind: 'flight', module: 'le', id: 'c4fb204d8f79e725' }, block({ layoutVersion: 3, olderSlots: older })),
    chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  assert.match(p.words(), /On the board: falling slot every 1 s, log every 0\.05 s changed/);
  assert.match(p.words(), /Air, falling slot every 1 s → gone: the block has one slot a phase/);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed.fixed1, '10000');
  assert.equal(p.o.flashed.fixed3, '50', 'slot 3 is the air log now, never the old landed transmit gap');
});

test('a board from before the phases has empty rate boxes, and the write says so', async () => {
  const p = await chipsats({ read: readOf(UID9, { kind: 'flight', module: 'le', id: 'c4fb204d8f79e725' }, block({ layoutVersion: 1, transmit: null, log: null })),
    chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  const h = p.sheet();
  assert.match(h, /data-build="csid" value="9"/, 'what the block has is used');
  assert.match(h, /data-build="p0s" value=""/, 'what it has not is empty, never a default');
  assert.match(h, /data-build="p0l" value=""/);
  assert.match(p.words(), /Fill in 4 boxes in Phases first\./);
});

test('a layout 2 block has transmit rates and no log gaps, and the write gives it them', async () => {
  const p = await chipsats({ read: readOf(UID9, { kind: 'flight', module: 'le', id: 'c4fb204d8f79e725' }, block({ layoutVersion: 2, log: null })),
    chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  assert.match(p.sheet(), /data-build="p1s" value="10"/, 'its own transmit gap');
  assert.match(p.sheet(), /data-build="p1l" value=""/, 'and no log gap until one is typed');
  assert.match(p.words(), /Fill in 2 boxes in Phases first\./);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p0l' }, { tagName: 'INPUT', value: '1' });
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1l' }, { tagName: 'INPUT', value: '0.05' });
  assert.match(p.words(), /Air, log no log gaps → every 0\.05 s/);
});

test("the drone's flight controller is never offered as a console", async () => {
  // Three pulls on 2026-10-16 read MAVLink off /dev/ttyACM0 because it was first in the list.
  const boards = [
    { port: '/dev/ttyACM0', usb: '1209:5741', label: 'flight controller', serial: '', key: '1209:5741:x', state: 'waiting', name: '' },
    { port: '/dev/ttyUSB0', usb: '1a86:7523', label: 'CH340', serial: '', key: '1a86:7523:-', state: 'waiting', name: '' },
  ];
  const p = await chipsats({ read: FLIGHT, boards, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  const h = p.sheet();
  assert.match(h, /<option value="\/dev\/ttyACM0" disabled>\/dev\/ttyACM0, the drone's flight controller<\/option>/);
  assert.match(h, /<option value="\/dev\/ttyUSB0" selected>\/dev\/ttyUSB0<\/option>/, 'the CH340 is picked although it is second');
});

test('how a flash ended stays on screen until it is dismissed, on any screen', async () => {
  const p = await chipsats({ read: DUMP });
  p.o.screen = 'live';
  p.jobs.chip = { what: 'flash', uid: UID9, steps: [{ pct: 100 }], read: FLIGHT };
  await p.jobEnded('chip');
  assert.equal(p.el('flash-done').hidden, false);
  assert.match(p.text('flash-done-body'), /Flashed and checked/);
  assert.match(p.text('flash-done-body'), /CS9/);
  assert.match(p.text('flash-done-body'), /\d\d:\d\d:\d\d/);
  await p.ticks(30);
  assert.equal(p.el('flash-done').hidden, false, 'time alone does not take it away');
  await p.fire('flash-done-ok', 'click');
  assert.equal(p.el('flash-done').hidden, true);
});

test('a failed flash says why in the same box, and a pull or a clear opens it too', async () => {
  const p = await chipsats({ read: DUMP });
  p.jobs.chip = { what: 'flash', uid: UID9, error: 'the board\'s settings differ from what was written: board id', steps: [] };
  await p.jobEnded('chip');
  assert.equal(p.el('flash-done').hidden, false);
  assert.match(p.text('flash-done-body'), /Flash failed/);
  assert.match(p.text('flash-done-body'), /differ from what was written: board id/);
  await p.fire('flash-done-ok', 'click');
  p.jobs.pull = { uid: UID9, running: false, keepFsw: true, restored: true, log: '2026-10-22/pulls/1402-CS9/ground.log', dir: '/home/op/DeSCENT/2026-10-22/pulls/1402-CS9',
    report: { read: 6301, found: 6301, complete: true, boots: [{}, {}], csids: [15, 17] } };
  await p.jobEnded('pull');
  assert.equal(p.el('flash-done').hidden, false);
  // the head: the board's name, the title and the state on one line, the time small beside them
  assert.match(p.html('flash-done-body'), /<h2 id="flash-done-title"><span>CS9 · Flight log pulled<\/span><span class="mark good">complete<\/span><span class="note">\d\d:\d\d:\d\d<\/span><\/h2>/);
  // then a label and a value a line, not sentences
  assert.match(tidy(p.text('flash-done-body')), /Records 6,301 of 6,301 Boots 2 GPS time none Board ids 15 and 17 Folder \/home\/op\/DeSCENT\/2026-10-22\/pulls\/1402-CS9 Flight software put back/);
  assert.match(p.html('flash-done-body'), /<dl class="kv"><dt>Records<\/dt><dd>6,301 of 6,301<\/dd><dt>Boots<\/dt><dd>2<\/dd>/);
  assert.doesNotMatch(p.text('flash-done-body'), /dump firmware|Last pull:/);
  assert.doesNotMatch(p.html('flash-done-body'), /data-openlog/, 'the box reports; the sheet has the actions');
  await p.fire('flash-done-ok', 'click');
  p.jobs.pull = { uid: UID9, running: false, wipe: true, keepFsw: false, restored: false, pct: 100, steps: ['writing the dump firmware'] };
  await p.jobEnded('pull');
  assert.match(tidy(p.text('flash-done-body')), /CS9 · Flash cleared \d\d:\d\d:\d\d Flight software not on the board/);
  assert.match(p.html('flash-done-body'), /<p class="outcome bad">The board is on the dump firmware: write a flight image next\.<\/p>$/, 'what to act on stands last, in its colour');
  p.jobs.pull = { uid: UID9, running: false, error: 'the console printed nothing', steps: [] };
  await p.jobEnded('pull');
  assert.match(p.text('flash-done-body'), /CS9 · Pull failed/);
  assert.match(p.html('flash-done-body'), /<p class="outcome bad">the console printed nothing<\/p>/);
  // a board the list does not know is named by the id in its pull, never "The board"
  p.o.chips = [];
  await p.ticks(1);
  p.jobs.pull = { uid: 'FFFF', running: false, keepFsw: true, restored: true, log: 'x/ground.log', report: { read: 145, found: 145, complete: true, csids: [255] } };
  await p.jobEnded('pull');
  assert.match(p.text('flash-done-body'), /Board 255 · Flight log pulled/);
});

test('clear flash asks first, then runs the pull job told to wipe, and says so while it runs', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  p.o.pulled = null;
  const hub = p.hub || null;
  let h = p.sheet();
  assert.match(h, /data-wipe="1"[^>]*>Clear flash</);
  // unticked by default: nothing saved, nothing put back
  assert.match(h, /data-keepfsw="1"(?![^>]*checked)[^>]*>/);
  assert.match(p.words(), /Save the flight software that is on it and put it back/);
  assert.match(h, /Erases the log on the chip and leaves the board on the dump firmware\./);
  assert.doesNotMatch(h, /data-wipego/);
  await p.fire('screen-chipsats', 'click', { 'data-wipe': '1' });
  h = p.sheet();
  // Nothing of CS9's was ever pulled here, and the warning says so first
  assert.match(p.words(), /never pulled CS9 has never been pulled on this computer\. Clearing erases its flight log for good, and nothing is saved first\./);
  assert.doesNotMatch(h, /Last pulled/);
  assert.match(h, /data-wipego="1"[^>]*>Clear the flash</);
  assert.match(h, /data-wipecancel="1"[^>]*>Cancel</);
  await p.fire('screen-chipsats', 'click', { 'data-wipecancel': '1' });
  assert.doesNotMatch(p.sheet(), /data-wipego/);
});

test('a clear sends the checkbox, and an unticked one ends on the dump firmware in plain words', async () => {
  const boards = [{ port: '/dev/ttyUSB0', usb: '1a86:7523', state: 'waiting' }];
  const p = await chipsats({ read: FLIGHT, boards, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  await p.fire('screen-chipsats', 'click', { 'data-wipe': '1' });
  await p.fire('screen-chipsats', 'click', { 'data-wipego': '1' });
  assert.equal(p.o.pulled.keepFsw, false);
  assert.equal(p.o.pulled.more.wipe, 1);
  p.jobs.pull = { running: false, wipe: true, keepFsw: false, restored: false, pct: 100, error: null, board: '/dev/ttyUSB0', uid: UID9, steps: ['writing the dump firmware'] };
  await p.jobEnded('pull');
  assert.match(p.words(), /Flash cleared (\d\d:\d\d )?Close Flight software not on the board The board is on the dump firmware: write a flight image next\./);

  await p.fire('screen-chipsats', 'change', { 'data-keepfsw': '1' }, { type: 'checkbox', checked: true });
  assert.match(p.sheet(), /The flight software is read off and put back, settings untouched\./);
  await p.fire('screen-chipsats', 'click', { 'data-wipe': '1' });
  await p.fire('screen-chipsats', 'click', { 'data-wipego': '1' });
  assert.equal(p.o.pulled.keepFsw, true);
});

test('clearing a board pulled before says when, off the newest of its pulls, and that nothing is saved', async () => {
  const now = new Date();
  // Noon two days back on this laptop's calendar, and one a week back.
  const twoDays = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 2, 11, 49);
  const week = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 7, 9, 5);
  const logs = [
    { name: '2026-10-19/pulls/1149-board9/ground.log', bytes: 2000, when: twoDays.getTime() / 1000, kind: 'pull', uid: UID9 },
    { name: '2026-10-14/pulls/0905-board9/ground.log', bytes: 1000, when: week.getTime() / 1000, kind: 'pull', uid: UID9 },
    // Another board's newer pull is not this one's, and an old clear's image is not a pull.
    { name: '2026-10-20/pulls/1000-board12/ground.log', bytes: 1000, when: now.getTime() / 1000 - 60, kind: 'pull', uid: UID12 },
    { name: '2026-10-18/clears/1125-board9/1125-board9-002C002E.bin', bytes: 262144, when: now.getTime() / 1000 - 120, kind: 'image', uid: UID9 },
  ];
  const p = await chipsats({ read: FLIGHT, logs, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  await p.fire('screen-chipsats', 'click', { 'data-wipe': '1' });
  const h = p.sheet();
  const at = twoDays.getDate() + ' ' + 'Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec'.split(' ')[twoDays.getMonth()] +
    (twoDays.getFullYear() === now.getFullYear() ? '' : ' ' + twoDays.getFullYear()) + ', 11:49';
  assert.ok(p.words().includes('Erases the flight log on CS9 for good. Last pulled ' + at +
    ' (2 days ago); anything it logged since is lost. Nothing is saved first.'), p.words());
  assert.doesNotMatch(h, /never pulled/);
  assert.match(h, /data-wipego="1"[^>]*>Clear the flash</);
});

test('a running clear says nowhere it is saving, as it saves nothing', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })], boards: [{ port: '/dev/ttyUSB0', usb: '1a86:7523', state: 'waiting' }] });
  // An older app still sent a clear's folder; it is not shown.
  p.jobs.pull = { running: true, wipe: true, keepFsw: true, board: '/dev/ttyUSB0', seconds: 10, pct: 30, steps: [], dir: '/home/op/DeSCENT/2026-10-18/clears/1125-CS9', raw: null };
  p.jobs.heard.forEach((fn) => fn([]));
  const h = p.sheet();
  assert.match(h, /Clearing the flash on CS9, console on \/dev\/ttyUSB0/);
  assert.doesNotMatch(h, /Saving to/);
  assert.doesNotMatch(h, /clears/);
});

test('with a flight software tree the pane says it builds, and names the module before it does', async () => {
  const p = await chipsats({ health: { fswTree: true, flightImages: [] }, chips: [Object.assign({}, CS9, { module: 'hp' })], storage: KEPT() });
  assert.match(p.sheet(), /A flight software tree is here, so this builds the exact image\./);
  assert.match(p.words(), /\bBuild with\b/);
  // one press, no second one: the app's own checks are what refuse a bad write
  assert.doesNotMatch(p.sheet(), /data-buildgo|Writes the flight software/);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed.embedded, 'built');
  assert.equal(p.o.flashed.module, 'hp');
});

test('with no tree the same button patches the carried image, says so, and sends the settings', async () => {
  const health = { fswTree: false, flightImages: [{ id: 'c4fb204d8f79e725', module: 'le', bytes: 66600 }] };
  const p = await chipsats({ health, storage: KEPT() });
  let h = p.sheet();
  assert.match(h, /No flight software tree, so this patches the image this app carries\./);
  assert.match(p.words(), /\bFlash with\b/);
  assert.match(h, /data-buildembed="1"[^>]*>Patch and flash/);
  assert.doesNotMatch(h, /Build and flash/);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'csid' }, { tagName: 'INPUT', value: '9' });
  await p.fire('screen-chipsats', 'change', { 'data-build': 'sf' }, { tagName: 'SELECT', value: '12' });
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  // No module in the request: the app picks the image by the module it has for the board.
  assert.deepStrictEqual(Object.assign({}, p.o.flashed),
    Object.assign({ uid: UID9, backup: false, csid: '9', sf: 12, dbm: 14 }, BLOCK_RATES, { embedded: 'flight' }), 'what the browser remembers, with the two edits');
});

test('with no tree and no carried image for the module, the button is off and says why', async () => {
  const health = { fswTree: false, flightImages: [{ id: '6935d29957db19b5', module: 'hp', bytes: 66600 }] };
  const p = await chipsats({ health });
  assert.match(p.sheet(), /this build carries no flight image for the LE module\. Flash a file instead\./);
  assert.match(p.sheet(), /data-buildembed="1"[^>]*disabled/);
});

test('the firmware pane names the dump image and offers a file or a saved image, not a carried one', async () => {
  const p = await chipsats({ logs: [{ kind: 'image', uid: UID9, name: 'descent-image-002C002E-CS9-x.bin', when: 1791000000, bytes: 262144 }] });
  const h = p.sheet();
  assert.match(h, /Dump firmware 25118bdf/);
  assert.match(h, /This board transmits nothing until flight software is on it\./);
  assert.match(h, /Flash a file/);
  assert.match(h, /data-putback="descent-image-002C002E-CS9-x.bin"[^>]*>Put back/);
  assert.doesNotMatch(h, /Prebuilt image|Flash it|data-embedded/);
});

test('a flash that read the board at its end leaves the sheet current, and one that could not says to read', async () => {
  const p = await chipsats({ read: FLIGHT });
  p.jobs.chip = { running: false, what: 'flash', uid: UID9, steps: ['x'], error: null, read: readOf(UID9, FLIGHT.firmware, block({ sf: 12 })) };
  await p.jobEnded('chip');
  assert.match(p.sheet(), />SF12<\/span>/);
  assert.doesNotMatch(p.sheet(), /Read again\./);
  p.jobs.chip = { running: false, what: 'flash', uid: UID9, steps: ['x'], error: 'did not answer afterwards', read: null };
  await p.jobEnded('chip');
  assert.match(p.sheet(), /Changed by the flash\. Read again\./);
});

test('after a write the sheet and the draft hold what the board reads, even when it differs', async () => {
  const p = await chipsats({ read: FLIGHT });
  p.jobs.chip = { running: false, what: 'flash', uid: UID9, steps: ['x'], error: 'settings differ: board id', read: readOf(UID9, FLIGHT.firmware, block({ csid: 42, sf: 12 })) };
  await p.jobEnded('chip');
  const h = p.sheet();
  assert.doesNotMatch(h, /Read again\./);
  assert.match(h, /value="42"/, 'the draft starts from the board');
  assert.match(h, /No changes/, 'nothing is left over from the old draft');
});

test('a write that leaves no block keeps the draft: bench firmware has no settings to read', async () => {
  const p = await chipsats({ read: FLIGHT });
  const before = p.sheet().match(/value="(\d+)"/)[1];
  p.jobs.chip = { running: false, what: 'flash', uid: UID9, steps: ['x'], error: null, read: readOf(UID9, { kind: 'bench', module: 'le' }, null) };
  await p.jobEnded('chip');
  assert.match(p.sheet(), new RegExp('value="' + before + '"'));
});

test('a probe is read once when it is plugged in, and again only when it comes back', async () => {
  const p = await chipsats({});
  assert.equal(p.o.reads, 1, 'read with no press');
  await p.ticks(5);
  assert.equal(p.o.reads, 1);
  p.o.probes = [];
  await p.ticks(1);
  p.o.probes = [PROBE];
  await p.ticks(1);
  assert.equal(p.o.reads, 2);
  // The button and its warning are still there for a read that is asked for.
  assert.match(p.sheet(), /data-read="002A00"/);
  assert.match(p.sheet(), /Reading can reset the board\./);
});

test('nothing touches a board while another screen is up', async () => {
  const p = await chipsats({ screen: 'tbeams' });
  await p.ticks(6);
  assert.deepStrictEqual([p.o.reads, p.o.peeks], [0, 0]);
});

test('a probe with no board on it is tried once and not again', async () => {
  const p = await chipsats({ chips: [], readFails: true, under: null });
  await p.ticks(6);
  assert.equal(p.o.reads, 1);
  assert.match(p.sheet(), /Could not read it: no board answered/);
});

test('a different board under the same probe is noticed and read, once', async () => {
  const p = await chipsats({});
  await p.ticks(8);
  assert.deepStrictEqual([p.o.reads, p.o.peeks], [1, 4], 'the same board is asked its id and not read again');
  p.o.under = null;   // lifted off the clip: nothing answers
  await p.ticks(4);
  assert.equal(p.o.reads, 1);
  p.o.under = UID12;
  p.o.read = readOf(UID12, { kind: 'flight', module: 'le', id: 'c4fb204d8f79e725' }, block({ csid: 12, sf: 9 }));
  p.o.chips = [CS9, CS12];
  await p.ticks(2);
  assert.equal(p.o.reads, 2);
  assert.match(p.sheet(), new RegExp('data-name="' + UID12 + '"'), 'the new board has the sheet');
  await p.ticks(6);
  assert.equal(p.o.reads, 2);
});

// User, 2026-10-31: a board swapped under the clip "stays on the old IDs board, and the same
// stlink errors show up there under the old chipsat ID which is very confusing"
test('a board lifted off the clip: the third empty id check takes the sheet to the probe\'s own view, and a failed read lands there, never under the old board', async () => {
  const p = await chipsats({});
  await p.ticks(2);
  assert.match(p.sheet(), new RegExp('data-name="' + UID9 + '"'), 'CS9 has the sheet');
  p.o.under = null;   // lifted off the clip: nothing answers
  await p.ticks(4);   // two checks
  assert.match(p.sheet(), new RegExp('data-name="' + UID9 + '"'), 'two empty checks say nothing');
  await p.ticks(2);   // the third
  assert.doesNotMatch(p.sheet(), /data-name=/, 'no board is on the sheet');
  assert.match(p.words(), /^The board on the ST-Link not read STLink V3 002A00 Read the board .*Nothing has answered on the ST-Link since \d\d:\d\d\./);
  assert.equal(p.o.reads, 1, 'going home attaches to nothing');
  // a board that answers the check and then fails its read: the error is the probe's, not CS9's
  p.o.under = UID12; p.o.readFails = true;
  await p.ticks(2);
  assert.equal(p.o.reads, 2);
  const w = p.words();
  assert.match(w, /^The board on the ST-Link not read .*Could not read it: no board answered/);
  assert.doesNotMatch(p.sheet(), /CS9|data-name=/);
  assert.doesNotMatch(w, /Nothing has answered/, 'the check did get an answer');
  // the board reads: its sheet
  p.o.readFails = false; p.o.tried = null;
  p.o.read = readOf(UID12, { kind: 'flight', module: 'le', id: 'c4fb204d8f79e725' }, block({ csid: 12 }));
  p.o.chips = [CS9, CS12];
  await p.fire('screen-chipsats', 'click', { 'data-read': PROBE.serial });
  assert.match(p.sheet(), new RegExp('data-name="' + UID12 + '"'));
  assert.doesNotMatch(p.sheet(), /Could not read it/);
});

test('a different board answering under the clip leaves the old sheet before its read, so a failed read is not CS9\'s', async () => {
  const p = await chipsats({});
  await p.ticks(2);
  p.o.under = UID12; p.o.readFails = true;
  await p.ticks(2);
  assert.equal(p.o.reads, 2);
  assert.doesNotMatch(p.sheet(), new RegExp('data-name="' + UID9 + '"'));
  assert.match(p.words(), /^The board on the ST-Link not read .*Could not read it: no board answered/);
  assert.doesNotMatch(p.list(), /CS9[^<]*<\/b>[\s\S]*on the ST-Link/, 'CS9 is not said to be on the probe');
});

test('nothing jumps while a job runs', async () => {
  const p = await chipsats({});
  await p.ticks(2);
  p.jobs.pull = { running: true, uid: UID9, board: '/dev/ttyUSB0', pct: 10, steps: [] };
  p.o.under = null;
  await p.ticks(10);
  assert.match(p.sheet(), new RegExp('data-name="' + UID9 + '"'));
});

test('a board that answers the id check and then fails its read is left for the button', async () => {
  const p = await chipsats({});
  p.o.under = UID12;
  p.o.readFails = true;
  await p.ticks(10);
  assert.equal(p.o.reads, 2, 'the first read and one try');
});

test('the board is not asked anything while something is waiting to be confirmed', async () => {
  const p = await chipsats({});
  await p.fire('screen-chipsats', 'click', { 'data-benchwrite': '1' });
  assert.match(p.sheet(), /data-benchgo="1"/);
  const asked = p.o.peeks;
  p.o.under = UID12;
  await p.ticks(6);
  assert.deepStrictEqual([p.o.peeks - asked, p.o.reads], [0, 1]);
});

test('forgetting a board asks first, drops it from the list and does not read it straight back', async () => {
  const p = await chipsats({ chips: [CS9, CS12] });
  assert.match(p.sheet(), /data-forget="1"[^>]*>Forget this board/);
  await p.fire('screen-chipsats', 'click', { 'data-forget': '1' });
  assert.equal(p.o.forgot, undefined);
  assert.match(p.words(), /Forget CS9\? .*The board is not touched/);
  await p.fire('screen-chipsats', 'click', { 'data-forgetgo': '1' });
  assert.equal(p.o.forgot, UID9);
  assert.doesNotMatch(p.list(), /CS9/);
  // It is still under the clip. It shows as a probe that has not been read.
  await p.ticks(8);
  assert.equal(p.o.reads, 1);
  assert.match(p.list(), /not read/);
  // Read on purpose, it is a board nobody has named or given a module.
  p.o.chips = p.o.chips.concat([{ uid: UID9, name: '', module: null, lastSeen: 3, firmware: 'dump' }]);
  await p.fire('screen-chipsats', 'click', { 'data-read': PROBE.serial });
  assert.equal(p.o.reads, 2);
  assert.match(p.list(), /Unnamed board …002E/);
  assert.match(p.list(), /module never set/);
});

test('cancelling a forget forgets nothing', async () => {
  const p = await chipsats({ probes: [] });
  await p.fire('screen-chipsats', 'click', { 'data-forget': '1' });
  await p.fire('screen-chipsats', 'click', { 'data-forgetcancel': '1' });
  assert.doesNotMatch(p.sheet(), /data-forgetgo/);
  assert.equal(p.o.forgot, undefined);
});

test('a running pull says where it is saving, can be stopped, and a wrong console can be pulled anyway', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })], boards: [{ port: '/dev/ttyUSB0', usb: '1a86:7523', state: 'waiting' }] });
  const repaint = () => p.jobs.heard.forEach((fn) => fn([]));
  const dir = '/home/op/DeSCENT/2026-10-18/pulls/1432-CS9';
  p.jobs.pull = { running: true, board: '/dev/ttyUSB0', seconds: 30, pct: 45, steps: [], dir, raw: null };
  repaint();
  let h = p.sheet();
  assert.match(p.words(), /Saving to \/home\/op\/DeSCENT\/2026-10-18\/pulls\/1432-CS9\b/);
  assert.match(h, /data-pullstop="1"[^>]*>Stop</);
  p.jobs.pull.raw = '2026-10-18/pulls/1432-CS9/dump.txt';
  repaint();
  assert.match(p.words(), /Saving to \/home\/op\/DeSCENT\/2026-10-18\/pulls\/1432-CS9\/dump\.txt/);
  await p.fire('screen-chipsats', 'click', { 'data-pullstop': '1' });
  assert.equal(p.o.stops, 1);
  p.jobs.pull.stopping = true;
  repaint();
  assert.match(p.sheet(), /data-pullstop="1"[^>]*disabled[^>]*>Stopping…/);

  p.jobs.pull = { running: false, board: '/dev/ttyUSB0', steps: ['stop asked for'], keepFsw: true, restored: true, error: 'stopped by the operator', dir };
  repaint();
  h = p.sheet();
  assert.match(p.words(), /Pull stopped .*stopped by the operator/);
  assert.doesNotMatch(h, /Pull failed/);

  p.jobs.pull = { running: false, board: '/dev/ttyUSB0', steps: [], keepFsw: true, restored: false,
    error: "/dev/ttyUSB0 is not this board's console: nothing on it said it had started when the board was reset. Nothing was written." };
  repaint();
  h = p.sheet();
  assert.match(h, /data-anyconsole="1"[^>]*>Pull anyway</);
  await p.fire('screen-chipsats', 'click', { 'data-anyconsole': '1' });
  assert.equal(p.o.pulled.port, '/dev/ttyUSB0');
  assert.equal(p.o.pulled.more.anyConsole, 1);
});

test('a board shows its last saved flight software to put back and every log pulled off it', async () => {
  const logs = [
    { name: '2026-10-18/pulls/1432-board9/ground.log', bytes: 2000000, when: 1792348489, kind: 'pull', uid: UID9 },
    { name: '2026-10-17/images/1454-board9-002C002E.bin', bytes: 262144, when: 1792263276, kind: 'image', uid: UID9 },
    { name: '2026-10-17/pulls/1124-board9/ground.log', bytes: 1000, when: 1792200000, kind: 'pull', uid: UID9 },
    { name: '2026-10-17/pulls/1509-board12/ground.log', bytes: 1000, when: 1792100000, kind: 'pull', uid: UID12 },
  ];
  const p = await chipsats({ read: FLIGHT, logs, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  await p.fire('screen-chipsats', 'click', { 'data-allpulls': '1' });
  const h = p.sheet();
  assert.match(h, /Last saved flight software/);
  assert.match(h, /data-putback="2026-10-17\/images\/1454-board9-002C002E.bin"[^>]*>Put back last saved flight software</);
  assert.match(h, /2 pulls/);
  assert.match(h, /data-openlog="2026-10-18\/pulls\/1432-board9\/ground.log"/);
  assert.match(h, /data-openlog="2026-10-17\/pulls\/1124-board9\/ground.log"/);
  assert.doesNotMatch(h, /1509-board12/, "another board's pull is not this one's");
});

// config.h changed, LE flight built, dump not built yet, HP bench failed
const WORDS_LE = 'flight LE built here 14:02 from abc1234 + config.h changed';
const part = (name, label, kind, module, more) => Object.assign({ part: name, label, kind, module, built: null, failed: null, carried: 'c4fb204d8f79e725', same: null }, more || {});
const FSW = (more) => Object.assign({
  ok: true, tree: '/home/op/DeSCENT-repo/fsw',
  source: { hash: 'aa', commit: 'abc1234def', dirty: true, changed: ['V2_6_X_Code/src/config.h'], words: 'from abc1234 + config.h changed' },
  parts: [
    part('flight-le', 'flight LE', 'flight', 'le', { built: { part: 'flight-le', words: WORDS_LE, id: '0a1b2c3d4e5f6071' }, same: false }),
    part('flight-hp', 'flight HP', 'flight', 'hp'),
    part('flash_dump', 'dump firmware', 'dump', null),
    part('bench-le', 'sensor bench LE', 'bench', 'le'),
    part('bench-hp', 'sensor bench HP', 'bench', 'hp', { failed: 'the flight software did not build.\nsensor_bench.ino:40:3: error: expected \';\' before \'}\' token' }),
    part('radio.json', 'radio table', 'radio', null),
  ],
}, more || {});
const HEALTH_ON = { fswTree: true, flightImages: [{ id: 'c4fb204d8f79e725', module: 'le', bytes: 63396 }] };

test('there is no flight software pane, switch or Rebuild button, tree or not', async () => {
  for (const fsw of [FSW(), { ok: true, tree: null, source: null, parts: [] }, undefined]) {
    const p = await chipsats({ fsw, health: HEALTH_ON });
    const h = p.sheet();
    assert.doesNotMatch(h, /Flight software on this computer/);
    assert.doesNotMatch(h, /data-fswon|data-fswrebuild|Rebuild from/);
    assert.doesNotMatch(h, /<pre class="raw">/, 'compiler output is shown where a write was asked for, not up front');
  }
});

test('a board running an image built here is named by where it came from, and so is the image about to be written', async () => {
  const read = readOf(UID9, { kind: 'flight', module: 'le', id: '0a1b2c3d4e5f6071', built: { part: 'flight-le', words: WORDS_LE } }, block());
  const p = await chipsats({ read, fsw: FSW(), health: HEALTH_ON, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  assert.ok(p.words().includes('Flight LE built here 14:02 from abc1234 + config.h changed'), p.words());
  assert.match(p.words(), /The image is the flight LE built here 14:02 from abc1234 \+ config\.h changed\./);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed.embedded, 'built');
});

test('an HP image not built yet says it is built from the tree first', async () => {
  const p = await chipsats({ fsw: FSW(), health: HEALTH_ON, chips: [Object.assign({}, CS9, { module: 'hp' })] });
  assert.match(p.sheet(), /It is built from abc1234 \+ config.h changed first, about 13 seconds./);
});

test('a write whose build fails shows the compiler\'s own words on the sheet it was asked from', async () => {
  const p = await chipsats({ read: FLIGHT, fsw: FSW(), health: HEALTH_ON, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  p.jobs.chip = { what: 'flash', uid: UID9, running: false, steps: ['building the flight LE'],
    error: 'the flight LE from this computer\'s flight software did not build, so nothing was written. config.h:60:26: error: static assertion failed' };
  await p.jobEnded('chip');
  assert.match(p.words(), /Flash failed .*the flight LE from this computer's flight software did not build, so nothing was written\. config\.h:60:26: error: static assertion failed/);
});

test('a pull says which dump firmware read it and which flight software was on the board', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  p.jobs.pull = {
    running: false, board: '/dev/ttyUSB0', steps: ['done'], keepFsw: true, restored: true, error: null,
    dir: '/home/op/DeSCENT/2026-10-22/pulls/1402-CS9', log: '2026-10-22/pulls/1402-CS9/ground.log',
    report: { read: 2, found: 2, complete: true, firmware: { dump: { words: 'dump firmware built here 14:01 from abc1234 + FlashLog.cpp changed' }, onBoard: { kind: 'flight', module: 'le', built: { words: WORDS_LE } } } },
  };
  p.jobs.heard.forEach((fn) => fn([]));
  assert.match(p.words(), /Flight software put back Read with the dump firmware built here 14:01 from abc1234 \+ FlashLog.cpp changed On the board was the flight LE built here 14:02 from abc1234 \+ config.h changed/);
});

test('an image chosen on Data with no board listed says it is waiting, and can be dropped', async () => {
  const p = await chipsats({ chips: [], probes: [] });
  p.shell.writeImage({ name: '2026-10-17/pulls/1432-board9/1432-board9-20393256.bin', when: 1790000000, uid: '20393256' });
  await p.settle();
  assert.match(p.sheet(), /Chosen to write: the image saved on .*Read the board first\./);
  await p.fire('screen-chipsats', 'click', { 'data-flashcancel': '1' });
  assert.doesNotMatch(p.sheet(), /Chosen to write/);
});

test('a Patch and flash that cannot go says why beside the button', async () => {
  const p = await chipsats({ probes: [] });
  assert.match(p.sheet(), /data-buildembed="1"[^>]*disabled/);
  // said right under the button, the first thing after its label
  assert.match(p.words(), /and flash(?: CS9)? Put the board on the ST-Link and press Read the board\./);
});

const FLIGHT_CS9 = Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' });
const CONSOLE_PORT = [{ port: '/dev/ttyUSB0', usb: '1a86:7523', state: 'waiting' }];
const pickRow = (uid) => ({ closest: (sel) => (sel === '[data-pick]' ? { getAttribute: () => uid } : null) });

test('the sheet reads as three numbered steps, then the flight logs and the other images', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9] });
  const w = p.words();
  const at = (re) => w.search(re);
  const order = [/Identity and radio/, /\bPhases\b/, /\bWrite\b/, /Flight logs/, /Other images/].map(at);
  assert.ok(order.every((i) => i >= 0), String(order));
  assert.deepStrictEqual(order.slice().sort((a, b) => a - b), order);
  assert.deepStrictEqual((p.sheet().match(/class="stepno">\d/g) || []).map((s) => s.slice(-1)), ['1', '2', '3']);
});

test('settings changed from the board are rows in Write, the board\'s value struck beside the new one', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9] });
  assert.match(p.words(), /No changes from the board/);
  await p.fire('screen-chipsats', 'change', { 'data-build': 'sf' }, { tagName: 'SELECT', value: '9' });
  assert.match(p.words(), /1 change from the board Spreading factor SF7 → SF9/);
  assert.match(p.words(), /On the board: SF7 changed/);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'p1l' }, { tagName: 'INPUT', value: '0.1' });
  assert.match(p.words(), /2 changes from the board Spreading factor SF7 → SF9 Air, always, log 0\.05 s → 0\.1 s/);
  // what it writes in full, by the way it writes
  assert.match(p.words(), /Build with board 9, LE, SF9, 14 dBm\. The rates are the Phases tables\./);
});

test('the write button names the board, and a running flash takes its place', async () => {
  let p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9] });
  assert.match(p.sheet(), /data-buildembed="1"(?![^>]*disabled)[^>]*>Build and flash CS9</);
  assert.match(p.words(), /A flight software tree is here, so this builds the exact image\./);
  assert.match(p.sheet(), /data-backup="1"/);
  p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], health: { fswTree: false, flightImages: [{ id: 'c4fb204d8f79e725', module: 'le', bytes: 66600 }] } });
  assert.match(p.sheet(), /data-buildembed="1"[^>]*>Patch and flash CS9</);
  p.jobs.chip = { running: true, what: 'flash', uid: UID9, steps: ['patching the image'], pct: 40, seconds: 3 };
  p.jobs.heard.forEach((fn) => fn([]));
  assert.match(p.sheet(), /data-buildembed="1"[^>]*disabled[^>]*>Patch and flash CS9</, 'off while it runs');
  // one card under the head says what is happening to the board; nothing else repeats it
  assert.match(p.words(), /on the ST-Link .* Flashing CS9 .* Identity and radio/);
  assert.equal((p.sheet().match(/class="job"/g) || []).length, 1);
  assert.doesNotMatch(p.sheet(), /data-closejob/, 'a running job has Stop, not Close');
});

// User, 2026-10-31: "I appreciate the single what's happening to the board / process panel, but
// I need to be able to close it"
test('a finished result can be closed, stays closed across repaints and a reload, and the next job opens the card again', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], boards: CONSOLE_PORT });
  const pulled = (log) => ({ running: false, board: '/dev/ttyUSB0', steps: ['done'], keepFsw: true, restored: true, error: null, uid: UID9, seconds: 70,
    log, dir: '/home/op/DeSCENT/x', report: { read: 10, found: 10, complete: true } });
  p.jobs.pull = pulled('2026-10-31/pulls/2200-board9/ground.log');
  p.jobs.heard.forEach((fn) => fn([]));
  let h = p.sheet();
  assert.match(h, /class="pane now"/);
  const key = h.match(/data-closejob="([^"]+)"/);
  assert.ok(key, 'a Close in the card');
  await p.fire('screen-chipsats', 'click', { 'data-closejob': key[1] });
  assert.doesNotMatch(p.sheet(), /class="pane now"/);
  p.jobs.heard.forEach((fn) => fn([]));
  assert.doesNotMatch(p.sheet(), /class="pane now"/, 'a repaint does not bring it back');
  const again = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], boards: CONSOLE_PORT, storage: p.storage });
  again.jobs.pull = pulled('2026-10-31/pulls/2200-board9/ground.log');
  again.jobs.heard.forEach((fn) => fn([]));
  assert.doesNotMatch(again.sheet(), /class="pane now"/, 'nor a reload');
  again.jobs.pull = pulled('2026-10-31/pulls/2230-board9/ground.log');
  again.jobs.heard.forEach((fn) => fn([]));
  assert.match(again.sheet(), /class="pane now"[\s\S]*data-closejob=/, 'the next pull is shown, with its own Close');
  again.jobs.pull = Object.assign(pulled('2026-10-31/pulls/2230-board9/ground.log'), { running: true, pct: 10 });
  again.jobs.heard.forEach((fn) => fn([]));
  assert.match(again.sheet(), /class="pane now"/);
  assert.doesNotMatch(again.sheet(), /data-closejob/);
});

test('the flight logs row counts the pulls, opens the latest, and lists them all only when asked', async () => {
  const now = Date.now() / 1000;
  const logs = [0, 1, 2].map((i) => ({ name: '2026-10-2' + (8 - i) + '/pulls/1400-board9/ground.log', bytes: 1000, when: now - 86400 * i - 60, kind: 'pull', uid: UID9 }));
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], logs, boards: CONSOLE_PORT });
  let h = p.sheet();
  assert.match(p.words(), /Flight logs 3 pulls · latest /);
  assert.match(h, /data-openlog="2026-10-28\/pulls\/1400-board9\/ground.log"[^>]*>Open latest</);
  assert.match(h, /data-allpulls="1"[^>]*>All 3</);
  assert.match(h, /data-pull="1"[^>]*>Pull the flight log</);
  assert.doesNotMatch(h, /2026-10-26\/pulls/, 'folded by default');
  await p.fire('screen-chipsats', 'click', { 'data-allpulls': '1' });
  h = p.sheet();
  assert.match(h, /data-allpulls="1" aria-expanded="true"/);
  for (const l of logs) {
    assert.ok(h.includes('data-openlog="' + l.name + '"'), l.name);
    assert.ok(h.includes('data-csv="' + l.name + '"'), l.name);
  }
  await p.fire('screen-chipsats', 'click', { 'data-allpulls': '1' });
  assert.doesNotMatch(p.sheet(), /2026-10-26\/pulls/);
});

test('Clear flash stays in the flight logs row, with its confirm naming the last pull and the keep box unticked', async () => {
  const now = new Date();
  const logs = [{ name: '2026-10-19/pulls/1149-board9/ground.log', bytes: 2000, when: now.getTime() / 1000 - 3 * 86400, kind: 'pull', uid: UID9 }];
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], logs, boards: CONSOLE_PORT });
  assert.match(p.sheet(), /data-keepfsw="1"(?![^>]*checked)[^>]*>/);
  await p.fire('screen-chipsats', 'click', { 'data-wipe': '1' });
  const w = p.words();
  const at = (re) => w.search(re);
  assert.ok(at(/Flight logs/) < at(/Clear flash/) && at(/Clear flash/) < at(/Erases the flight log on CS9 for good\. Last pulled/));
  assert.ok(at(/Erases the flight log on CS9/) < at(/Other images/), 'the confirm is in the row, not elsewhere');
  assert.match(p.sheet(), /data-wipego="1"[^>]*>Clear the flash</);
});

test('other images are on the sheet beside the flight logs, nothing to open first', async () => {
  const logs = [{ kind: 'image', uid: UID9, name: 'img-CS9.bin', when: 1791000000, bytes: 262144 }];
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], logs });
  const h = p.sheet();
  assert.doesNotMatch(h, /data-images=|>Show</);
  assert.match(h, /data-putback="img-CS9.bin"[^>]*>Put back last saved flight software</);
  assert.match(h, /data-file="1"/);
  assert.match(h, /data-benchwrite="1"[^>]*>Write the sensor bench</);
  await p.fire('screen-chipsats', 'click', { 'data-benchwrite': '1' });
  assert.match(p.words(), /Writes the LE sensor bench over whatever is on CS9/);
  assert.match(p.sheet(), /data-benchgo="1"/);
});

test('a probe whose board does not answer: a remembered board still gets its steps, the write off with why, pull and clear on the sheet', async () => {
  const p = await chipsats({ chips: [FLIGHT_CS9], readFails: true, under: null });
  assert.match(p.words(), /Could not read it: no board answered/);
  await p.fire('screen-chipsats', 'click', {}, pickRow(UID9));
  const h = p.sheet();
  const w = p.words();
  assert.match(w, /Identity and radio.*\bPhases\b.*\bWrite\b.*Flight logs/);
  assert.match(h, /data-buildembed="1"[^>]*disabled[^>]*>Build and flash CS9</);
  assert.match(w, /Build and flash CS9 Put the board on the ST-Link and press Read the board\./);
  assert.match(h, /data-pull="1"[^>]*>Pull the flight log</);
  assert.match(h, /data-wipe="1"[^>]*>Clear flash</);
  await p.fire('screen-chipsats', 'click', { 'data-wipe': '1' });
  assert.match(p.words(), /never pulled CS9 has never been pulled on this computer/);
  assert.match(p.sheet(), /data-wipego="1"/);
});

test('the list has the board on the ST-Link in its own card and the rest under Remembered', async () => {
  const p = await chipsats({ chips: [CS9, Object.assign({}, CS12, { csid: 12 })] });
  const w = tidy(p.text('cs-list'));
  assert.match(w, /^On the ST-Link STLink V3 002A00 CS9 LE dump firmware · read \d\d:\d\d Remembered · 1 board CS12 LE Board 12 · last on the probe /);
  assert.match(w, /Reading can reset the board\./);
  const none = await chipsats({ chips: [CS9], probes: [] });
  assert.match(tidy(none.text('cs-list')), /^On the ST-Link Nothing on the probe\. Remembered · 1 board CS9/);
});

test('a probe nobody has read yet has Read the board in its card', async () => {
  const p = await chipsats({ chips: [], readFails: true, under: null });
  assert.match(p.list(), /data-read="002A00"[^>]*>Read the board</);
  assert.match(tidy(p.text('cs-list')), /The board on the ST-Link not read · STLink V3 002A00/);
});

test('a pull the app calls complete says so', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9] });
  p.jobs.pull = { running: false, board: '/dev/ttyUSB0', steps: ['done'], keepFsw: true, restored: true, error: null, uid: UID9,
    log: '2026-10-22/pulls/1402-CS9/ground.log', report: { read: 12480, found: 12480, complete: true, boots: [{ timeSource: 'anchor' }] } };
  p.jobs.heard.forEach((fn) => fn([]));
  assert.match(p.words(), /Flight log pulled complete .*Records 12,480 of 12,480 Boots 1 GPS time 1 boot/);
});

// The user's 2026-10-31 example: four lines with space between each, where one block will do
test('a pull\'s result is a labelled list in the card under the head, said once, with what to act on last', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], boards: CONSOLE_PORT });
  p.jobs.pull = { running: false, board: '/dev/ttyUSB0', steps: ['done'], keepFsw: false, restored: false, error: null, uid: UID9,
    log: '2026-10-31/pulls/2121-board17/ground.log', dir: '/home/cats/DeSCENT/2026-10-31/pulls/2121-board17', raw: '2026-10-31/pulls/2121-board17/dump.txt',
    report: { read: 6301, found: 6301, complete: true, boots: new Array(27).fill({}), csids: [15, 16, 17],
      firmware: { dump: { words: 'dump firmware built here 21:21 from 09589b7 + Settings.cpp, Settings.h and 11 more changed' } } } };
  p.jobs.heard.forEach((fn) => fn([]));
  const h = p.sheet();
  const w = p.words();
  assert.equal((w.match(/Records 6,301/g) || []).length, 1);
  assert.ok(w.search(/Flight log pulled/) < w.search(/Identity and radio/), 'under the head, before the steps');
  assert.match(w, /Flight log pulled complete Close Records 6,301 of 6,301 Boots 27 GPS time none Board ids 15, 16 and 17 Folder \/home\/cats\/DeSCENT\/2026-10-31\/pulls\/2121-board17 Read with the dump firmware built here 21:21 from 09589b7 \+ Settings.cpp, Settings.h and 11 more changed The records carry board ids 15, 16 and 17, not 9\. The board is still running the dump firmware, not flight software\. Open Save log Save CSV Save raw/);
  assert.equal((h.match(/<dl class="kv">/g) || []).length, 1, 'one list');
  assert.doesNotMatch(h, /<p class="(outcome|step) inpane">/);
  assert.match(h, /data-openlog="2026-10-31\/pulls\/2121-board17\/ground.log"[^>]*>Open</);
  assert.match(h, /download="[^"]*dump.txt"/);
});

test('the flight logs card reads top down: what is there, the console, the keep box, then pull and clear with what each does', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], boards: CONSOLE_PORT });
  const w = p.words();
  const at = (re) => { const i = w.search(re); assert.ok(i >= 0, String(re)); return i; };
  const order = [/Flight logs/, /never pulled/, /Console port/, /Flight software Save the flight software that is on it and put it back/,
    /Pull the flight log Flashes the dump firmware\. Takes minutes\./, /Clear flash Erases the log on the chip and leaves the board on the dump firmware\./].map(at);
  assert.deepStrictEqual(order.slice().sort((a, b) => a - b), order, String(order));
  assert.ok(at(/Clear flash/) < at(/Other images/));
});

test('a running pull is one job card under the head, and the flight logs card keeps still', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [FLIGHT_CS9], boards: CONSOLE_PORT });
  p.jobs.pull = { running: true, board: '/dev/ttyUSB0', seconds: 30, pct: 45, steps: [], dir: '/home/op/DeSCENT/x', raw: null, uid: UID9 };
  p.jobs.heard.forEach((fn) => fn([]));
  const w = p.words();
  assert.equal((p.sheet().match(/class="job"/g) || []).length, 1);
  assert.equal((w.match(/Pulling the flight log/g) || []).length, 1);
  assert.ok(w.search(/Pulling the flight log/) < w.search(/Identity and radio/));
  assert.match(w, /Flight logs .* Console port .* Pull the flight log/);
  assert.match(p.sheet(), /data-pull="1"[^>]*disabled/);
  assert.match(p.sheet(), /data-buildembed="1"[^>]*disabled/);
});
