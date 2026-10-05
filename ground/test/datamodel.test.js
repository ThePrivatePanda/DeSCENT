const test = require('node:test');
const assert = require('node:assert');
const D = require('../web/js/datamodel.js');

test('a duration reads in the two largest units it needs', () => {
  assert.equal(D.duration(4), '4 ms');
  assert.equal(D.duration(48000), '48 s');
  assert.equal(D.duration(26 * 60000 + 48000), '26 min 48 s');
  assert.equal(D.duration(60000), '1 min');
  assert.equal(D.duration(3600e3 + 56 * 60000 + 30000), '1 h 56 min');
  assert.equal(D.duration(2 * 3600e3), '2 h');
  assert.equal(D.duration(NaN), '—');
  assert.equal(D.duration(-5), '—');
});

test('a clock rate is said as a percentage and a direction', () => {
  assert.equal(D.clockWords(7500), '0.75% fast');
  assert.equal(D.clockWords(-12000), '1.2% slow');
  assert.equal(D.clockWords(100000), '10% fast');
  // Under a hundredth of a percent the percentage would round to nothing.
  assert.equal(D.clockWords(40), '40 ppm fast');
  assert.equal(D.clockWords(0), 'on time');
  assert.equal(D.clockWords(null), '—');
  assert.equal(D.clockWords(undefined), '—');
});

test('sizes', () => {
  assert.equal(D.bytes(512), '512 B');
  assert.equal(D.bytes(262144), '256 kB');
  assert.equal(D.bytes(5.2 * 1048576), '5.2 MB');
});

test('the board part of a pull folder or an image name, whichever side the date is on', () => {
  assert.equal(D.boardPart('descent-pull-2026-09-26T16-45-50-ttyACM1/ground.log'), 'ttyACM1');
  assert.equal(D.boardPart('descent-pull-board-9-2026-09-26_16-45/ground.log'), 'board-9');
  assert.equal(D.boardPart('descent-pull-2026-09-26T16-45-50-ttyACM1/descent-ground-fsw-ttyACM1-2026-09-26T16-45-50.bin'), 'ttyACM1');
  assert.equal(D.boardPart('descent-ground-backup-ttyUSB0-1790722068411.bin'), 'ttyUSB0');
  assert.equal(D.boardPart('descent-2026-10-04_19-12.log'), '');
});

test('rows say what each file is, where it came from and what can be done with it', () => {
  const logs = [
    { name: 'descent-2026-10-04_19-12.log', bytes: 41000, when: 1791141120, kind: 'session' },
    { name: 'descent-pull-2026-09-26T16-45-50-ttyACM1/ground.log', bytes: 5452595, when: 1790741150, kind: 'pull', uid: '003400415650501320383352' },
    { name: 'descent-image-0034-2026-09-29T18-47-00.bin', bytes: 262144, when: 1790707620, kind: 'image', uid: '003400415650501320383352' },
    { name: 'descent-2026-09-24_20-41.log', bytes: 900, when: 1790282460, kind: 'session' },
  ];
  const rows = D.rows(logs, {
    current: '/home/op/DeSCENT/descent-2026-10-04_19-12.log',
    names: { '003400415650501320383352': 'Board 9' },
  });
  assert.deepStrictEqual(rows.map((r) => r.what), ['Live session', 'Flight log pull', 'Firmware image', 'Live session']);
  assert.deepStrictEqual(rows.map((r) => r.current), [true, false, false, false]);
  assert.deepStrictEqual(rows.map((r) => r.from), ['Receivers', 'Board 9', 'Board 9', 'Receivers']);
  assert.deepStrictEqual(rows[0].actions, ['open', 'log', 'csv']);
  assert.deepStrictEqual(rows[1].actions, ['open', 'log', 'csv', 'raw']);
  assert.deepStrictEqual(rows[2].actions, ['save', 'write']);
  assert.equal(rows[1].raw, 'descent-pull-2026-09-26T16-45-50-ttyACM1/dump.txt');
  assert.equal(rows[1].report, 'descent-pull-2026-09-26T16-45-50-ttyACM1/report.json');
  assert.equal(rows[1].folder, 'descent-pull-2026-09-26T16-45-50-ttyACM1');
  assert.equal(rows[0].raw, null);
  assert.equal(rows[1].size, '5.2 MB');
});

