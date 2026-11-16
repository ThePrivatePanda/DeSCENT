// The Analysis screen: what it paints for a check of a pull, from the JSON /api/check gives, and
// what it asks the app for. The pull below is made up, laid out the way check.rs writes one.
const test = require('node:test');
const assert = require('node:assert');
const { page } = require('./mock/fakepage.js');
const AN = require('../web/js/analysis.js');

const NOW = new Date(2026, 10, 7, 19, 0, 0);
const spacing = (o) => Object.assign({ air: null, ground: null, none: null }, o);
const sensor = (name, records, clear, longest, firstValidMs) => ({ name, clear, records, clearMs: clear ? 1000 : 0, spells: (longest || []).length, longest: longest || [], firstValidMs,
  lastValidMs: firstValidMs === null ? null : 30000, spacing: spacing(firstValidMs === null ? {} : { none: 750 }) });
const sensors = (records, gpsClear) => ['accel', 'gyro', 'mag', 'quat', 'gps', 'soc', 'env']
  .map((n) => sensor(n, records, n === 'gps' ? gpsClear : 0, n === 'gps' && gpsClear ? [{ fromMs: 1000, ms: 9000, toEnd: true }] : [], !records || (n === 'gps' && gpsClear === records) ? null : 1000));
const range = (key, words, unit, flag, o) => Object.assign({ key, words, unit, flag, min: null, minMs: null, max: null, maxMs: null, set: 0, clear: 0, heldFix: 0 }, o);
// every reading check.rs sends, in its order, with those given in o filled in
const ranges = (o) => [
  ['envAlt', 'barometer altitude', 'm', 'env'], ['gpsAlt', 'GPS altitude', 'm', 'gps'], ['lat', 'GPS latitude', '°', 'gps'], ['lon', 'GPS longitude', '°', 'gps'],
  ['accel', 'linear acceleration', 'm/s²', 'accel'], ['gyro', 'rotation rate', 'deg/s', 'gyro'], ['mag', 'magnetic field', 'µT', 'mag'], ['pressure', 'pressure', 'hPa', 'env'],
  ['temp', 'temperature', '°C', 'env'], ['humidity', 'humidity', '%RH', 'env'], ['battery', 'cell voltage', 'V', 'soc'], ['counter', 'packet counter, the last one sent', '', null],
].map(([key, words, unit, flag]) => range(key, words, unit, flag, o[key]));
const boot = (o) => Object.assign({
  n: 0, records: 0, firstMs: 0, lastMs: 0, startUtc: null, anchors: 0, anchorsRejected: 0, anchorMs: [], csids: [15], resetCsr: null, reset: null,
  hardwareWatchdog: false, dropped: 0, droppedEvents: [], otherEvents: [], systemEvents: [], systemRaw: [], events: [], eventsLeftOut: 0,
  phases: [], ranges: ranges({}), end: 'reset', endReset: null, fastMs: 0, steadyMs: 0, gapMs: 0, gaps: [], spacingChanges: [], uptimeBackward: 0,
  recordSpacing: spacing({}), sensors: sensors(0, 0), imuFirstValidMs: null, fresh: 0, badPacketCrc: 0, verdict: '',
}, o);
const run = (o) => Object.assign({ count: 1, part: 'imu', reason: 'none', detail: 0, detailWords: '', reasonMeaning: null }, o);

