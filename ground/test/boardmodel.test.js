const test = require('node:test');
const assert = require('node:assert');
const B = require('../web/js/boardmodel.js');

const UID9 = '003400415650501320383352';
const UID12 = '002a00195650501320383311';

test('a UID is read in groups of four', () => {
  assert.equal(B.uidWords(UID9), '0034 0041 5650 5013 2038 3352');
  assert.equal(B.uidWords(''), '');
  assert.equal(B.chipName({ uid: UID9, name: 'Board 9' }), 'Board 9');
  assert.equal(B.chipName({ uid: UID9, name: '' }), 'Unnamed board …3352');
});

test('a file names its board by the end of the id, which is still that board', () => {
  assert.equal(B.sameBoard(UID9, UID9.toUpperCase()), true);
  assert.equal(B.sameBoard(UID9, '20383352'), true);
  assert.equal(B.sameBoard('20383352', UID9), true);
  assert.equal(B.sameBoard(UID9, UID12), false);
  assert.equal(B.sameBoard(UID9, '3352'), false);   // too short to be an id
  assert.equal(B.sameBoard(UID9, ''), false);
  assert.equal(B.sameBoard(null, undefined), false);
});

test('the list is the remembered boards in name order, and the probe when nothing on it has been read', () => {
  const chips = [
    { uid: UID12, name: 'Board 12', module: 'hp', lastSeen: 1790700000, firmware: 'unknown' },
    { uid: UID9, name: 'Board 9', module: null, lastSeen: 1790600000, firmware: 'dump' },
  ];
  const probes = [{ serial: '002A00', kind: 'STLink V3', access: true }];
  const unread = B.chipList(chips, probes, null);
  assert.deepStrictEqual(unread.map((r) => r.id), [UID9, UID12, 'probe:002A00']);
  assert.equal(unread[2].unread, true);
  assert.equal(unread[0].onProbe, false);

  const read = { uid: UID12, probe: { serial: '002A00', kind: 'STLink V3' }, firmware: { kind: 'unknown' } };
  const after = B.chipList(chips, probes, read);
  assert.deepStrictEqual(after.map((r) => r.id), [UID9, UID12]);
  assert.equal(after[1].onProbe, true);
  assert.equal(after[1].probe.serial, '002A00');
});

test('a board just read is listed even before the app has it among the remembered ones', () => {
  const read = { uid: UID9, probe: { serial: 'A1', kind: 'STLink V2' }, firmware: { kind: 'dump' } };
  const rows = B.chipList([], [{ serial: 'A1', kind: 'STLink V2', access: true }], read);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].uid, UID9);
  assert.equal(rows[0].firmware, 'dump');
  assert.equal(rows[0].onProbe, true);
});

test('a probe is read without the button once, when it is plugged in', () => {
  const a = { serial: 'A1', access: true };
  const b = { serial: 'B2', access: true };
  assert.deepStrictEqual(B.probeToAutoRead([], []), { serial: null, handled: [] });
  assert.deepStrictEqual(B.probeToAutoRead([a], []), { serial: 'A1', handled: [] });
  // Read or tried already, and still plugged in: not again.
  assert.deepStrictEqual(B.probeToAutoRead([a], ['A1']), { serial: null, handled: ['A1'] });
  // Unplugged, it is forgotten, so plugging it back in is new.
  assert.deepStrictEqual(B.probeToAutoRead([], ['A1']), { serial: null, handled: [] });
  assert.deepStrictEqual(B.probeToAutoRead([a, b], ['A1']), { serial: 'B2', handled: ['A1'] });
  // Two at once: the operator picks, and neither is read later behind their back.
  assert.deepStrictEqual(B.probeToAutoRead([a, b], []), { serial: null, handled: ['A1', 'B2'] });
  // One that cannot be opened is not tried.
  assert.deepStrictEqual(B.probeToAutoRead([{ serial: 'A1', access: false }], []), { serial: null, handled: [] });
});

test('a different board answering under the clip is read, once', () => {
  assert.equal(B.swapSeen(UID12, UID9, null), true);
  // The same board, or nothing answering, changes nothing.
  assert.equal(B.swapSeen(UID9, UID9, null), false);
  assert.equal(B.swapSeen(null, UID9, null), false);
  assert.equal(B.swapSeen(null, null, null), false);
  // A probe whose first read failed, and now a board answers.
  assert.equal(B.swapSeen(UID9, null, null), true);
  // That board's read was started once already and did not put it on the sheet.
  assert.equal(B.swapSeen(UID12, UID9, UID12), false);
  assert.equal(B.swapSeen(UID9, null, UID9), false);
});

test('what was read stops counting once its probe is unplugged', () => {
  const read = { uid: UID9, probe: { serial: 'A1' } };
  assert.equal(B.stillOnProbe(read, [{ serial: 'A1' }]), true);
  assert.equal(B.stillOnProbe(read, [{ serial: 'B2' }]), false);
  assert.equal(B.stillOnProbe(read, []), false);
  assert.equal(B.stillOnProbe(null, [{ serial: 'A1' }]), false);
});

