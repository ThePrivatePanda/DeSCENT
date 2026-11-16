// What the flight view draws from a run's packets or a boot's flash records. The drop below is
// made up in the shape of board 17's on 2026-11-08: a minute on the ground, carried up, a tumbling
// fall of about 12 s, then lying still near 42.457705, -76.384733.
const test = require('node:test');
const assert = require('node:assert');
const F = require('../web/js/flight.js');

const LAT = 42.457705;
const LON = -76.384733;

// One radio history point a second, the way stream.js keeps them: a field null where its flag was clear
function history(points) {
  const keys = ['t', 'counter', 'lat', 'lon', 'gpsAlt', 'envAlt', 'ax', 'ay', 'az', 'gx', 'gy', 'gz', 'qi', 'qj', 'qk', 'qr'];
  const h = Object.fromEntries(keys.map((k) => [k, []]));
  for (const p of points) for (const k of keys) h[k].push(p[k] === undefined ? null : p[k]);
  return h;
}

function drop() {
  const pts = [];
  let c = 0;
  const add = (t, o) => pts.push(Object.assign({ t, counter: c++, qi: 0, qj: 0, qk: 0, qr: 32767 / 32767 }, o));
  for (let t = 0; t < 60; t++) add(t, { lat: LAT - 0.0003, lon: LON - 0.0002, gpsAlt: 250, envAlt: 233, ax: 0, ay: 0, az: 0.05, gx: 0.5, gy: 0, gz: 0 });
  for (let t = 60; t < 120; t++) add(t, { lat: LAT - 0.0003, lon: LON - 0.0002, gpsAlt: 250 + (t - 60), envAlt: 233 + (t - 60), ax: 0.6, ay: 0, az: 0, gx: 5, gy: 0, gz: 0 });
  for (let t = 120; t < 132; t++) add(t, { lat: LAT, lon: LON, gpsAlt: 310 - 5 * (t - 120), envAlt: 293 - 5 * (t - 120), ax: 3, ay: 0, az: 0, gx: t === 120 ? 200 : 600, gy: 0, gz: 0 });
  for (let t = 132; t < 200; t++) add(t, { lat: LAT + (t % 2 ? 0.00001 : -0.00001), lon: LON, gpsAlt: 252, envAlt: 233.5, ax: 0.1, ay: 0, az: 0, gx: 0.3, gy: 0, gz: 0 });
  return pts;
}

test('a counter heard twice is one sample, at its first receipt', () => {
  const pts = drop().slice(0, 3);
  pts.splice(2, 0, Object.assign({}, pts[1], { t: 1.9 }));
  const s = F.fromHistory(history(pts));
  assert.deepStrictEqual(s.map((x) => [x.t, x.counter]), [[0, 0], [1, 1], [2, 2]]);
  const restarted = F.fromHistory(history([{ t: 0, counter: 0 }, { t: 1, counter: 1 }, { t: 300, counter: 0 }]));
  assert.strictEqual(restarted.length, 3, 'the same counter minutes later is the board starting again');
});

test('a counter jump is a break with the count missed, and the trail does not cross it', () => {
  const pts = drop().slice(0, 10).filter((p) => p.counter !== 4 && p.counter !== 5);
  pts[3].lat += 0.0001;
  const s = F.fromHistory(history(pts));
  const gaps = F.counterGaps(s);
  assert.deepStrictEqual(gaps.map((g) => [g.i, g.missed]), [[4, 2]]);
  const tr = F.trail(s, gaps, null);
  assert.strictEqual(tr.dots.length, 8, 'every fix is a dot');
  assert.deepStrictEqual(tr.lines.map((l) => l.length), [4, 4], 'dashed up to the gap and after it, not across');
  assert.strictEqual(tr.marks[0].words, '2 missed');
  assert.deepStrictEqual(F.counterGaps(F.fromHistory(history([{ t: 0, counter: 65535 }, { t: 1, counter: 0 }]))), [], 'the counter wraps');
});

test('only fixes with the GPS flag are drawn, and 0, 0 is counted rather than drawn', () => {
  const pts = drop().slice(0, 4);
  pts[1].lat = null; pts[1].lon = null;   // flag clear: the history holds null
  pts[2].lat = 0; pts[2].lon = 0;
  const s = F.fromHistory(history(pts));
  assert.strictEqual(F.trail(s, [], null).dots.length, 2);
  assert.strictEqual(F.zeroFixes(s), 1);
});

test('heights are above the boot\'s first reading of the chosen source, never mixed', () => {
  const s = F.fromHistory(history(drop()));
  const baro = F.heights(s, 'baro');
  const gps = F.heights(s, 'gps');
  assert.strictEqual(baro.base, 233);
  assert.strictEqual(gps.base, 250);
  assert.strictEqual(baro.of(s[119]), 59);
  assert.strictEqual(gps.of(s[119]), 59);
  assert.strictEqual(F.heights(s.map((x) => Object.assign({}, x, { gpsAlt: null, ok: Object.assign({}, x.ok, { gps: false }) })), 'gps').base, null);
});

test('touchdown is the start of the last 20 s lying still after moving, and release the first big turn before it', () => {
  const s = F.fromHistory(history(drop()));
  const td = F.touchdown(s);
  assert.strictEqual(td.t, 132);
  assert.strictEqual(F.release(s, td).t, 121, 'half the 600 deg/s peak, not the 200 at 120');
  const bench = F.fromHistory(history(drop().slice(0, 60)));
  assert.strictEqual(F.touchdown(bench), null, 'a board that never moved has not landed');
  const short = F.fromHistory(history(drop().slice(0, 150)));
  assert.strictEqual(F.touchdown(short), null, '18 s still is not yet a touchdown');
});