// Boot 0 changes spacing and has no phase events, boot 1 reset before its first record, boot 2 is
// the one the board was pulled out of and went to sleep on a held fix
const BOOTS = [
  boot({
    n: 0, records: 40, firstMs: 1000, lastMs: 30000, reset: 'PIN+SFT', resetCsr: 0x14000000, endReset: 'PIN+BOR', fastMs: 10000, steadyMs: 15000, gapMs: 4000,
    spacingChanges: [{ ms: 5000, to: 'steady' }, { ms: 20000, to: 'fast' }], gaps: [{ afterMs: 21000, ms: 4000, droppedRose: false }],
    sensors: sensors(40, 40), imuFirstValidMs: 1000, verdict: 'boot 0, 29.0 s, 40 records: nothing flagged, reset PIN+SFT',
    ranges: ranges({ envAlt: { min: 233.31, minMs: 1000, max: 251.2, maxMs: 9000, set: 40 }, gpsAlt: { clear: 40 } }), recordSpacing: spacing({ none: 750 }),
  }),
  boot({ n: 1, reset: 'PIN+BOR', endReset: 'PIN', csids: [], verdict: 'boot 1, 0 ms, 0 records: no records, reset PIN+BOR' }),
  boot({
    n: 2, records: 30, firstMs: 1300, lastMs: 95000, reset: 'PIN', resetCsr: 0x04000000, anchors: 2, anchorMs: [20000, 40000, 50000], anchorsRejected: 1, end: 'pull', imuFirstValidMs: 19400,
    sensors: sensors(30, 0).map((s) => (s.name === 'accel' ? Object.assign(s, { clear: 3, spells: 1, longest: [{ fromMs: 1300, ms: 18100, toEnd: false }] }) : s)),
    systemEvents: [{ source: 'phase', what: 'bootairend', count: 1, records: 1, firstMs: 37004, lastMs: 37004, parts: ['imu'] },
      { source: 'phase', what: 'asleep', count: 1, records: 1, firstMs: 50000, lastMs: 50000, parts: ['gps'] },
      { source: 'bus', what: 'jam', count: 2, records: 1, firstMs: 922, lastMs: 930, parts: ['imu'] }],
    events: [
      run({ firstMs: 922, lastMs: 930, count: 2, source: 'bus', what: 'jam', reason: 'sdalow', reasonMeaning: 'the data line SDA was low', detail: 74, detailWords: 'address 0x4A',
        meaning: 'Right after a transfer with this part an I2C line was left held low, so this part is blamed.' }),
      run({ firstMs: 37004, lastMs: 37004, source: 'phase', what: 'bootairend', reason: 'quiet', reasonMeaning: 'the IMU read no movement for long enough, with the altitude flat where the rule needs it',
        meaning: 'Every boot starts in boot-air, logging at full rate, in case the reset came in the middle of a fall.' }),
      run({ firstMs: 41000, lastMs: 41000, source: 'phase', what: 'imufast', reason: 'moving', meaning: 'On the ground the IMU read movement, so the board stepped it up to its fast rate.' }),
      run({ firstMs: 50000, lastMs: 50000, source: 'phase', what: 'asleep', part: 'gps', reason: 'goodfix', reasonMeaning: 'enough good GPS fixes in a row', detail: 2400, detailWords: 'hAcc 2400 mm',
        meaning: 'Landed and holding a good GPS fix.' }),
    ],
    phases: [
      { fromMs: 0, toMs: 37004, phase: 'boot-air', by: null, reason: null, reasonMeaning: null, count: 0, heldFix: false },
      { fromMs: 37004, toMs: 50000, phase: 'ground', by: 'bootairend', reason: 'quiet', reasonMeaning: 'the IMU read no movement for long enough, with the altitude flat where the rule needs it', count: 1, heldFix: false },
      { fromMs: 50000, toMs: 95000, phase: 'landed, asleep', by: 'asleep', reason: 'goodfix', reasonMeaning: 'enough good GPS fixes in a row', count: 1, heldFix: true },
    ],
    ranges: ranges({ gpsAlt: { min: 101.5, minMs: 1300, max: 140, maxMs: 30000, set: 12, heldFix: 18 } }), recordSpacing: spacing({ air: 50, ground: 5000 }),
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
const merged = (html) => html.slice(html.indexOf('class="pane an-merged"'), html.indexOf('data-bootpane="') < 0 ? undefined : html.indexOf('data-bootpane="'));
// what one row of the merged bar holds, boot by boot: [boot, flex-grow, the cell's markup]
const cells = (html, cls, label) => {
  const row = html.slice(html.indexOf('class="mrow ' + cls + '"><b>' + (label || '') + '</b>'));
  return [...row.slice(0, row.indexOf('</span></div>') + 13).matchAll(/data-mboot="(\d+)" style="flex-grow:(\d+)">(.*?)<\/span>(?=<span class="mcell"|<\/span><\/div>)/g)]
    .map((m) => [Number(m[1]), Number(m[2]), m[3]]);
};

test('every boot is in the table in chip order, the one with no records too, and the last ended with the pull', () => {
  const html = AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set(), now: NOW });
  const boots = html.slice(html.indexOf('class="pane an-boots"'));
  const rows = text(boots.slice(boots.indexOf('<tbody>'), boots.indexOf('</tbody>'))).match(/Boot \d/g);
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
  assert.match(text(merged(html)), /Boot 0 No phase events in this boot\. Firmware from before 2026-11-06 wrote none\./);
  const lane = cells(merged(html), 'mlane', 'Phase').find((c) => c[0] === 0)[2];
  assert.match(lane, /class="nophase"/);
  assert.doesNotMatch(lane, /class="seg/);
  assert.match(text(zero), /Record spacing, which is not a phase: 10\.0 s close together, 15\.0 s at a steady slower rate, 4\.0 s in gaps, changing 2 times/);
  assert.match(text(zero), /4\.0 s after up 21\.0 s · cause not in the log/);
  assert.match(text(zero), /gps .*clear on all 40/);
});

test('the phases are the phase events, and asleep is a held fix until the boot ends', () => {
  const html = AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set([2]), now: NOW });
  const two = paneOf(html, 2);
  const m = merged(html);
  const lane = cells(m, 'mlane', 'Phase').find((c) => c[0] === 2)[2];
  assert.equal((lane.match(/class="seg/g) || []).length, 3);
  assert.equal((lane.match(/class="tick"/g) || []).length, 1, 'imufast is a tick, not a phase');
  assert.match(text(m), /Boot 2 boot-air up 0 ms to 37\.0 s Every boot starts in boot-air\./);
  assert.match(text(m), /Boot 2 ground up 37\.0 s to 50\.0 s The IMU read no movement for long enough, with the altitude flat where the rule needs it\. bootairend, quiet/);
  assert.match(text(m), /Boot 2 landed, asleep, held fix up 50\.0 s to 95\.0 s Enough good GPS fixes in a row\. asleep, goodfix/);
  assert.match(text(m), /2 up 922 ms to 930 ms bus jam on imu, sdalow Right after a transfer with this part an I2C line was left held low, so this part is blamed\. Why: the data line SDA was low\. 2 address 0x4A the last time/);
  assert.doesNotMatch(two, /class="seg|System events/, 'the boot pane keeps no lane or event table of its own');
  assert.match(text(two), /While landed asleep the GPS fields are the fix it held/);
  assert.match(text(two), /18 readings on a held fix left out/);
  assert.match(text(two), /This boot ended when the board was restarted for the pull/);
});

test('the ticked boots make one bar, each as wide as it ran, with a line and how it ended between them', () => {
  const html = AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set(), now: NOW });
  const m = merged(html);
  assert.deepStrictEqual(cells(m, 'mlane', 'Phase').map((c) => c.slice(0, 2)), [[0, 30000], [1, 1], [2, 95000]], 'chip order, widths by uptime');
  assert.match(text(m), /Boot 0 · reset PIN, BOR Boot 1 · reset PIN Boot 2 · restarted for the pull/);
  assert.match(m, /data-pickboot="0" checked/);
  const only = merged(AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set(), picked: new Set([2, 0]), now: NOW }));
  assert.deepStrictEqual(cells(only, 'mlane', 'Phase').map((c) => c[0]), [0, 2]);
  assert.match(only, /data-pickboot="1">/);
  assert.doesNotMatch(text(only), /Boot 1 ·/);
  assert.deepStrictEqual([...only.matchAll(/<tr><td class="r">(\d+)<\/td>/g)].map((x) => x[1]), ['2', '2', '2', '2'], 'only boot 2 has events');
  const none = merged(AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set(), picked: new Set(), now: NOW }));
  assert.match(text(none), /No boot ticked\./);
  assert.doesNotMatch(none, /class="mrow/);
});