test('the dump firmware is named with its id, and an earlier one is said to be earlier', () => {
  const now = B.firmwareWords({ kind: 'dump', current: true, id: '25118bdfafab456c' }, 'le');
  assert.equal(now.head, 'running dump firmware 25118bdf, not flight software');
  assert.equal(now.value, 'Dump firmware 25118bdf');
  assert.equal(now.bad, true);
  const old = B.firmwareWords({ kind: 'dump', current: false, id: '4a5f0ead11223344' }, 'le');
  assert.equal(old.head, 'running earlier dump firmware 4a5f0ead, not flight software');
  assert.equal(old.value, 'Earlier dump firmware 4a5f0ead');
  assert.equal(old.bad, true);
  // A build that does not say which dump it is: taken to be the one it carries, with no id to give.
  assert.equal(B.firmwareWords({ kind: 'dump' }, null).head, 'running dump firmware, not flight software');
  assert.equal(B.firmwareWords({ kind: 'dump' }, null).value, 'Dump firmware');
  assert.equal(now.mismatch, '');
  assert.equal(now.note, 'Not flight software. This board transmits nothing until flight software is on it.');
});

test('a flight image this app carries is named by its id and module', () => {
  const fw = { kind: 'flight', module: 'le', id: 'f84691175ddf2641' };
  const same = B.firmwareWords(fw, 'le');
  assert.deepStrictEqual(same, { kind: 'flight', head: '', value: 'Flight software f8469117 for the LE module', note: '', mismatch: '', bad: false, built: '' });
  assert.equal(B.firmwareWords(fw, 'hp').mismatch, 'This image is for the LE module and the board is marked HP.');
  assert.equal(B.firmwareWords({ kind: 'flight', module: 'hp', id: '0c2e77aa11' }, 'le').mismatch,
    'This image is for the HP module and the board is marked LE.');
  // A module nobody set is not a disagreement.
  assert.equal(B.firmwareWords(fw, null).mismatch, '');
  assert.equal(B.firmwareWords(fw, undefined).mismatch, '');
  // What a remembered board carries: the kind and the module, no id.
  assert.equal(B.firmwareWords({ kind: 'flight', module: 'hp' }, 'hp').value, 'Flight software for the HP module');
  assert.equal(B.firmwareWords({ kind: 'flight', id: 'f84691175ddf2641' }, 'hp').value, 'Flight software f8469117');
  assert.equal(B.firmwareWords({ kind: 'flight', id: 'f84691175ddf2641' }, 'hp').mismatch, '');
  assert.equal(B.firmwareWords({ kind: 'flight' }, null).value, 'Flight software');
});

test('an image this computer built is named by where it came from', () => {
  const built = { part: 'flight-le', words: 'flight LE built here 14:02 from abc1234 + config.h changed' };
  const w = B.firmwareWords({ kind: 'flight', module: 'le', id: '0a1b2c3d4e5f6071', built }, 'hp');
  assert.equal(w.value, 'Flight LE built here 14:02 from abc1234 + config.h changed');
  assert.equal(w.built, built.words);
  assert.equal(w.mismatch, 'This image is for the LE module and the board is marked HP.', 'built here or not, the module still has to agree');
  const dump = B.firmwareWords({ kind: 'dump', current: true, id: '77aa', built: { words: 'dump firmware built here 09:10 from abc1234' } }, 'le');
  assert.equal(dump.value, 'Dump firmware 77aa');
  assert.equal(dump.built, 'dump firmware built here 09:10 from abc1234');
  assert.equal(dump.bad, true);
  const bench = B.firmwareWords({ kind: 'bench', module: 'hp', built: { words: 'sensor bench HP built here 09:12 from abc1234' } }, 'hp');
  assert.equal(bench.value, 'Sensor bench for the HP module');
  assert.equal(bench.built, 'sensor bench HP built here 09:12 from abc1234');
});

test('a tree always means building, whatever else health says', () => {
  const images = [{ id: 'f84691175ddf2641', module: 'le', bytes: 63396 }];
  assert.equal(B.flashWay({ fswTree: true, flightImages: images }, 'le').way, 'build');
  // 1.2.0 sent fswLocal false with its switch off; there is no switch now
  assert.equal(B.flashWay({ fswTree: true, fswLocal: false, flightImages: images }, 'hp').way, 'build');
  assert.equal(B.flashWay({ fswTree: false, flightImages: images }, 'le').says, 'No flight software tree, so this patches the image this app carries.');
});

test('anything else is not recognised, and is called neither flight software nor fine', () => {
  const w = B.firmwareWords({ kind: 'unknown' }, 'le');
  assert.deepStrictEqual(w, {
    kind: 'unknown', head: '', value: 'Not recognised',
    note: 'Not the dump firmware and not a flight image this app carries or this computer built.', mismatch: '', bad: false, built: '',
  });
  assert.equal(B.firmwareWords({ kind: 'something new' }, 'le').value, 'Not recognised');
  for (const none of [null, undefined, {}]) {
    assert.deepStrictEqual(B.firmwareWords(none, 'le'), { kind: '', head: '', value: '', note: '', mismatch: '', bad: false, built: '' });
  }
});