test('rows are newest first, whatever order the app listed them in', () => {
  const rows = D.rows([
    { name: 'a.log', bytes: 1, when: 100, kind: 'session' },
    { name: 'c.log', bytes: 1, when: 300, kind: 'session' },
    { name: 'b.log', bytes: 1, when: 200, kind: 'session' },
  ], {});
  assert.deepStrictEqual(rows.map((r) => r.name), ['c.log', 'b.log', 'a.log']);
});

test('a board nobody named goes by the board part of its file name', () => {
  const rows = D.rows([
    { name: 'descent-pull-2026-09-26T16-45-50-ttyACM1/ground.log', bytes: 5, when: 1, kind: 'pull' },
    { name: 'descent-pull-x/ground.log', bytes: 5, when: 1, kind: 'pull', uid: '0011223344556677889900aa' },
  ], {});
  assert.equal(rows[0].from, 'ttyACM1');
  // A UID with no name is still a board: its last four digits, as on the sheet.
  assert.equal(rows[1].from, 'board …00aa');
});

test('a file that names its board by the end of the id still gets the board\'s name', () => {
  const rows = D.rows([
    { name: 'descent-fsw-20383352-Board-9-2026-09-29T18-47-02.bin', bytes: 5, when: 2, kind: 'image', uid: '20383352' },
    { name: 'descent-pull-2026-09-26T16-45-50-left/descent-fsw-2039abcd-left-2026-09-26T16-45-50.bin', bytes: 5, when: 1, kind: 'image', uid: '2039ABCD' },
  ], { names: { '003400415650501320383352': 'Board 9' } });
  assert.equal(rows[0].from, 'Board 9');
  assert.equal(rows[1].from, 'board …ABCD');
  assert.equal(rows[1].raw, null);
});

test('the folder everything is kept in comes from the path of the log being written', () => {
  assert.equal(D.keptIn('/home/op/DeSCENT/descent-2026-10-04_19-12.log'), '/home/op/DeSCENT');
  assert.equal(D.keptIn('C:\\Users\\op\\DeSCENT\\a.log'), 'C:\\Users\\op\\DeSCENT');
  assert.equal(D.keptIn(null), '');
  assert.equal(D.keptIn('a.log'), '');
});

const REPORT = {
  format: 'raw', found: 20663, boardGood: 20663, boardBad: 0, read: 20592, failedCheck: 71, missing: 0,
  complete: false, longestReaderGapMs: 4, bytes: 5452595,
  boots: [
    { n: 4, records: 488, firstMs: 1200, lastMs: 1200 + 81 * 60000, timeSource: 'none', uptime0Ms: 0, anchorsUsed: 0, anchorsRejected: 0, clockFastPpm: null },
    { n: 6, records: 5914, firstMs: 5000, lastMs: 5000 + 116 * 60000, timeSource: 'anchor', uptime0Ms: Date.UTC(2026, 8, 26, 16, 42, 0), anchorsUsed: 690, anchorsRejected: 3, clockFastPpm: 7500 },
    { n: 7, records: 11184, firstMs: 900, lastMs: 900 + 26 * 60000 + 48000, timeSource: 'anchor', uptime0Ms: Date.UTC(2026, 8, 26, 18, 37, 26), anchorsUsed: 158, anchorsRejected: 0, clockFastPpm: -40 },
  ],
};

test('a report becomes one row per boot', () => {
  const rows = D.bootRows(REPORT);
  assert.deepStrictEqual(rows[0], {
    boot: '4', records: '488', ranFor: '1 h 21 min', started: 'not dated', dated: false, anchors: 'none', clock: '—', ids: '—',
  });
  assert.deepStrictEqual(rows[1], {
    boot: '6', records: '5,914', ranFor: '1 h 56 min', started: '26 Sep 2026 16:42:05 UTC', dated: true,
    anchors: '690 used, 3 left out', clock: '0.75% fast', ids: '—',
  });
  assert.equal(rows[2].anchors, '158 used');
  assert.equal(rows[2].clock, '40 ppm slow');
  assert.equal(rows[2].ranFor, '26 min 48 s');
});

test('which board the records say they came from: one id, several, or none', () => {
  assert.equal(D.boardIdWords([13]), 'board id 13');
  assert.equal(D.boardIdWords([9, 13]), 'board ids 9 and 13');
  assert.equal(D.boardIdWords([9, 12, 13]), 'board ids 9, 12 and 13');
  assert.equal(D.boardIdWords([]), '');
  assert.equal(D.boardIdWords(undefined), '');   // an older report does not say
  assert.equal(D.boardIdWords(null), '');
});