test('each sensor gets a strip on the same axis, empty where its flag was clear, and the IMU says when it was first valid', () => {
  const m = merged(AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set(), now: NOW }));
  const strips = [...m.matchAll(/class="mrow mstrip"><b>(\w+)<\/b>/g)].map((x) => x[1]);
  assert.deepStrictEqual(strips, ['accel', 'gyro', 'mag', 'quat', 'gps', 'soc', 'env']);
  const lane = cells(m, 'mlane', 'Phase').map((c) => c.slice(0, 2));
  for (const name of strips) {
    assert.deepStrictEqual(cells(m, 'mstrip', name).map((c) => c.slice(0, 2)), lane, name + ' lines up with the phases');
  }
  const accel = cells(m, 'mstrip', 'accel');
  assert.match(accel[2][2], /class="off" style="left:1\.37%;width:max\(2px,19\.05%\)"/, 'boot 2 accel clear from up 1.3 s for 18.1 s');
  assert.match(accel[1][2], /class="norec" style="left:0;width:100%"/, 'boot 1 has no records');
  const gps = cells(m, 'mstrip', 'gps');
  assert.match(gps[0][2], /class="off" style="left:3\.33%;width:max\(2px,96\.67%\)"/, 'boot 0 gps clear on all its records');
  assert.match(text(m), /Boot 0 IMU first valid at up 1\.0 s · GPS never had a fix in this boot Boot 1 no records Boot 2 IMU first valid at up 19\.4 s · GPS first fix at up 1\.0 s/);
  const never = Object.assign({}, PULL, { boots: [Object.assign({}, BOOTS[0], { imuFirstValidMs: null })] });
  assert.match(text(merged(AN.pullHtml(never, { item: ITEM, report: null, open: new Set(), now: NOW }))), /Boot 0 IMU never valid in this boot/);
});