test('a row carries what is known of its firmware: the read while on the probe, else what was remembered', () => {
  const chips = [
    { uid: UID9, name: 'Board 9', module: 'le', lastSeen: 1, firmware: 'flight', firmwareModule: 'hp' },
    { uid: UID12, name: 'Board 12', module: 'hp', lastSeen: 1, firmware: 'dump' },
  ];
  const read = { uid: UID12, probe: { serial: 'A1' }, firmware: { kind: 'dump', current: false } };
  const rows = B.chipList(chips, [{ serial: 'A1' }], read);
  assert.deepStrictEqual(rows[0].fw, { kind: 'flight', module: 'hp' });
  assert.equal(rows[0].firmware, 'flight');
  assert.deepStrictEqual(rows[1].fw, { kind: 'dump', current: false });
  assert.equal(rows[1].firmware, 'dump');
  assert.equal(B.chipList([{ uid: UID9, name: '', module: null, firmware: null }], [], null)[0].fw, null);
});

const PROBE_A = { serial: 'A1', kind: 'STLink V3', access: true };
const PROBE_B = { serial: 'B2', kind: 'STLink V2', access: true };

test('which probe the button reads: the only one, or the one whose row is picked', () => {
  assert.deepStrictEqual(B.probeToRead([PROBE_A], null), { serial: 'A1', why: '' });
  assert.deepStrictEqual(B.probeToRead([PROBE_A], 'gone'), { serial: 'A1', why: '' });
  assert.deepStrictEqual(B.probeToRead([PROBE_A, PROBE_B], 'B2'), { serial: 'B2', why: '' });
  assert.deepStrictEqual(B.probeToRead([PROBE_A, PROBE_B], null), { serial: null, why: 'Two probes are plugged in. Pick one in the list.' });
  assert.deepStrictEqual(B.probeToRead([PROBE_A, PROBE_B, { serial: 'C3' }], null).why, '3 probes are plugged in. Pick one in the list.');
  assert.deepStrictEqual(B.probeToRead([], null), { serial: null, why: 'No probe found' });
  // A probe this computer will not let the app open cannot be read.
  assert.deepStrictEqual(B.probeToRead([{ serial: 'A1', kind: 'STLink V3', access: false }], null),
    { serial: null, why: 'No permission to open the probe' });
  assert.deepStrictEqual(B.probeToRead([{ serial: 'A1', access: false }, PROBE_B], null), { serial: 'B2', why: '' });
});

test('the button at the head of the list says what it will do', () => {
  assert.deepStrictEqual(B.readButton([PROBE_A], false),
    { label: 'Read the board', title: 'Reads the board on the ST-Link. Can reset the board.', disabled: false });
  assert.equal(B.readButton([PROBE_A], true).disabled, true);
  assert.deepStrictEqual(B.readButton([], false),
    { label: 'Look for a probe', title: 'Looks for an ST-Link on USB', disabled: false });
});

test('what a read answers with: the board by name, or as a new one', () => {
  const chips = [{ uid: UID9, name: 'Board 9' }, { uid: UID12, name: '' }];
  assert.equal(B.readToast(UID9, chips), 'Board 9 on the ST-Link');
  assert.equal(B.readToast(UID12, chips), 'Unnamed board …3311 on the ST-Link');
  assert.equal(B.readToast('0041002f5650501320383377', chips), 'New board …3377 on the ST-Link');
  assert.equal(B.readToast(UID9, []), 'New board …3352 on the ST-Link');
});

test('a read stops counting when the probes plugged in change in any way', () => {
  assert.equal(B.probesChanged([PROBE_A], [PROBE_A]), false);
  assert.equal(B.probesChanged([PROBE_A, PROBE_B], [PROBE_B, PROBE_A]), false);
  assert.equal(B.probesChanged([PROBE_A], []), true);                    // unplugged
  assert.equal(B.probesChanged([], [PROBE_A]), true);                    // plugged in
  assert.equal(B.probesChanged([PROBE_A], [PROBE_B]), true);             // replaced
  assert.equal(B.probesChanged([PROBE_A], [PROBE_A, PROBE_B]), true);    // a second one appears
  assert.equal(B.probesChanged(undefined, []), false);
});

test('what was chosen for one board does not follow to another', () => {
  // (the board the choice was made on, the board read before, the board read now)
  assert.equal(B.dropChoice(UID9, UID9, UID12), true);
  assert.equal(B.dropChoice(UID9, null, UID12), true);
  assert.equal(B.dropChoice(null, UID9, UID12), true);
  assert.equal(B.dropChoice(UID9, UID9, UID9), false);
  // An image picked on Data before any board was read waits for the first read.
  assert.equal(B.dropChoice(null, null, UID12), false);
});

