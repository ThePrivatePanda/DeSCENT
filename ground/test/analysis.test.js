// The Analysis screen: what it paints for a check of a pull, from the JSON /api/check gives, and
// what it asks the app for. The pull below is made up, laid out the way check.rs writes one.
const test = require('node:test');
const assert = require('node:assert');
const { page } = require('./mock/fakepage.js');
const AN = require('../web/js/analysis.js');

const NOW = new Date(2026, 10, 7, 19, 0, 0);
const sensor = (name, records, clear, longest) => ({ name, clear, records, clearMs: clear ? 1000 : 0, spells: (longest || []).length, longest: longest || [] });
const sensors = (records, gpsClear) => ['accel', 'gyro', 'mag', 'quat', 'gps', 'soc', 'env']
  .map((n) => sensor(n, records, n === 'gps' ? gpsClear : 0, n === 'gps' && gpsClear ? [{ fromMs: 1000, ms: 9000, toEnd: true }] : []));
const range = (key, words, unit, flag, o) => Object.assign({ key, words, unit, flag, min: null, minMs: null, max: null, maxMs: null, set: 0, clear: 0, heldFix: 0 }, o);
const boot = (o) => Object.assign({
  n: 0, records: 0, firstMs: 0, lastMs: 0, startUtc: null, anchors: 0, anchorsRejected: 0, csids: [15], resetCsr: null, reset: null,
  hardwareWatchdog: false, dropped: 0, droppedEvents: [], otherEvents: [], systemEvents: [], systemRaw: [], events: [], eventsLeftOut: 0,
  phases: [], ranges: [], end: 'reset', endReset: null, fastMs: 0, steadyMs: 0, gapMs: 0, gaps: [], spacingChanges: [], uptimeBackward: 0,
  sensors: [], fresh: 0, badPacketCrc: 0, verdict: '',
}, o);
const run = (o) => Object.assign({ count: 1, part: 'imu', reason: 'none', detail: 0, detailWords: '', reasonMeaning: null }, o);