test('the inventory lists every kind of data the chip keeps, over the boots ticked, with the flags and nothing judged', () => {
  const inv = (picked) => {
    const html = AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set(), picked, now: NOW });
    return text(html.slice(html.indexOf('class="pane an-inv"'), html.indexOf('class="pane an-boots"')));
  };
  const all = inv(new Set([0, 1, 2]));
  assert.match(all, /over the boots ticked under Phases and sensors: boots 0–2\./);
  const kinds = ['Linear acceleration, length of x y z', 'Rotation rate, length of x y z', 'Magnetic field, length of x y z', 'Orientation quaternion, i j k real',
    'Pressure', 'Temperature', 'Humidity', 'Barometer altitude', 'GPS latitude', 'GPS longitude', 'GPS altitude', 'Cell voltage', 'Packet counter, the last one sent',
    'Board id', 'All-fresh flag', 'GPS time anchors', 'Boot events, with the reset cause', 'Phase events', 'System events: sensor, bus, radio, flash', 'Dropped-record events', 'Settings'];
  let at = 0;
  for (const k of kinds) {
    const i = all.indexOf(k, at);
    assert.ok(i >= 0, k + ' is listed, in order');
    at = i;
  }
  assert.match(all, /Barometer altitude env 40 of 40 750 ms boot 0 up 1\.0 s to 30\.0 s boot 2 up 1\.0 s to 30\.0 s 233\.31 m boot 0, up 1\.0 s 251\.2 m boot 0, up 9\.0 s/);
  assert.match(all, /GPS altitude gps 30 of 70 18 on a held fix, left out of lowest and highest 750 ms boot 2 up 1\.0 s to 30\.0 s 101\.5 m boot 2, up 1\.3 s 140 m boot 2, up 30\.0 s/);
  assert.match(all, /Packet counter, the last one sent — 0 50 ms in the air 5\.0 s on the ground 750 ms with no phase events boot 0 up 1\.0 s to 30\.0 s boot 2 up 1\.3 s to 95\.0 s/);
  assert.match(all, /GPS time anchors — 3 20\.0 s boot 2 up 20\.0 s to 50\.0 s 2 used for the clock, 1 rejected/);
  assert.match(all, /Boot events, with the reset cause — 2 of 3 boots — boots 0, 2 PIN\+SFT ×1, PIN ×1/);
  assert.match(all, /Phase events — 2 2 times — boot 2 up 37\.0 s to 50\.0 s bootairend ×1, asleep ×1/);
  assert.match(all, /System events: sensor, bus, radio, flash — 1 2 times — boot 2 up 922 ms to 930 ms bus jam ×2/);
  assert.match(all, /Settings — 1 line — the restart for the pull from=block csid=15 module=hp sf=7 dbm=14 fixedms=5000,0/);
  assert.match(all, /Not in the record: the satellite count, the fix type, GPS speed and heading, and the state of charge in percent\./);
  const one = inv(new Set([0]));
  assert.match(one, /boot 0\./);
  assert.match(one, /GPS altitude gps 0 of 40 — none — —/, 'boot 0 never had the gps flag set');
  assert.match(inv(new Set()), /No boot ticked\./);
});