test('the steps of a job: done, the one running now, and the one it failed on', () => {
  const steps = ['listening to the board', 'writing the dump firmware', 'writing the dump firmware', 'reading the log off the chip'];
  assert.deepStrictEqual(B.stepList(steps, true, null), [
    { text: 'listening to the board', state: 'done' },
    { text: 'writing the dump firmware', state: 'done' },
    { text: 'reading the log off the chip', state: 'now' },
  ]);
  assert.deepStrictEqual(B.stepList(steps, false, null).map((s) => s.state), ['done', 'done', 'done']);
  assert.deepStrictEqual(B.stepList(steps, false, 'the board stopped answering').map((s) => s.state), ['done', 'done', 'failed']);
  assert.deepStrictEqual(B.stepList(undefined, true, null), []);
});

const RADIO = {
  layoutVersion: 1, spreadingFactor: { min: 5, max: 12 },
  le: { sleepsDuringTx: false, dbm: [{ dbm: 10 }, { dbm: 14 }, { dbm: 15 }] },
  hp: { dbm: [{ dbm: 14 }, { dbm: 17 }, { dbm: 20 }, { dbm: 22 }] },
};

test('power choices come from the table the app serves, for the module chosen', () => {
  assert.deepStrictEqual(B.powerOptions(RADIO, 'le'), [10, 14, 15]);
  assert.deepStrictEqual(B.powerOptions(RADIO, 'hp'), [14, 17, 20, 22]);
  assert.deepStrictEqual(B.powerOptions(RADIO, null), []);
  assert.deepStrictEqual(B.powerOptions(null, 'le'), []);
  assert.deepStrictEqual(B.sfOptions(RADIO), [5, 6, 7, 8, 9, 10, 11, 12]);
  assert.deepStrictEqual(B.sfOptions(null), []);
  assert.equal(B.dbmWords(14), '+14 dBm');
  assert.equal(B.dbmWords(-3), '−3 dBm');
});

test('flight software is built when a tree is at hand and patched into the carried image when it is not', () => {
  const images = [{ id: 'c4fb204d8f79e725', module: 'le', bytes: 66600 }];
  const built = B.flashWay({ fswTree: true, flightImages: images }, 'hp');
  assert.deepStrictEqual([built.way, built.verb, built.column, built.stop], ['build', 'Build and flash', 'Build with', '']);
  assert.equal(built.says, 'A flight software tree is here, so this builds the exact image.');
  // An app from before it said which could only build.
  assert.equal(B.flashWay({ flightImages: images }, 'le').way, 'build');
  assert.equal(B.flashWay(null, 'le').way, 'build');

  const patched = B.flashWay({ fswTree: false, flightImages: images }, 'le');
  assert.deepStrictEqual([patched.way, patched.verb, patched.column, patched.stop], ['carried', 'Patch and flash', 'Flash with', '']);
  assert.equal(patched.says, 'No flight software tree, so this patches the image this app carries.');
  // The module not chosen yet is the pane's to say.
  assert.equal(B.flashWay({ fswTree: false, flightImages: images }, null).stop, '');
  // Nothing carried for that module: nothing to flash this way, said where the button is.
  const none = B.flashWay({ fswTree: false, flightImages: images }, 'hp');
  assert.equal(none.stop, 'No flight software tree, and this build carries no flight image for the HP module. Flash a file instead.');
  assert.equal(none.says, none.stop);
  assert.equal(B.flashWay({ fswTree: false }, 'le').stop.includes('LE module'), true);
});

test('the radio table with no settings block is all dashes under the plain line', () => {
  const t = B.settingsTable(null, 'hp');
  assert.deepStrictEqual(t.rows.map((r) => [r.key, r.onBoard]),
    [['csid', '—'], ['sf', '—'], ['dbm', '—'], ['ground', '—'], ['air', '—']]);
  assert.equal(t.line, 'This board\'s firmware predates the settings block, so the id, spreading factor, power and interval ' +
    'it was built with cannot be read off it. Flashing it from here gives it one.');
  // The dump firmware is not an old flight image, and is not called one.
  assert.equal(B.settingsTable(null, 'hp', '', 'dump').line,
    'The dump firmware has no radio settings. Flashing from here puts flight software on this board.');
  assert.equal(B.settingsTable(null, 'hp', '', 'flight').line, t.line);
  assert.equal(t.disagree, '');
  assert.equal(t.module, '');
});

const RATE = (fixedMs, more) => Object.assign({ fixedMs, follow: false, percent: [75, 50, 35, 20], intervalMs: [10000, 30000, 60000, 120000, 180000] }, more || {});
const FLAT = (ms, more) => RATE(ms, Object.assign({ intervalMs: [ms, ms, ms, ms, ms] }, more || {}));
const BLOCK = { layoutVersion: 4, csid: 255, module: 'le', sf: 7, dbm: 14, setBy: 'build', txIntervalMs: 10000, transmit: [RATE(60000), RATE(10000)], log: [FLAT(10000), FLAT(50)], olderSlots: [], reservedClear: true };