// Boot 0 changes spacing and has no phase events, boot 1 reset before its first record, boot 2 is
// the one the board was pulled out of and went to sleep on a held fix
const BOOTS = [
  boot({
    n: 0, records: 40, firstMs: 1000, lastMs: 30000, reset: 'PIN+SFT', endReset: 'PIN+BOR', fastMs: 10000, steadyMs: 15000, gapMs: 4000,
    spacingChanges: [{ ms: 5000, to: 'steady' }, { ms: 20000, to: 'fast' }], gaps: [{ afterMs: 21000, ms: 4000, droppedRose: false }],
    sensors: sensors(40, 40), verdict: 'boot 0, 29.0 s, 40 records: nothing flagged, reset PIN+SFT',
    ranges: [range('envAlt', 'barometer altitude', 'm', 'env', { min: 233.31, minMs: 1000, max: 251.2, maxMs: 9000, set: 40 })],
  }),
  boot({ n: 1, reset: 'PIN+BOR', endReset: 'PIN', csids: [], verdict: 'boot 1, 0 ms, 0 records: no records, reset PIN+BOR' }),
  boot({
    n: 2, records: 30, firstMs: 1300, lastMs: 95000, reset: 'PIN', end: 'pull', sensors: sensors(30, 0),
    systemEvents: [{ source: 'phase', what: 'bootairend', count: 1, records: 1, firstMs: 37004, lastMs: 37004, parts: ['imu'] },
      { source: 'phase', what: 'asleep', count: 1, records: 1, firstMs: 50000, lastMs: 50000, parts: ['gps'] },
      { source: 'bus', what: 'jam', count: 2, records: 1, firstMs: 922, lastMs: 930, parts: ['imu'] }],
    events: [
      run({ firstMs: 922, lastMs: 930, count: 2, source: 'bus', what: 'jam', reason: 'sdalow', reasonMeaning: 'SDA low', detail: 74, detailWords: 'address 0x4A', meaning: 'a bus line was low right after a transfer, blamed on that address' }),
      run({ firstMs: 37004, lastMs: 37004, source: 'phase', what: 'bootairend', reason: 'quiet', reasonMeaning: 'no moving IMU reading for the landed time', meaning: 'boot-air ended' }),
      run({ firstMs: 41000, lastMs: 41000, source: 'phase', what: 'imufast', reason: 'moving', meaning: 'the IMU stepped up to its fast rate on the ground' }),
      run({ firstMs: 50000, lastMs: 50000, source: 'phase', what: 'asleep', part: 'gps', reason: 'goodfix', detail: 2400, detailWords: 'hAcc 2400 mm', meaning: 'landed with a fix held' }),
    ],
    phases: [
      { fromMs: 0, toMs: 37004, phase: 'boot-air', by: null, reason: null, count: 0, heldFix: false },
      { fromMs: 37004, toMs: 50000, phase: 'ground', by: 'bootairend', reason: 'quiet', count: 1, heldFix: false },
      { fromMs: 50000, toMs: 95000, phase: 'landed, asleep', by: 'asleep', reason: 'goodfix', count: 1, heldFix: true },
    ],
    ranges: [range('gpsAlt', 'GPS altitude', 'm', 'gps', { min: 101.5, minMs: 1300, max: 140, maxMs: 30000, set: 12, heldFix: 18 })],
    verdict: 'boot 2, 93.7 s, 30 records: nothing flagged, reset PIN',
  }),
];
const PULL = {
  kind: 'pull', name: '1813-board15', file: 'x/dump.txt', board: 'board 15', uid: '203237455543500D00060024', fromLog: false, format: 'raw',
  found: 70, boardGood: 70, boardBad: 0, read: 70, failedCheck: 0, missing: 0, ignoredOldFormat: 0, complete: true, wrapped: false,
  printings: 1, bytes: 9000, csids: [15], unreadLines: 0, unreadSamples: [], bootsNotInLog: [], lockEvents: 0, csidDiffers: null,
  // worst first, the way check.rs sorts them
  boots: [BOOTS[2], BOOTS[0], BOOTS[1]],
  consoles: [{ source: 'dump.txt', settings: 'from=block csid=15 module=hp sf=7 dbm=14 fixedms=5000,0', settingsCut: false, refused: null, watch: {}, others: {}, phases: [] }],
};
const ITEM = { kind: 'pull', name: '2026-11-07/pulls/1813-board15', title: 'board15', at: '7 Nov, 18:13', log: '2026-11-07/pulls/1813-board15/ground.log' };
const text = (html) => html.replace(/<[^>]*>/g, ' ').replace(/&#39;|&quot;/g, '"').replace(/&amp;/g, '&').replace(/\s+/g, ' ').trim();
const paneOf = (html, n) => html.split('data-bootpane="').find((p) => p.startsWith(n + '"')) || '';

test('every boot is in the table in chip order, the one with no records too, and the last ended with the pull', () => {
  const html = AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set(), now: NOW });
  const rows = text(html.slice(html.indexOf('<tbody>'))).match(/Boot \d/g);
  assert.deepStrictEqual(rows, ['Boot 0', 'Boot 1', 'Boot 2']);
  assert.match(text(html), /Boot 1 0 — reset PIN no GPS time none 0 0/);
  assert.match(text(html), /Boot 0 40 up 1\.0 s to 30\.0 s reset PIN, BOR/);
  assert.match(text(html), /Boot 2 30 up 1\.3 s to 95\.0 s restarted for the pull/);
  assert.match(text(html), /As the board printed them when it restarted for the pull/);
});