test('a chart keeps records with the flag set, breaks at a clear one, a gap and the boot end, and keeps a pixel\'s lowest and highest', () => {
  const b = {
    n: 3, lastMs: 1000, phases: [], events: [], gaps: [{ afterMs: 600, ms: 300 }],
    track: { ms: [0, 10, 20, 30, 40, 100, 500, 600, 900, 1000], valid: [64, 64, 64, 64, 64, 64, 0, 64, 64, 64], envAlt: [5, 9, 1, 7, 3, 4, 99, 6, 2, 8] },
  };
  const p = AN.chartPoints([{ b, left: 0, width: 10 }], { flag: 6, keys: ['envAlt'] });
  assert.deepStrictEqual(p.x, [0.1, 0.2, 1, 5, 6, 9, 9, 10, 10]);
  assert.deepStrictEqual(p.ys, [[9, 1, 4, null, 6, null, 2, 8, null]], 'the 99 has its flag clear and is not drawn');
  assert.deepStrictEqual(p.at.slice(0, 2), [[3, 10], [3, 20]], 'the highest came first in that pixel');
  const imu = { n: 4, lastMs: 100, phases: [], events: [], gaps: [], track: { ms: [0, 100], valid: [1, 1], ax: [3, 0], ay: [4, 0], az: [0, 2] } };
  assert.deepStrictEqual(AN.chartPoints([{ b: imu, left: 0, width: 100 }], { flag: 0, keys: ['ax', 'ay', 'az'], length: true }).ys, [[5, 2, null]]);
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

test('a tick takes a boot in or out of the merged bar, and all and none do every boot', async () => {
  const p = await screen();
  const shown = () => cells(merged(p.html('an-sheet')), 'mlane', 'Phase').map((c) => c[0]);
  assert.deepStrictEqual(shown(), [0, 1, 2], 'every boot to start with');
  await p.press({ 'data-pickboot': '1' });
  assert.deepStrictEqual(shown(), [0, 2]);
  await p.press({ 'data-pickall': '0' });
  assert.deepStrictEqual(shown(), []);
  await p.press({ 'data-pickboot': '2' });
  assert.deepStrictEqual(shown(), [2]);
  await p.press({ 'data-pickall': '1' });
  assert.deepStrictEqual(shown(), [0, 1, 2]);
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

test('the flight picker offers each boot with a track, whole and by air spell, the first air spell first', () => {
  const track = (ms) => ({ every: 1, ms, valid: ms.map(() => 127) });
  const flown = Object.assign({}, BOOTS[2], {
    n: 7, firstMs: 2134, lastMs: 296309, track: track([2134, 296309]),
    phases: [
      { fromMs: 0, toMs: 200817, phase: 'boot-air' }, { fromMs: 200817, toMs: 228005, phase: 'air' },
      { fromMs: 228005, toMs: 266099, phase: 'air, falling' }, { fromMs: 266099, toMs: 296309, phase: 'landed, looking for a fix' },
    ],
  });
  const ground = Object.assign({}, BOOTS[0], { n: 3, track: track([1000, 30000]) });
  const pull = Object.assign({}, PULL, { boots: [ground, BOOTS[1], flown] });
  const c = AN.flightChoices(pull);
  assert.deepStrictEqual(c.list.map((f) => f.label), ['Boot 7, the whole boot', 'Boot 7, air spell 1', 'Boot 3, the whole boot'], 'newest boot first, boot 1 has no track');
  assert.strictEqual(c.first.value, '7:1');
  assert.match(AN.pullHtml(pull, { item: ITEM, report: null, open: new Set(), now: NOW }), /an-flightslot/);
  assert.doesNotMatch(AN.pullHtml(PULL, { item: ITEM, report: null, open: new Set(), now: NOW }), /an-flightslot/, 'no track, no flight view');
});