test('a settings block fills the on-the-board column, one row per phase', () => {
  const s = BLOCK;
  const t = B.settingsTable(s, 'le');
  assert.deepStrictEqual(t.rows.map((r) => r.onBoard), ['unconfigured (255)', 'SF7', '+14 dBm', 'every 60 s, log every 10 s', 'every 10 s, log every 0.05 s']);
  assert.deepStrictEqual(t.rows.slice(3).map((r) => r.label), ['Ground', 'Air']);
  // older firmware can leave a different value in a spare slot
  const older = [{ phase: 1, name: 'falling', rate: RATE(1000), logMs: 50 }, { phase: 0, name: 'landed', rate: RATE(60000), logMs: 1000 }];
  assert.deepStrictEqual(B.settingsTable(Object.assign({}, s, { layoutVersion: 3, olderSlots: older }), 'le').rows.slice(3).map((r) => r.onBoard),
    ['every 60 s, log every 10 s; landed slot every 60 s, log every 1 s', 'every 10 s, log every 0.05 s; falling slot every 1 s, log every 0.05 s']);
  // A ground log gap of 0 writes nothing; in the air it is every reading.
  assert.deepStrictEqual(B.settingsTable(Object.assign({}, s, { log: [FLAT(0), FLAT(0)] }), 'le').rows.slice(3).map((r) => r.onBoard),
    ['every 60 s, log nothing written', 'every 10 s, log every reading']);
  // A log on the battery says so, with 0 in the words of its slot.
  assert.equal(B.settingsTable(Object.assign({}, s, { log: [FLAT(1000, { follow: true, intervalMs: [1000, 2000, 3000, 4000, 0] }), FLAT(50)] }), 'le').rows[3].onBoard,
    'every 60 s, log by battery: 1 s above 75 %, 2 s to 50 %, 3 s to 35 %, 4 s to 20 %, then nothing written (1 s when the gauge cannot be read)');
  // A layout-2 block, from before the log gaps, has the gaps and no logs.
  assert.deepStrictEqual(B.settingsTable(Object.assign({}, s, { layoutVersion: 2, log: null }), 'le').rows.slice(3).map((r) => r.onBoard),
    ['every 60 s', 'every 10 s']);
  assert.equal(t.module, 'LE');
  assert.equal(t.setBy, 'from the build, not set from here');
  assert.equal(t.line, '');
  assert.equal(t.damaged, false);
  assert.equal(t.disagree, '');
  assert.equal(t.rows[3].apart, '');
  // A phase on the battery says what it follows and what it falls back to; back to back is said.
  const ground = RATE(300000, { follow: true, percent: [90, 60, 30, 5], intervalMs: [0, 20000, 60000, 90000, 200000] });
  assert.equal(B.settingsTable(Object.assign({}, s, { transmit: [ground, RATE(10000)] }), 'le').rows[3].onBoard,
    'by battery: back to back above 90 %, 20 s to 60 %, 60 s to 30 %, 90 s to 5 %, then 200 s (300 s when the gauge cannot be read), log every 10 s');
  assert.equal(B.settingsTable(Object.assign({}, s, { transmit: [RATE(60000), RATE(0)] }), 'le').rows[4].onBoard, 'back to back, log every 0.05 s');
  // A block from before the phases has one gap, the old falling one, shown under the air.
  const old = B.settingsTable(Object.assign({}, s, { layoutVersion: 1, txIntervalMs: 250, transmit: null, log: null }), 'le');
  assert.deepStrictEqual(old.rows.slice(3).map((r) => r.onBoard),
    ['no phases in this firmware', 'every 0.25 s (no phases in this firmware)']);

  const u = B.settingsTable(Object.assign({}, s, { csid: 12, setBy: 'tool' }), 'hp');
  assert.equal(u.rows[0].onBoard, '12');
  assert.equal(u.setBy, 'set from this tool');
  assert.match(u.disagree, /The board's settings say LE and this app has it down as HP/);
  // Nothing remembered is not a disagreement; it is the never-set case, said elsewhere.
  assert.equal(B.settingsTable(s, null).disagree, '');
});

test('what the board says about a block it could not give is shown as it comes', () => {
  const damaged = B.settingsTable(null, 'le', 'the settings block is damaged; the board is running the values it was built with');
  assert.equal(damaged.line, 'the settings block is damaged; the board is running the values it was built with');
  assert.equal(damaged.damaged, true);
  const none = B.settingsTable(null, 'le', 'this image has no settings block');
  assert.equal(none.line, 'this image has no settings block');
  assert.equal(none.damaged, false);
  assert.equal(B.settingsTable(null, 'le').damaged, false);
  assert.match(B.settingsTable(null, 'le').line, /predates the settings block/);
  assert.doesNotMatch(B.settingsTable(null, 'le').line, /written/);
});

// p0 is ground, p1 the air.
const DRAFT = { csid: '255', sf: 7, dbm: 14 };
[60, 10].forEach((secs, n) => {
  DRAFT['p' + n + 's'] = String(secs); DRAFT['p' + n + 'f'] = false;
  DRAFT['p' + n + 'l'] = n === 0 ? '10' : '0.05';
  [75, 50, 35, 20].forEach((p, k) => { DRAFT['p' + n + 'soc' + k] = String(p); });
  [10, 30, 60, 120, 180].forEach((v, k) => {
    DRAFT['p' + n + 'ms' + k] = String(v);
    DRAFT['p' + n + 'lms' + k] = DRAFT['p' + n + 'l'];
  });
});

test('a row is marked as replaced only when the build would change what the board holds', () => {
  const s = BLOCK;
  const same = DRAFT;
  const none = { module: false, csid: false, sf: false, dbm: false, ground: false, air: false };
  assert.deepStrictEqual(B.buildDiffers(s, 'le', same), none);
  assert.deepStrictEqual(B.buildDiffers(s, 'hp', Object.assign({}, same, { csid: '9' })),
    Object.assign({}, none, { module: true, csid: true }));
  assert.deepStrictEqual(B.buildDiffers(s, 'le', Object.assign({}, same, { sf: 12, dbm: 15, p1s: '0.5' })),
    Object.assign({}, none, { sf: true, dbm: true, air: true }));
  // Each phase differs on its own: its gap in seconds, its switch, a threshold or a band.
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p1f: true })).air, true);
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p0soc2: '36' })).ground, true);
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p0ms4: '181' })).ground, true);
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p0ms4: '181' })).air, false);
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p0lms4: '181' })).ground, true);
  // A log gap differs on its own phase.
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p1l: '0.1' })).air, true);
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p1l: '0.1' })).ground, false);
  // A spare slot of an older block is replaced whatever the boxes say.
  assert.equal(B.buildDiffers(Object.assign({}, s, { olderSlots: [{ phase: 1, name: 'falling', rate: RATE(1000), logMs: null }] }), 'le', same).air, true);
  // A board from before the log gaps is replaced in every phase: flashing gives it them.
  assert.deepStrictEqual(B.buildDiffers(Object.assign({}, s, { log: null }), 'le', same),
    Object.assign({}, none, { ground: true, air: true }));
  // An empty or half-typed box is not the board's value.
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p1s: '' })).air, true);
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { p1lms2: '0.0x' })).air, true);
  assert.equal(B.buildDiffers(s, 'le', Object.assign({}, same, { csid: '25x' })).csid, true);
  // No module chosen yet is said elsewhere; no block read means nothing is known to be replaced.
  assert.equal(B.buildDiffers(s, null, same).module, false);
  assert.deepStrictEqual(B.buildDiffers(null, 'hp', same), none);
  // A board with no phases is replaced in every phase: flashing gives it them.
  assert.deepStrictEqual(B.buildDiffers(Object.assign({}, s, { transmit: null, log: null }), 'le', same),
    Object.assign({}, none, { ground: true, air: true }));
});

