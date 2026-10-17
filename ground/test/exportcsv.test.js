const test = require('node:test');
const assert = require('node:assert');
const P = require('../web/js/packet.js');
const X = require('../web/js/exportcsv.js');

// The real packet test/packet.test.js uses. Its validity is 0x6F: no GPS, not all fresh.
const BASE = P.decode(P.hexToBytes('00 00 00 00 00 00 00 00 98 BD FF FF 23 5B 00 00 FD FF 00 00 FD FF 01 00 FF FF FF FF AC FF 18 FF D4 FD 4A 06 8E 02 ED 82 56 1A 7A 0A 7F 26 0F 11 04 00 09 6D 6F 09 23')).values;

const HEADER = [
  'time_ms', 'time_utc', 'source', 'boot', 'uptime_ms', 'csid', 'counter', 'validity',
  'lat', 'lon', 'gps_alt_m', 'baro_alt_m', 'ax', 'ay', 'az', 'gx', 'gy', 'gz', 'mx', 'my', 'mz',
  'qi', 'qj', 'qk', 'qr', 'temp_c', 'pressure_hpa', 'humidity', 'cell_v',
  'rssi', 'snr', 'freq_err',
];

// That packet re-stamped, with a correct CRC. tail is what a receiver adds after the hex;
// a chip log leaves all three empty.
function pkt(over, tail) {
  const v = Object.assign({}, BASE, over);
  const crc = P.crc16(P.encode(v, 0), 53);
  return 'PKT,55,' + P.bytesToHex(P.encode(v, crc)) + (tail || ',,,');
}

function table(events) {
  const lines = X.toCsv(events).replace(/\n$/, '').split('\n');
  const head = lines[0].split(',');
  const rows = lines.slice(1).map((l) => Object.fromEntries(l.split(',').map((c, i) => [head[i], c])));
  return { lines, head, rows };
}

test('a radio log row carries its time, receiver and link numbers', () => {
  const t = 1790000000000;
  const { head, rows } = table([
    { t: t - 500, rx: 'rx1', text: '#DG,RX,v1,id=A1B2C3,f=915.0,bw=125.0,sf=9' },
    { t, rx: 'rx1', text: pkt({ counter: 12, csid: 3, validity: 0x7f, lat: 42.4440001, lon: -76.5019 }, ',-50.0,13.75,-1234') },
    { t: t + 100, rx: 'rx1', text: 'HB,5000,1,0' },
  ]);
  assert.deepStrictEqual(head, HEADER);
  assert.strictEqual(rows.length, 1);
  const r = rows[0];
  assert.strictEqual(r.time_ms, String(t));
  assert.strictEqual(r.time_utc, new Date(t).toISOString());
  assert.strictEqual(r.source, 'rx1');
  assert.strictEqual(r.boot, '');
  assert.strictEqual(r.uptime_ms, '');
  assert.strictEqual(r.csid, '3');
  assert.strictEqual(r.counter, '12');
  assert.strictEqual(r.validity, '7F');
  assert.strictEqual(r.lat, '42.4440001');
  assert.strictEqual(r.lon, '-76.5019');
  assert.strictEqual(Number(r.ax), BASE.ax);
  assert.strictEqual(Number(r.temp_c), BASE.temp);
  assert.strictEqual(Number(r.pressure_hpa), BASE.pressure);
  assert.strictEqual(Number(r.cell_v), BASE.battery);
  assert.strictEqual(Math.round(Number(r.qr) * 32767), Math.round(BASE.qr * 32767), 'a quaternion term survives the rounding');
  assert.strictEqual(r.rssi, '-50');
  assert.strictEqual(r.snr, '13.75');
  assert.strictEqual(r.freq_err, '-1234');
});

test('a flash log: the boot with no GPS time has no UTC, both have uptime', () => {
  const anchored = 1790000000000;
  // The two boots are interleaved on purpose, so a row has to take its boot and its
  // clock from its own source and not from whichever line came last.
  const { rows } = table([
    { t: 1000, rx: 'flash-boot0', text: '#DG,SRC,v1,kind=flash,from=dump.txt,boot=0,packets=2,time_source=none,uptime0_ms=-1517' },
    { t: 1000, rx: 'flash-boot0', text: '#DG,BOOT,v1,n=0' },
    { t: anchored, rx: 'flash-boot1', text: '#DG,SRC,v1,kind=flash,from=dump.txt,boot=1,packets=1,time_source=anchor,anchor_uptime_ms=41230,anchor_unix_ms=1790000038730,anchor_utc=2026-09-21T14:13:58.730Z,uptime0_ms=1789999997500' },
    { t: anchored, rx: 'flash-boot1', text: '#DG,BOOT,v1,n=1' },
    { t: 1000, rx: 'flash-boot0', text: pkt({ counter: 1 }) },
    { t: anchored, rx: 'flash-boot1', text: pkt({ counter: 7 }) },
    { t: 1050, rx: 'flash-boot0', text: pkt({ counter: 1, ax: 1.5 }) },
  ]);
  assert.deepStrictEqual(rows.map((r) => r.source), ['flash-boot0', 'flash-boot1', 'flash-boot0']);
  assert.deepStrictEqual(rows.map((r) => r.boot), ['0', '1', '0']);
  assert.deepStrictEqual(rows.map((r) => r.time_utc), ['', new Date(anchored).toISOString(), '']);
  assert.deepStrictEqual(rows.map((r) => r.time_ms), ['1000', String(anchored), '1050']);
  assert.deepStrictEqual(rows.map((r) => r.uptime_ms), ['2517', '2500', '2567']);
  // Nothing was received, so there is no link to report.
  assert.deepStrictEqual(rows.map((r) => r.rssi + r.snr + r.freq_err), ['', '', '']);
});