test('a boot with no phase events gets no phases, whatever its record spacing', () => {
  const html = AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set([0]), now: NOW });
  const zero = paneOf(html, 0);
  assert.ok(zero, 'boot 0 opened');
  assert.match(text(zero), /No phase events in this boot\. Firmware from before 2026-11-06 wrote none\./);
  assert.doesNotMatch(zero, /class="seg/);
  assert.match(text(zero), /Record spacing, which is not a phase: 10\.0 s close together, 15\.0 s at a steady slower rate, 4\.0 s in gaps, changing 2 times/);
  assert.match(text(zero), /4\.0 s after up 21\.0 s · cause not in the log/);
  assert.match(text(zero), /gps .*clear on all 40/);
});

test('the phases are the phase events, and asleep is a held fix until the boot ends', () => {
  const html = AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set([2]), now: NOW });
  const two = paneOf(html, 2);
  assert.equal((two.match(/class="seg/g) || []).length, 3);
  assert.equal((two.match(/class="tick"/g) || []).length, 1, 'imufast is a tick, not a phase');
  assert.match(text(two), /boot-air up 0 ms to 37\.0 s every boot starts in boot-air/);
  assert.match(text(two), /ground up 37\.0 s to 50\.0 s by bootairend, quiet/);
  assert.match(text(two), /landed, asleep, held fix up 50\.0 s to 95\.0 s by asleep, goodfix/);
  assert.match(text(two), /While landed asleep the GPS fields are the fix it held/);
  assert.match(text(two), /18 readings on a held fix left out/);
  assert.match(text(two), /up 922 ms to 930 ms bus jam on imu sdalow SDA low 2 address 0x4A the last time/);
  assert.match(text(two), /This boot ended when the board was restarted for the pull/);
});

test('a boot with no records says so and draws no flags or readings', () => {
  const one = paneOf(AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set([1]), now: NOW }), 1);
  assert.match(text(one), /Boot 1 0 records · no GPS time · ended: reset PIN/);
  assert.match(text(one), /No records in this boot\./);
  assert.doesNotMatch(one, /Validity flags|Readings/);
});

test('nothing is called healthy, fine or flagged-free, and the verdict line is not shown', () => {
  const html = AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set([0, 1, 2]), now: NOW });
  assert.doesNotMatch(text(html).toLowerCase(), /nothing flagged|healthy|no issues|looks (fine|ok|wrong|right)|\bnormal\b|plausib|\bwrong\b/);
});

test('an empty pull says nothing was read', () => {
  const empty = Object.assign({}, PULL, { format: 'none', read: 0, found: null, complete: false, bytes: 0, boots: [], consoles: [] });
  const html = AN.pullHtml(empty, { item: ITEM, report: null, open: new Set(), now: NOW });
  assert.match(text(html), /Nothing in this pull reads as a record: dump\.txt is 0 B\./);
  assert.match(text(html), /nothing read/);
  assert.match(text(html), /No settings line in this pull\./);
  assert.doesNotMatch(html, /class="pane an-boots"/);
});

test("pulls and session logs are listed newest first, and a pull's ground.log is not listed twice", () => {
  const pulls = [
    { name: '2026-11-07/pulls/1813-board15', when: 300, dumpBytes: 9000, log: true, report: true, uid: '203237455543500D00060024' },
    { name: '2026-11-07/pulls/1742-board15', when: 100, dumpBytes: 0, log: false, report: false, uid: null },
    { name: 'descent-pull-2026-09-26T17-00-00-ttyACM1', when: 50, dumpBytes: null, log: false, report: false, uid: null },
  ];
  const logs = [
    { name: '2026-11-07/pulls/1813-board15/ground.log', kind: 'pull', when: 300, bytes: 1 },
    { name: '2026-11-07/logs/1800-ground.log', kind: 'session', when: 200, bytes: 10 },
  ];
  const items = AN.itemsOf(pulls, logs, { '203237455543500D00060024': 'Falcon 3' }, NOW);
  assert.deepStrictEqual(items.map((i) => [i.kind, i.title, i.check]), [
    ['pull', 'Falcon 3', '2026-11-07/pulls/1813-board15/dump.txt'],
    ['log', 'Session log', '2026-11-07/logs/1800-ground.log'],
    ['pull', 'board15', '2026-11-07/pulls/1742-board15/dump.txt'],
    ['pull', 'ttyACM1', null],
  ]);
  assert.equal(items[0].at, '7 Nov, 18:13', 'a pull is dated by its folder');
  assert.match(text(AN.itemHtml(items[2], null)), /empty/);
  assert.match(text(AN.itemHtml(items[3], null)), /no dump\.txt/);
});