test('what a write changes is listed by row and column', () => {
  const changes = (d, n, s) => B.phaseChanges(s || BLOCK, Object.assign({}, DRAFT, d), n);
  assert.deepStrictEqual(changes({}, 1), []);
  assert.deepStrictEqual(changes({ p1f: true }, 0), []);
  // Off to on names both switches, and each row whose gap moves. The log bands were seeded flat.
  const on = changes({ p1f: true }, 1);
  assert.deepStrictEqual(on.slice(0, 2), [['Air, log by battery level', 'off', 'on'], ['Air, transmit by battery level', 'off', 'on']]);
  assert.deepStrictEqual(on.slice(2), [['Air, to 50 %, transmit', '10 s', '30 s'], ['Air, to 35 %, transmit', '10 s', '60 s'],
    ['Air, to 20 %, transmit', '10 s', '120 s'], ['Air, below 20 %, transmit', '10 s', '180 s']]);
  // A band named as the design says it.
  assert.deepStrictEqual(changes({ p1f: true, p1lms4: '5' }, 1)[2], ['Air, below 20 %, log', '0.05 s', '5 s']);
  // 0 in the words of the column and the phase.
  assert.deepStrictEqual(changes({ p0l: '0', p0lms0: '10' }, 0), [['Ground, always, log', '10 s', 'nothing written']]);
  assert.deepStrictEqual(changes({ p1l: '0' }, 1), [['Air, always, log', '0.05 s', 'every reading']]);
  assert.deepStrictEqual(changes({ p1s: '0' }, 1), [['Air, always, transmit', '10 s', 'back to back']]);
  // A threshold is one row for both columns, and a value the table holds unused is still said.
  assert.deepStrictEqual(changes({ p0soc2: '36' }, 0), [['Ground, threshold 3', '35 %', '36 %']]);
  assert.deepStrictEqual(changes({ p0ms4: '181' }, 0), [['Ground, below 20 % (not in use), transmit', '180 s', '181 s']]);
  // A column that had its own thresholds: only that column changes.
  const own = Object.assign({}, BLOCK, { log: [FLAT(10000, { percent: [70, 50, 35, 20] }), FLAT(50)] });
  assert.deepStrictEqual(changes({}, 0, own), [['Ground, log threshold 1', '70 %', '75 %']]);
  // A board that follows and a switch turned off: the Always box replaces every band.
  const follows = Object.assign({}, BLOCK, { log: [FLAT(10000), FLAT(50, { follow: true, intervalMs: [50, 100, 200, 500, 1000] })] });
  const off = changes({}, 1, follows);
  assert.deepStrictEqual(off[0], ['Air, log by battery level', 'on', 'off']);
  assert.deepStrictEqual(off.slice(1), [['Air, to 50 %, log', '0.1 s', '0.05 s'], ['Air, to 35 %, log', '0.2 s', '0.05 s'],
    ['Air, to 20 %, log', '0.5 s', '0.05 s'], ['Air, below 20 %, log', '1 s', '0.05 s']]);
});