test('a source line without uptime0_ms leaves uptime blank', () => {
  const { rows } = table([
    { t: 5000, rx: 'flash-boot3', text: '#DG,SRC,v1,kind=flash,from=dump.txt,boot=3,packets=1,time_source=none' },
    { t: 5000, rx: 'flash-boot3', text: '#DG,BOOT,v1,n=3' },
    { t: 5000, rx: 'flash-boot3', text: pkt({}) },
  ]);
  assert.strictEqual(rows[0].uptime_ms, '');
  assert.strictEqual(rows[0].boot, '3');
  assert.strictEqual(rows[0].time_utc, '');
});

test('a field whose validity bit is clear is an empty cell', () => {
  const { rows } = table([
    { t: 1, rx: 'rx1', text: pkt({ validity: 0x6f, lat: 42.5, lon: -76.5, gpsAlt: 120 }, ',-90.0,5.00,0') },
    { t: 2, rx: 'rx1', text: pkt({ validity: 0x7f, lat: 42.5, lon: -76.5, gpsAlt: 120 }, ',-90.0,5.00,0') },
    { t: 3, rx: 'rx1', text: pkt({ validity: 0x7f & ~0x20, battery: 3.8 }, ',-90.0,5.00,0') },
  ]);
  assert.strictEqual(rows[0].validity, '6F');
  assert.deepStrictEqual([rows[0].lat, rows[0].lon, rows[0].gps_alt_m], ['', '', '']);
  assert.notStrictEqual(rows[0].ax, '', 'the accelerometer bit is set, so its value stays');
  assert.notStrictEqual(rows[0].temp_c, '');
  assert.deepStrictEqual([rows[1].lat, rows[1].lon, rows[1].gps_alt_m], ['42.5', '-76.5', '120']);
  assert.strictEqual(rows[2].cell_v, '');
  assert.strictEqual(rows[1].cell_v, String(BASE.battery));
});

test('a line that fails its CRC, or is not a packet, is not exported', () => {
  const good = pkt({ counter: 5 }, ',-90.0,5.00,0');
  const hex = good.split(',')[2];
  const flipped = (hex[0] === '0' ? '1' : '0') + hex.slice(1);
  const { rows } = table([
    { t: 1, rx: 'rx1', text: 'PKT,55,' + flipped + ',-90.0,5.00,0' },
    { t: 2, rx: 'rx1', text: 'PKT,3,010203,-90.0,1.00,0' },
    { t: 3, rx: 'rx1', text: 'ERR,-7,-120.5,-12.25' },
    { t: 4, rx: 'rx1', text: 'Radio receive failed. RadioLib code -24' },
    { t: 5, rx: 'rx1', text: good },
  ]);
  assert.strictEqual(rows.length, 1);
  assert.strictEqual(rows[0].time_ms, '5');
  assert.strictEqual(rows[0].counter, '5');
});

test('every row has as many cells as the header', () => {
  const { lines, head } = table([
    { t: 1790000000000, rx: 'rx1', text: pkt({ validity: 0x7f }, ',-50.0,13.75,-12') },
    { t: 1790000000100, rx: 'rx2', text: pkt({ validity: 0x00 }, ',-120.5,-3.25,') },
    { t: 1000, rx: 'flash-boot0', text: '#DG,SRC,v1,kind=flash,from=dump.txt,boot=0,packets=1,time_source=none,uptime0_ms=0' },
    { t: 1000, rx: 'flash-boot0', text: '#DG,BOOT,v1,n=0' },
    { t: 1000, rx: 'flash-boot0', text: pkt({}) },
  ]);
  assert.strictEqual(lines.length, 4);
  assert.strictEqual(head.length, new Set(head).size, 'no column name twice');
  for (const l of lines) assert.strictEqual(l.split(',').length, HEADER.length, l);
});

test('a receiver name with a comma in it is quoted, and nothing else is', () => {
  const csv = X.toCsv([{ t: 1, rx: 'lab, bench "A"', text: pkt({}, ',-90.0,5.00,0') }]);
  const row = csv.split('\n')[1];
  assert.ok(row.startsWith('1,' + new Date(1).toISOString() + ',"lab, bench ""A""",'), row);
  assert.strictEqual(row.split('"').length - 1, 6, 'only that one cell is quoted');
});

test('nothing to export is still a file with a header', () => {
  assert.strictEqual(X.toCsv([]), HEADER.join(',') + '\n');
});

test('a row that says its own uptime is believed over the boot start', () => {
  // A long boot is dated from the anchors around each record, so its dates slide and
  // time less the boot's start is only near the uptime. The row's own figure is exact.
  const { rows } = table([
    { t: 1790000000000, rx: 'flash-boot6', text: '#DG,SRC,v1,kind=flash,from=dump.txt,boot=6,packets=1,time_source=anchor,uptime0_ms=1789999990000' },
    { t: 1790000000000, rx: 'flash-boot6', text: '#DG,BOOT,v1,n=6' },
    { t: 1790000000000, rx: 'flash-boot6', text: pkt({}, ',,,,up=10075,n=41') },
  ]);
  assert.strictEqual(rows[0].uptime_ms, '10075');
  assert.strictEqual(rows[0].boot, '6');
});