test('the find spot is the median of the minute after touchdown, with its spread and its rule', () => {
  const s = F.fromHistory(history(drop()));
  const spot = F.findSpot(s, F.touchdown(s));
  assert.strictEqual(spot.n, 61);
  // 31 fixes 1.1 m south and 30 as far north: the median is one of the southern ones
  assert.ok(Math.abs(spot.lat - (LAT - 0.00001)) < 1e-9 && spot.lon === LON);
  assert.strictEqual(spot.half, 0);
  assert.ok(Math.abs(spot.all - 2.2264) < 0.001, String(spot.all));
  assert.match(spot.rule, /^median of the 61 fixes in the first 60 s after touchdown$/);
  const none = F.findSpot(F.fromHistory(history(drop().slice(0, 100))), null);
  assert.match(none.rule, /up to the last fix, no touchdown found$/);
});

test('distance and compass bearing from a station', () => {
  const north = F.metres(LAT, LON, LAT + 0.001, LON);
  assert.ok(Math.abs(north - 111.32) < 0.01);
  assert.strictEqual(Math.round(F.bearing(LAT, LON, LAT + 0.001, LON)), 0);
  assert.strictEqual(Math.round(F.bearing(LAT, LON, LAT, LON + 0.001)), 90);
  assert.strictEqual(F.compass(F.bearing(LAT, LON, LAT - 0.001, LON - 0.001)), 'SW');
  assert.strictEqual(F.compass(359), 'N');
});

test('the quaternion is normalised and the world frame is z up', () => {
  const level = F.orientation([0.99, 0, 0, 0]);
  assert.deepStrictEqual(level.up.map((v) => Math.round(v * 1000) / 1000), [0, 0, 1], 'flat board: world up is the board z');
  assert.strictEqual(Math.round(level.tilt), 0);
  assert.strictEqual(Math.round(level.heading), 90, 'board x along world x, east');
  // a quarter turn about z: board x along world y, magnetic north
  const c = Math.SQRT1_2;
  assert.strictEqual(Math.round(F.orientation([c, 0, 0, c]).heading), 0);
  // a half turn about x: upside down
  assert.strictEqual(Math.round(F.orientation([0, 1, 0, 0]).tilt), 180);
  assert.strictEqual(F.orientation([0, 0, 0, 0]), null);
  assert.strictEqual(F.orientation(null), null);
});

test('derived speeds say over how long they were taken', () => {
  const s = F.fromHistory(history(drop()));
  const v = F.vspeed(s, 125, F.heights(s, 'baro'));
  assert.deepStrictEqual(v, { v: -5, dt: 1 });
  const hz = F.hspeed(s, 120);
  assert.strictEqual(hz.dt, 1);
  assert.ok(Math.abs(hz.v - F.metres(LAT - 0.0003, LON - 0.0002, LAT, LON)) < 1e-6);
  assert.strictEqual(F.at(s, 125.5), 125);
  assert.strictEqual(F.at(s, -1), -1);
});

test('a flash track keeps its flags: a held fix is marked and a flag clear is clear', () => {
  const tr = {
    every: 1, ms: [1000, 1050, 1100], valid: [127, 111, 127 & ~64],
    lat: [LAT, null, LAT], lon: [LON, null, LON], gpsAlt: [250, null, 250], envAlt: [233.3, 233.4, 999],
    qi: [0, 0, 0], qj: [0, 0, 0], qk: [0, 0, 0], qr: [1, 1, 1], gx: [0, 0, 0], gy: [0, 0, 0], gz: [0, 0, 0], ax: [0, 0, 0], ay: [0, 0, 0], az: [0, 0, 0],
  };
  const s = F.fromTrack(tr, [[1100, 2000]]);
  assert.deepStrictEqual(s.map((x) => x.ok.gps), [true, false, true]);
  assert.deepStrictEqual(s.map((x) => x.held), [false, false, true]);
  assert.strictEqual(s[2].ok.env, false);
  assert.strictEqual(s[2].baroAlt, 999, 'the value as logged, its flag beside it');
  const h = F.heights(s, 'baro');
  assert.strictEqual(F.vspeed(s, 2, h), null, 'no derived speed from a reading whose flag is clear');
  assert.deepStrictEqual(F.trackGaps(s, [{ afterMs: 1050, ms: 4000 }]), [{ i: 2, words: 'gap 4.0 s' }]);
  assert.deepStrictEqual(F.trail(s, [], null).dots.map((d) => d.held), [false, true]);
});

test('a boot is one flight and each air spell another, from its phase events', () => {
  const b = {
    n: 7, firstMs: 2134, lastMs: 296309,
    phases: [
      { fromMs: 0, toMs: 200817, phase: 'boot-air' },
      { fromMs: 200817, toMs: 228005, phase: 'air' },
      { fromMs: 228005, toMs: 266099, phase: 'air, falling' },
      { fromMs: 266099, toMs: 296309, phase: 'landed, looking for a fix' },
    ],
  };
  const f = F.flightsOf(b);
  assert.strictEqual(f.length, 2);
  assert.deepStrictEqual([f[0].kind, f[0].fromMs, f[0].toMs], ['boot', 2134, 296309]);
  assert.deepStrictEqual([f[1].toAirMs, f[1].releaseMs, f[1].landedMs, f[1].fromMs, f[1].toMs], [200817, 228005, 266099, 170817, 296309]);
  assert.strictEqual(F.flightsOf({ n: 1, firstMs: 0, lastMs: 10, phases: [] }).length, 1, 'no phase events, only the whole boot');
});