test('the rows of a table say what the board holds and which box replaces it', () => {
  const rows = (d, s) => B.phaseRows(s === undefined ? BLOCK : s, Object.assign({}, DRAFT, d), 1);
  assert.deepStrictEqual(rows({}), [{ key: B.FIXED, log: { was: '0.05 s', changed: false }, transmit: { was: '10 s', changed: false } }]);
  const on = rows({ p1f: true, p1lms4: '5' });
  assert.deepStrictEqual(on.map((r) => r.key), [0, 1, 2, 3, 4, B.FIXED]);
  // With the board off, each band holds its fixed gap.
  assert.deepStrictEqual(on[1].transmit, { was: '10 s', changed: true });
  assert.deepStrictEqual(on[0].transmit, { was: '10 s', changed: false });
  assert.deepStrictEqual(on[4].log, { was: '0.05 s', changed: true });
  assert.deepStrictEqual(on[3].log, { was: '0.05 s', changed: false });
  // No block read: boxes and nothing to compare with.
  assert.deepStrictEqual(rows({}, null), [{ key: B.FIXED, log: { was: '', changed: false }, transmit: { was: '', changed: false } }]);
  // A board that follows the battery with the switch off.
  const follows = Object.assign({}, BLOCK, { transmit: [RATE(60000), RATE(10000, { follow: true })] });
  assert.deepStrictEqual(rows({}, follows)[0].transmit, { was: 'by battery', changed: true });
});

test('the draft is one rate per column, and a half-typed box is never 0', () => {
  const d = Object.assign({}, DRAFT, { p1f: true, p1lms2: '0.2', p1s: '' });
  assert.deepStrictEqual(B.draftRate(d, 1, 'log'), { fixedMs: 50, follow: true, percent: [75, 50, 35, 20], intervalMs: [50, 50, 200, 50, 50] });
  assert.deepStrictEqual(B.draftRate(d, 0, 'transmit'), { fixedMs: 60000, follow: false, percent: [75, 50, 35, 20], intervalMs: [10000, 30000, 60000, 120000, 180000] });
  assert.ok(Number.isNaN(B.draftRate(d, 1, 'transmit').fixedMs));
});

test('which receivers are listening on a spreading factor, by name', () => {
  const rx = [{ name: 'T-Beam 04', sf: 'SF9' }, { name: '01', sf: 'SF7' }, { name: 'North', sf: 'SF9' }, { name: 'quiet one', sf: '' }];
  assert.deepStrictEqual(B.listeningOn(7, rx), { none: false, words: '01 is on SF7' });
  assert.deepStrictEqual(B.listeningOn(9, rx), { none: false, words: 'T-Beam 04 and North are on SF9' });
  assert.deepStrictEqual(B.listeningOn(12, rx), { none: true, words: 'no receiver is on SF12' });
  assert.equal(B.listeningOn(9, rx.concat([{ name: 'West', sf: 'SF9' }])).words, 'T-Beam 04, North and West are on SF9');
  assert.deepStrictEqual(B.listeningOn(9, []), { none: true, words: 'no receiver is on SF9' });
});

test('a receiver goes by its own name, then the name kept for its id, then its id, and the port key last', () => {
  const heard = { id: 'A4C1F3', info: { sf: 7 } };
  assert.equal(B.receiverName('rx1', { key: 'rx1', rxId: 'A4C1F3', rxName: 'North', name: '01' }, heard), 'North');
  assert.equal(B.receiverName('rx1', { key: 'rx1', rxId: 'A4C1F3', rxName: null, name: '01' }, heard), '01');
  assert.equal(B.receiverName('rx1', { key: 'rx1', rxId: 'A4C1F3', rxName: null, name: null }, heard), 'T-Beam A4C1F3');
  // Before the app has heard the board announce itself, the page may already have.
  assert.equal(B.receiverName('rx2', { key: 'rx2', rxId: null, rxName: null, name: null }, heard), 'T-Beam A4C1F3');
  assert.equal(B.receiverName('rx2', { key: 'rx2' }, { id: 'B2', info: { name: 'South' } }), 'South');
  // A name given to the USB adapter before ids existed is not thrown away.
  assert.equal(B.receiverName('rx1', { key: 'rx1', rxId: 'A4C1F3' }, heard, { name: 'Left T-Beam' }), 'Left T-Beam');
  assert.equal(B.receiverName('rx1', { key: 'rx1', rxId: 'A4C1F3', name: '01' }, heard, { name: 'Left T-Beam' }), '01');
  // Web Serial and logs have no port entry at all.
  assert.equal(B.receiverName('rx3', null, { id: 'C3D4E5', info: {} }), 'T-Beam C3D4E5');
  assert.equal(B.receiverName('rx3', null, undefined), 'rx3');
  assert.equal(B.receiverName('boot6', null, undefined), 'boot6');
});