test('each boot carries its own board ids, and a dash when the report has none', () => {
  const rows = D.bootRows({ boots: [
    { n: 1, records: 2, firstMs: 0, lastMs: 10, timeSource: 'none', csids: [13] },
    { n: 2, records: 2, firstMs: 0, lastMs: 10, timeSource: 'none', csids: [9, 13] },
    { n: 3, records: 2, firstMs: 0, lastMs: 10, timeSource: 'none', csids: [] },
    { n: 4, records: 2, firstMs: 0, lastMs: 10, timeSource: 'none' },
  ] });
  assert.deepStrictEqual(rows.map((r) => r.ids), ['13', '9, 13', '—', '—']);
});

test('records from a different board than the one read are said so, and only when both ids are known', () => {
  assert.equal(D.idMismatch([13], 9), 'The records carry board id 13, not 9.');
  assert.equal(D.idMismatch([9, 13], 12), 'The records carry board ids 9 and 13, not 12.');
  assert.equal(D.idMismatch([13], 13), '');
  assert.equal(D.idMismatch([9, 13], 9), '');
  // 255 is a board nobody has given an id, so there is nothing to compare with.
  assert.equal(D.idMismatch([13], 255), '');
  assert.equal(D.idMismatch([13], null), '');
  assert.equal(D.idMismatch([13], undefined), '');
  assert.equal(D.idMismatch([], 9), '');
  assert.equal(D.idMismatch(undefined, 9), '');
});

test('a boot whose anchor gives no usable date is not dated', () => {
  const rows = D.bootRows({ boots: [{ n: 1, records: 2, firstMs: 0, lastMs: 10, timeSource: 'anchor', uptime0Ms: null }] });
  assert.equal(rows[0].started, 'not dated');
  assert.equal(rows[0].dated, false);
  assert.deepStrictEqual(D.bootRows({}), []);
  assert.deepStrictEqual(D.bootRows(null), []);
});

test('how the transfer went, with what fell short marked', () => {
  const rows = D.transferRows(REPORT);
  const by = Object.fromEntries(rows.map((r) => [r.key, r]));
  assert.equal(by.found.value, '20,663 records');
  assert.equal(by.read.value, '20,592');
  assert.equal(by.failed.value, '71');
  assert.equal(by.failed.short, '0.3%');
  assert.equal(by.missing.value, '0');
  assert.equal(by.missing.short, '');
  assert.equal(by.gap.value, '4 ms');
  assert.equal(by.bytes.value, '5.2 MB');
  assert.equal(by.whole.value, 'no');
  assert.equal(by.whole.short, 'incomplete');
});

test('a board that did not say how much it holds, and one with records bad on the chip', () => {
  const quiet = Object.fromEntries(D.transferRows({ found: null, read: 12, failedCheck: 0, missing: 0, complete: false, longestReaderGapMs: 0, bytes: 100 }).map((r) => [r.key, r]));
  assert.equal(quiet.found.value, 'it did not say');
  assert.equal(quiet.failed.short, '');
  const bad = Object.fromEntries(D.transferRows(Object.assign({}, REPORT, { boardBad: 12, complete: true })).map((r) => [r.key, r]));
  assert.equal(bad.found.value, '20,663 records, 12 of them bad on the chip');
  assert.equal(bad.whole.value, 'yes');
  assert.equal(bad.whole.short, '');
});

test('when the running app was built, and nothing when it does not say', () => {
  const now = new Date(2026, 9, 5, 15, 0);
  const built = new Date(2026, 9, 5, 12, 20).getTime() / 1000;
  assert.equal(D.builtWords({ built }, now), 'Built 5 Oct, 12:20');
  assert.equal(D.builtWords({ built: new Date(2025, 11, 31, 9, 5).getTime() / 1000 }, now), 'Built 31 Dec 2025, 09:05');
  for (const h of [{}, { built: 0 }, { built: null }, { built: 'x' }, null]) assert.equal(D.builtWords(h, now), '');
});

test('local day and time for the table, with the year only when it is not this one', () => {
  const now = new Date(2026, 9, 5, 12, 0);
  assert.equal(D.dayTime(new Date(2026, 8, 26, 16, 42), now), '26 Sep, 16:42');
  assert.equal(D.dayTime(new Date(2025, 0, 3, 9, 5), now), '3 Jan 2025, 09:05');
  assert.equal(D.dayTime(new Date(NaN), now), '—');
  assert.equal(D.day(new Date(2026, 8, 29, 1, 2), now), '29 Sep');
});
