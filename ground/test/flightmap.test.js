// The flight view's markup and its tile arithmetic. The map itself needs WebGL and is looked at
// in a browser.
const test = require('node:test');
const assert = require('node:assert');
const M = require('../web/js/flightmap.js');

const LAT = 42.457705;
const LON = -76.384733;
const AROUND = { west: LON - 0.006, east: LON + 0.006, north: LAT + 0.0045, south: LAT - 0.0045 };

test('the tiles a save asks for cover the view at every zoom it saves', () => {
  // counts worked out separately in Python for this box: 2 at z14, 4 at z15, 9 at z16
  const z16 = M.tilesFor(AROUND, 'imagery-usgs', 16, 16);
  assert.strictEqual(z16.length, 9);
  assert.ok(z16.some((t) => t.x === 18862 && t.y === 24215), 'the drop site\'s own tile');
  assert.strictEqual(M.tilesFor(AROUND, 'imagery-usgs', 14, 16).length, 2 + 4 + 9);
});

test('a save says its size from the tiles already saved, or from typical sizes and says so', () => {
  const typical = M.saveEstimate(AROUND, null);
  assert.strictEqual(typical.measured, false);
  assert.deepStrictEqual(typical.parts.map((p) => p.layer), ['imagery-usgs', 'terrain'], 'never Esri');
  const im = typical.parts[0];
  assert.strictEqual(im.bytes, im.tiles.length * 22000);
  assert.ok(im.tiles.every((t) => t.z >= 14 && t.z <= 16));
  assert.ok(typical.parts[1].tiles.every((t) => t.z >= 10 && t.z <= 15));
  const stored = { layers: { 'imagery-usgs': { tiles: 10, bytes: 300000 }, terrain: { tiles: 2, bytes: 100000 } } };
  const measured = M.saveEstimate(AROUND, stored);
  assert.strictEqual(measured.measured, true);
  assert.strictEqual(measured.parts[0].bytes, measured.parts[0].tiles.length * 30000);
  assert.strictEqual(M.mb(1234567), '1.2 MB');
});

const CARD = {
  key: '17', name: 'CSID 17', colour: '#2a78d6', why: 'down', td: { t: 1794159592.7 },
  spot: { lat: LAT, lon: LON, n: 61, half: 1.4, all: 4.2, rule: 'median of the 61 fixes in the first 60 s after touchdown' },
  station: { lat: LAT - 0.001, lon: LON, from: 'set on the map' }, lastT: 1794159700, now: 1794159730, battery: 3.91,
  rx: [{ name: 'SF12', rssi: -97.5, snr: 6.25 }, { name: 'LNA SF10', rssi: -110, snr: -2 }],
};

test('a Find card says where to walk, from where, and that touchdown is derived', () => {
  const html = M.findCardHtml(CARD);
  assert.match(html, /CSID 17/);
  assert.match(html, /on the ground/);
  assert.match(html, /derived: IMU still \(under 0\.3 m\/s², 2 deg\/s\) for 20 s/);
  assert.match(html, /42\.457705, -76\.384733/);
  assert.match(html, /half within 1\.4 m, all within 4\.2 m/);
  assert.match(html, /<b>111 m at 0° N<\/b>/, 'the station is 0.001 degrees south');
  assert.match(html, /station set on the map/);
  assert.match(html, /href="geo:42\.457705,-76\.384733"/);
  assert.match(html, /data-copy="42\.457705, -76\.384733"/);
  assert.match(html, /SF12 <b>-98<\/b> dBm, 6\.3 dB<br>LNA SF10 <b>-110<\/b> dBm/);
  assert.match(html, /<b>3\.91<\/b> V/);
  assert.match(html, /30 s ago/);
});

test('a silent board with no fix and no station says so rather than guessing', () => {
  const html = M.findCardHtml(Object.assign({}, CARD, { why: 'silent', td: null, spot: null, station: null, battery: null, rx: [] }));
  assert.match(html, /silent 30 s/);
  assert.match(html, /none found: no 20 s still after moving/);
  assert.match(html, /no GPS fix with its flag set/);
  assert.match(html, /flag clear/);
  assert.doesNotMatch(html, /geo:/);
});

test('the readout gives each value with its flag, its age and how a derived one was made', () => {
  const html = M.readoutHtml({
    when: 'up 228.0 s, 12:19:40.1', source: 'baro',
    height: { v: 58.04, ok: false }, vs: { v: -5.2, dt: 1.05 }, hs: null,
    fix: { lat: LAT, lon: LON, held: true }, o: { tilt: 92, heading: null, ok: true }, gyro: { v: 612, ok: true },
    ages: { height: 0, fix: 0.4, quat: 0.05 },
  });
  assert.match(html, /Barometer height/);
  assert.match(html, /<b>58\.0<\/b> m <span class="note">flag clear<\/span>/);
  assert.match(html, /derived: height difference over 1\.05 s/);
  assert.match(html, /Ground speed<\/span><span class="fv-val">—/);
  assert.match(html, /held fix/);
  assert.match(html, /0\.40 s before/);
  assert.match(html, /Magnetic heading<\/span><span class="fv-val">—/, 'no heading with the board on its edge');
  assert.match(html, /<b>612<\/b> deg\/s/);
  assert.match(M.readoutHtml(null), /No sample at or before this time/);
});

// A run in the history shape stream.js keeps: t s, a field null where its flag was clear
function run(t0, n, o) {
  const F = require('../web/js/flight.js');
  const keys = ['t', 'counter', 'lat', 'lon', 'gpsAlt', 'envAlt', 'ax', 'ay', 'az', 'gx', 'gy', 'gz', 'qi', 'qj', 'qk', 'qr'];
  const h = Object.fromEntries(keys.map((k) => [k, []]));
  for (let i = 0; i < n; i++) {
    const p = Object.assign({ t: t0 + i, counter: i, ax: 0, ay: 0, az: 0.05, gx: 0.5, gy: 0, gz: 0 }, o(i));
    for (const k of keys) h[k].push(p[k] === undefined ? null : p[k]);
  }
  return { samples: F.fromHistory(h), firstT: t0 };
}

test('a board that restarted with no fix is found by the landing before the restart, and the card says so', () => {
  // carried, then lying still with fixes; then a brown-out restart on the ground with no fix
  const landed = run(1000, 120, (i) => Object.assign({ lat: LAT, lon: LON }, i < 30 ? { ax: 1, gx: 20 } : {}));
  const after = run(1500, 10, () => ({}));
  const l = M.landing({ samples: after.samples, firstT: after.firstT, before: [landed] });
  assert.strictEqual(l.td.t, 1030);
  assert.strictEqual(l.spot.lat, LAT);
  assert.deepStrictEqual(l.restart, { t: 1500, fixes: 0 });
  const html = M.findCardHtml(Object.assign({}, CARD, { td: l.td, spot: l.spot, restart: l.restart }));
  assert.match(html, /Restarted<\/dt><dd><b>[0-9:]{8}<\/b> <span class="note">after this, in a new run with no fix since/);
  // the same board with fixes again after the restart: those are newer, and the card says how many
  const moved = run(1500, 90, () => ({ lat: LAT + 0.001, lon: LON }));
  const m = M.landing({ samples: moved.samples, firstT: moved.firstT, before: [landed] });
  assert.strictEqual(m.spot.lat, LAT + 0.001);
  assert.match(m.spot.rule, /up to the last fix, none after touchdown/);
  assert.strictEqual(m.restart, null, 'the fixes are from the run transmitting now');
  assert.strictEqual(M.landing({ samples: landed.samples, firstT: 1000 }).restart, null);
});