test('where a receiver\'s name is kept: on the board, on this computer against its id, or against its USB adapter', () => {
  assert.deepStrictEqual(B.namedBy({ rxId: 'A4C1F3', rxName: 'North' }, null), { by: 'board', id: 'A4C1F3' });
  assert.deepStrictEqual(B.namedBy({ rxId: 'A4C1F3', rxName: null }, null), { by: 'computer', id: 'A4C1F3' });
  assert.deepStrictEqual(B.namedBy({ rxId: null }, { id: 'A4C1F3', info: {} }), { by: 'computer', id: 'A4C1F3' });
  assert.deepStrictEqual(B.namedBy({ rxId: null }, undefined), { by: 'port', id: '' });
  assert.deepStrictEqual(B.namedBy(null, undefined), { by: 'port', id: '' });
});

test('one version is newer than another by its numbers, not its letters', () => {
  assert.equal(B.isNewer('1.10.0', '1.2.1'), true);
  assert.equal(B.isNewer('1.2.1', '1.10.0'), false);
  assert.equal(B.isNewer('1.2.1', '1.2.1'), false);
  assert.equal(B.isNewer('1.2.2', '1.2.1'), true);
  assert.equal(B.isNewer('2.0', '1.9.9'), true);
  assert.equal(B.isNewer('1.2.1', '1.2'), true);
  assert.equal(B.isNewer('1.2', '1.2.0'), false);
  // Nothing is decided from a version that is not known or does not read as one.
  assert.equal(B.isNewer('1.2.1', null), false);
  assert.equal(B.isNewer(null, '1.2.1'), false);
  assert.equal(B.isNewer('abc', '1.2.1'), false);
});

test('what a receiver\'s firmware row says', () => {
  const app = '1.2.1';
  assert.deepStrictEqual(B.receiverFirmware({ rxId: 'A4C1F3', rxFw: '1.2.1' }, app), { onBoard: '1.2.1', inApp: '1.2.1', newer: false });
  assert.deepStrictEqual(B.receiverFirmware({ rxId: 'A4C1F3', rxFw: '1.2.1' }, '1.10.0'), { onBoard: '1.2.1', inApp: '1.10.0', newer: true });
  // Announced itself and says no version: it is from before versions were reported.
  assert.deepStrictEqual(B.receiverFirmware({ rxId: 'A4C1F3', rxFw: null }, app), { onBoard: 'older than 1.2.1', inApp: '1.2.1', newer: true });
  assert.deepStrictEqual(B.receiverFirmware({ rxId: null, rxFw: null }, app), { onBoard: '—', inApp: '1.2.1', newer: false });
  assert.deepStrictEqual(B.receiverFirmware(null, null), { onBoard: '—', inApp: '—', newer: false });
});

test('receivers for the bar: a name, a spreading factor and a state word', () => {
  const now = 1000000;
  const ports = [
    { key: 'rx1', port: '/dev/ttyUSB0', status: 'open', rxId: 'A1F4C0', rxName: null, name: '01' },
    { key: 'rx2', port: '/dev/ttyUSB1', status: 'open', rxId: 'B2B2B2', rxName: null, name: null },
    { key: 'rx3', port: '/dev/ttyUSB2', status: 'open', rxId: null, rxName: null, name: null },
    { key: 'rx4', port: '/dev/ttyUSB3', status: 'error' },
  ];
  const boards = [{ port: '/dev/ttyUSB0', name: 'old name' }, { port: '/dev/ttyUSB2', name: 'Spare' }];
  const heard = new Map([
    ['rx1', { id: 'A1F4C0', info: { sf: 7 }, lastT: now - 2000 }],
    ['rx2', { id: 'B2B2B2', info: { sf: 9 }, lastT: now - 125000 }],
  ]);
  assert.deepStrictEqual(B.receiverWords(ports, boards, heard, now), [
    { key: 'rx1', name: '01', sf: 'SF7', word: 'reading', cls: 'good' },
    { key: 'rx2', name: 'T-Beam B2B2B2', sf: 'SF9', word: 'quiet for 2 min', cls: 'off' },
    { key: 'rx3', name: 'Spare', sf: '', word: 'nothing heard', cls: 'warn' },
    { key: 'rx4', name: 'rx4', sf: '', word: 'error', cls: 'bad' },
  ]);
});

test('a refusal over which board an image came from is told from any other', () => {
  assert.equal(B.isOtherBoardRefusal('this image was read off a different board (0034…3352); add force=1 to write it anyway'), true);
  // As the app words it today.
  assert.equal(B.isOtherBoardRefusal("descent-fsw-20383352-Board-9-2026-09-29T18-47-02.bin was saved off the board whose id ends 20383352, and this board's ends 20383311. One board's image is not written to another; force=1 overrides"), true);
  assert.equal(B.isOtherBoardRefusal('no debug probe found'), false);
  assert.equal(B.isOtherBoardRefusal(''), false);
});