function hub(o) {
  const asked = [];
  return {
    asked, native: true, log: '/home/op/DeSCENT/2026-11-07/logs/1800-ground.log',
    pulls: async () => [
      { name: '2026-11-07/pulls/1813-board15', when: 300, dumpBytes: 9000, log: true, report: true, uid: null },
      { name: '2026-11-07/pulls/1742-board15', when: 100, dumpBytes: 0, log: false, report: false, uid: null },
    ],
    logs: async () => [],
    chips: async () => [],
    report: async () => ({ firmware: { dump: { words: 'dump firmware built here 17:38 from 886261c' }, onBoard: null } }),
    check: async (name) => {
      asked.push(name);
      if (o && o.fail) throw new Error('2026-11-07/pulls/1813-board15/dump.txt: No such file or directory');
      if (name.includes('1742')) return { items: [Object.assign({}, PULL, { format: 'none', read: 0, boots: [], consoles: [], bytes: 0 })] };
      return { items: [PULL] };
    },
  };
}

async function screen(o) {
  const h = hub(o);
  const p = page(['datamodel.js', 'analysis.js'], h, {});
  await p.parts.analysis.show();
  await p.settle();
  return Object.assign(p, { hub: h, press: (attrs) => p.fire('screen-analysis', 'click', attrs) });
}

test('the screen picks the newest pull, asks the app to read it and opens its newest boot', async () => {
  const p = await screen();
  assert.deepStrictEqual(p.hub.asked, ['2026-11-07/pulls/1813-board15/dump.txt']);
  assert.match(p.text('an-list'), /^board15 Pull · 7 Nov, 18:13 board15 Pull · 7 Nov, 17:4\d · empty$/);
  const sheet = p.html('an-sheet');
  assert.match(text(sheet), /Read with the dump firmware built here 17:38 from 886261c\./);
  assert.ok(paneOf(sheet, 2) && !paneOf(sheet, 0), 'only the newest boot is open');
  await p.press({ 'data-boot': '0' });
  assert.ok(paneOf(p.html('an-sheet'), 0), 'a press on a boot opens it');
  await p.press({ 'data-boot': '2' });
  assert.ok(!paneOf(p.html('an-sheet'), 2), 'and a second press closes it');
});

test("Open in Replay hands the pull's ground.log to the app, with its dump beside it", async () => {
  const p = await screen();
  await p.press({ 'data-open': '2026-11-07/pulls/1813-board15' });
  // made in the page's own realm, so compared as JSON
  assert.equal(JSON.stringify(p.opened), JSON.stringify([['2026-11-07/pulls/1813-board15/ground.log',
    { raw: '2026-11-07/pulls/1813-board15/dump.txt', dir: '/home/op/DeSCENT/2026-11-07/pulls/1813-board15' }]]));
});

test('an empty pull is read and says so, and a check the app could not do says why', async () => {
  const p = await screen();
  await p.press({ 'data-pick': '2026-11-07/pulls/1742-board15' });
  assert.match(p.text('an-sheet'), /Nothing in this pull reads as a record: dump\.txt is 0 B\./);
  const failed = await screen({ fail: true });
  assert.match(failed.text('an-sheet'), /Could not read 2026-11-07\/pulls\/1813-board15\/dump\.txt: .*No such file/);
});
