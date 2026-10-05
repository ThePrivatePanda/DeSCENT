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
const block = (o) => Object.assign({ layoutVersion: 1, csid: 255, module: 'le', sf: 7, dbm: 14, setBy: 'build', txIntervalMs: 1000, reservedClear: true }, o);
const readOf = (uid, firmware, settings) => ({ ok: true, uid, flashBytes: 262144, firmware, settings, settingsError: null, probe: PROBE });
const DUMP = readOf(UID9, { kind: 'dump', current: true, id: '25118bdfafab456c' }, null);
const FLIGHT = readOf(UID9, { kind: 'flight', module: 'le', id: 'c4fb204d8f79e725' }, block());

// The screen with a fake app behind it. o.read is what a read answers, o.under what
// answers the id check (the board that was read, unless a test says otherwise).
async function chipsats(o) {
  o = Object.assign({ screen: 'chipsats', chips: [CS9], probes: [PROBE], read: DUMP, logs: [], reads: 0, peeks: 0 }, o);
  const hub = {
    native: true, ports: [], boards: [],
    chips: async () => o.chips, chipProbes: async () => o.probes, logs: async () => o.logs, radio: async () => null,
    health: async () => o.health || null,
    chipRead: async () => { o.reads++; if (o.readFails) throw new Error('no board answered'); return o.read; },
    chipPeek: async () => { o.peeks++; return o.under === undefined ? o.read.uid : o.under; },
    chipFlash: async (q) => { o.flashed = q; },
    chipForget: async (uid) => { o.forgot = uid; o.chips = o.chips.filter((c) => c.uid !== uid); },
  };
  const p = page(['boardmodel.js', 'datamodel.js', 'chipsats.js'], hub, o);
  await p.settle();
  p.parts.chipsats.show();
  await p.settle();
  return Object.assign(p, { o, sheet: () => p.html('cs-sheet'), list: () => p.html('cs-list') });
}
const struck = (html) => (html.match(/<span class="val was">[^<]*<\/span>/g) || []).map((s) => s.replace(/<[^>]*>/g, ''));

test('a value on the board is struck through only when the build would replace it', async () => {
  const p = await chipsats({ read: FLIGHT, chips: [Object.assign({}, CS9, { firmware: 'flight', firmwareModule: 'le' })] });
  assert.deepStrictEqual(struck(p.sheet()), []);
  await p.fire('screen-chipsats', 'change', { 'data-build': 'sf' }, { tagName: 'SELECT', value: '12' });
  assert.deepStrictEqual(struck(p.sheet()), ['SF7']);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'interval' }, { tagName: 'INPUT', value: '500' });
  assert.deepStrictEqual(struck(p.sheet()), ['SF7', '1 s']);
});

test('a board with no settings block still gets the table and a Build and flash that works', async () => {
  const p = await chipsats({});
  const h = p.sheet();
  assert.match(h, /Build with/);
  assert.match(h, /The dump firmware has no radio settings\./);
  assert.match(h, /data-buildembed="1">/, 'the button is not switched off');
  await p.fire('screen-chipsats', 'input', { 'data-build': 'csid' }, { tagName: 'INPUT', value: '9' });
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.equal(p.o.flashed, undefined, 'the first press only asks');
  await p.fire('screen-chipsats', 'click', { 'data-buildgo': '1' });
  assert.deepStrictEqual(Object.assign({}, p.o.flashed),
    { uid: UID9, backup: true, csid: '9', sf: 7, dbm: 14, interval: '1000', embedded: 'built', module: 'le' });
});

test('with a flight software tree the pane says it builds, and names the module before it does', async () => {
  const p = await chipsats({ health: { fswTree: true, flightImages: [] }, chips: [Object.assign({}, CS9, { module: 'hp' })] });
  assert.match(p.sheet(), /A flight software tree is here, so this builds the exact image\./);
  assert.match(p.sheet(), /<div class="h">Build with<\/div>/);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  assert.match(p.sheet(), /Builds the flight software for the <b>HP module<\/b> with these settings and writes it to <b>CS9<\/b>/);
  assert.match(p.sheet(), /data-buildgo="1" class="primary">Build and flash</);
});

test('with no tree the same button patches the carried image, says so, and sends the settings', async () => {
  const health = { fswTree: false, flightImages: [{ id: 'c4fb204d8f79e725', module: 'le', bytes: 66600 }] };
  const p = await chipsats({ health });
  let h = p.sheet();
  assert.match(h, /No flight software tree, so this patches the image this app carries\./);
  assert.match(h, /<div class="h">Flash with<\/div>/);
  assert.match(h, /data-buildembed="1">Patch and flash</);
  assert.doesNotMatch(h, /Build and flash/);
  await p.fire('screen-chipsats', 'input', { 'data-build': 'csid' }, { tagName: 'INPUT', value: '9' });
  await p.fire('screen-chipsats', 'change', { 'data-build': 'sf' }, { tagName: 'SELECT', value: '12' });
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  h = p.sheet();
  assert.match(h, /Writes the flight image this app carries for the <b>LE module<\/b> to <b>CS9<\/b>, with these settings in it\./);
  assert.equal(p.o.flashed, undefined, 'the first press only asks');
  await p.fire('screen-chipsats', 'click', { 'data-buildgo': '1' });
  // No module in the request: the app picks the image by the module it has for the board.
  assert.deepStrictEqual(Object.assign({}, p.o.flashed),
    { uid: UID9, backup: true, csid: '9', sf: 12, dbm: 14, interval: '1000', embedded: 'flight' });
});

test('with no tree and no carried image for the module, the button is off and says why', async () => {
  const health = { fswTree: false, flightImages: [{ id: '6935d29957db19b5', module: 'hp', bytes: 66600 }] };
  const p = await chipsats({ health });
  assert.match(p.sheet(), /this build carries no flight image for the LE module\. Flash a file instead\./);
  assert.match(p.sheet(), /data-buildembed="1" disabled>/);
});

test('the firmware pane names the dump image and offers a file or a saved image, not a carried one', async () => {
  const p = await chipsats({ logs: [{ kind: 'image', uid: UID9, name: 'descent-image-002C002E-CS9-x.bin', when: 1791000000, bytes: 262144 }] });
  const h = p.sheet();
  assert.match(h, /Dump firmware 25118bdf/);
  assert.match(h, /This board transmits nothing until flight software is on it\./);
  assert.match(h, /Flash a file/);
  assert.match(h, /data-putback="descent-image-002C002E-CS9-x.bin">Put back/);
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

test('a board that answers the id check and then fails its read is left for the button', async () => {
  const p = await chipsats({});
  p.o.under = UID12;
  p.o.readFails = true;
  await p.ticks(10);
  assert.equal(p.o.reads, 2, 'the first read and one try');
});

test('the board is not asked anything while something is waiting to be confirmed', async () => {
  const p = await chipsats({});
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1' });
  const asked = p.o.peeks;
  p.o.under = UID12;
  await p.ticks(6);
  assert.deepStrictEqual([p.o.peeks - asked, p.o.reads], [0, 1]);
});

test('forgetting a board asks first, drops it from the list and does not read it straight back', async () => {
  const p = await chipsats({ chips: [CS9, CS12] });
  assert.match(p.sheet(), /data-forget="1">Forget this board/);
  await p.fire('screen-chipsats', 'click', { 'data-forget': '1' });
  assert.equal(p.o.forgot, undefined);
  assert.match(p.sheet(), /Forget <b>CS9<\/b>\? .*The board is not touched/);
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
