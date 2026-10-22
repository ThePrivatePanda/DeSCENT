// bench.js, lines in the formats fsw/V2_6_X_Tests/bench/sensor_bench prints
const test = require('node:test');
const assert = require('node:assert');
const B = require('../web/js/bench.js');
const P = require('../web/js/packet.js');

const BOOT = [
  'I BOOT 12 c0: start test=bench module=le csid=254 sf=7 dbm=10 txoffms=30000',
  'I RST 13 c0: cause csr=0x0C000000 lpwr=0 wwdg=0 iwdg=0 sft=0 bor=0 pin=1 obl=0 rfilar=0',
  'I BOOT 40 c0: scan found=0x36,0x4A,0x77 n=3 noack=109 timeouts=0 errors=0 stopped=0',
  'I FLASH 41 c0: init ok=1 id=0xEF4015',
  'I IMU 300 c0: init ok=1 sh2=1 attempts=1 ms=250',
  'E GPS 900 c0: init ok=0 reason=config attempts=3',
  'I SOC 910 c0: init ok=1 ver=0x0012 powerup=0',
  'I ENV 920 c0: init ok=1 ms=8',
  'I RADIO 930 c0: init ok=1 code=0',
  'I RADIO 931 c0: tx on=0 why=start',
  'I BOOT 932 c0: setup done=1 watchdogms=5000',
];

function fed(lines, t0 = 1000, step = 10) {
  const s = new B.Session();
  lines.forEach((l, i) => s.add(t0 + i * step, l));
  return s;
}

test('a line is split into its words and its values are kept as printed', () => {
  const l = B.parse('I ENV 1234 c5: bench n=7 ok=1 status=ok valid=1 temp=21.40 phpa=1012.07 rh=40.10 altm=10.2');
  assert.deepStrictEqual([l.level, l.tag, l.ms, l.cycle, l.event], ['I', 'ENV', 1234, 5, 'bench']);
  assert.strictEqual(l.fields.temp, '21.40', 'text, not a number: the trailing zero is what the board printed');
  assert.strictEqual(l.fields.phpa, '1012.07');
  const pkt = B.parse('I PKT 2000 c6: 00 01 02 FE');
  assert.strictEqual(pkt.hex, '00 01 02 FE');
  assert.strictEqual(B.parse('garbage \u0000 from booting'), null);
});

test('a clean start: each part with what its init line said, and the start with its cause', () => {
  const s = fed(BOOT);
  assert.strictEqual(s.boots.length, 1);
  assert.strictEqual(s.bench, true);
  assert.strictEqual(s.boots[0].cause.pin, '1');
  assert.strictEqual(s.parts.get('FLASH').init.id, '0xEF4015');
  assert.strictEqual(s.parts.get('GPS').init.ok, '0');
  assert.strictEqual(s.parts.get('GPS').lastFail.reason, 'config');
  assert.strictEqual(s.parts.get('GPS').errors, 1);
  assert.strictEqual(s.parts.get('IMU').lastOk, true);
  assert.ok(!s.parts.has('BOOT') && !s.parts.has('RST'), 'bookkeeping is not a part');
});

test('a failing read keeps its reason, and the count and rate come from the board', () => {
  const lines = [...BOOT];
  for (let i = 1; i <= 20; i++) {
    lines.push('I ENV ' + (1000 + i * 200) + ' c1: bench n=' + i + ' ok=1 status=ok valid=1 temp=21.40 phpa=1012.07 rh=40.10 altm=10.2');
  }
  lines.push('W ENV 5150 c5: read ok=0 reason=bus');
  lines.push('I ENV 5200 c5: bench n=21 ok=0 status=buserror valid=0 temp=21.40 phpa=1012.07 rh=40.10 altm=10.2');
  const s = fed(lines);
  const env = s.parts.get('ENV');
  assert.strictEqual(env.lastOk, false);
  assert.strictEqual(env.lastFail.reason, 'buserror', 'the bench line says status=, no reason=');
  assert.strictEqual(env.lastReason.reason, 'bus', "the driver's own word, from its W line");
  assert.ok(env.failed >= 2 && env.ok >= 20, env.failed + ' ' + env.ok);
  assert.ok(Math.abs(env.rate('bench') - 5) < 0.01, String(env.rate('bench')));
  assert.strictEqual(env.events.get('bench').fields.status, 'buserror');
});

test('a part that goes quiet says so while the others keep talking', () => {
  const s = new B.Session();
  BOOT.forEach((l, i) => s.add(1000 + i, l));
  for (let i = 1; i <= 10; i++) {
    s.add(2000 + i * 200, 'I ENV ' + (2000 + i * 200) + ' c2: bench n=' + i + ' ok=1 status=ok valid=1');
    s.add(2000 + i * 200, 'I IMU ' + (2000 + i * 200) + ' c2: bench n=' + i + ' ok=1 valid=1111 new=1111');
  }
  // The barometer stops; the IMU goes on.
  for (let i = 11; i <= 40; i++) s.add(2000 + i * 200, 'I IMU ' + (2000 + i * 200) + ' c3: bench n=' + i + ' ok=1 valid=1111 new=1111');
  const now = 2000 + 40 * 200;
  const env = s.parts.get('ENV').quiet(now);
  assert.strictEqual(env.usual, 200);
  assert.strictEqual(env.ms, 6000);
  assert.strictEqual(env.gone, true);
  assert.strictEqual(s.parts.get('IMU').quiet(now).gone, false);
});

test('a restart is marked with its cause, how long the board was up, and the last thing it said', () => {
  const lines = [...BOOT, 'I CYC 7000 c7: beat n=7 txon=1 loopmaxms=12 sent=20 txbad=0', 'I TX 7100 c7: done ok=1 ctr=21 code=0 ms=143'];
  lines.push('I BOOT 11 c0: start test=bench module=le csid=254 sf=7 dbm=10 txoffms=30000');
  lines.push('I RST 12 c0: cause csr=0x0A000000 lpwr=0 wwdg=0 iwdg=0 sft=0 bor=1 pin=0 obl=0 rfilar=0');
  const s = fed(lines);
  assert.strictEqual(s.boots.length, 2);
  const b = s.boots[1];
  assert.strictEqual(b.cause.bor, '1');
  assert.strictEqual(b.upMs, 7100);
  assert.strictEqual(b.before, 'I TX 7100 c7: done ok=1 ctr=21 code=0 ms=143');
  assert.ok(!s.parts.has('TX') && !s.parts.has('CYC'), 'what the parts said before the start is gone with it');
});

test('three starts inside a minute is a loop; a quiet port is not a start', () => {
  const s = new B.Session();
  for (const t of [0, 8000, 16000]) BOOT.forEach((l, i) => s.add(t + i, l));
  assert.strictEqual(s.looping(17000), 3);
  assert.strictEqual(s.looping(17000 + B.LOOP_MS), 0, 'a minute on, the loop has stopped');

  const q = fed(BOOT);
  q.add(1000 + 60000, 'I CYC 61000 c61: beat n=61 txon=1 loopmaxms=10 sent=1 txbad=0');
  assert.strictEqual(q.boots.length, 1);
  assert.strictEqual(q.gaps.length, 1, 'the port said nothing for a minute: shown as a gap, not a start');
});

test('a digit lost from the clock is not a start', () => {
  const s = fed([...BOOT, 'I CYC 52000 c52: beat n=52 txon=1 loopmaxms=10 sent=1 txbad=0']);
  s.add(99999, 'I ENV 5200 c52: bench n=9 ok=1 status=ok valid=1');
  assert.strictEqual(s.boots.length, 1);
});

test("a start whose line was lost is still a start, from the board's clock going back", () => {
  const s = fed([...BOOT, 'I CYC 50000 c50: beat n=50 txon=1 loopmaxms=10 sent=1 txbad=0']);
  s.add(99999, 'I ENV 900 c0: init ok=1 ms=8');
  assert.strictEqual(s.boots.length, 2);
  assert.strictEqual(s.boots[1].unseen, true);
  assert.strictEqual(s.boots[1].upMs, 50000);
});

test('the receivers count only the bench, and say what went missing', () => {
  const bytes = (counter, csid) => {
    const values = {};
    for (const f of P.FIELDS) values[f.key] = 0;
    Object.assign(values, { counter, csid, validity: 0 });
    const b = P.encode(values, 0);
    return P.bytesToHex(P.encode(values, P.crc16(b, P.DATA_LENGTH)));
  };
  const r = new B.Radio();
  const line = (c, csid = 254) => 'PKT,55,' + bytes(c, csid) + ',-40,9.5,120';
  assert.strictEqual(r.add(1, 'rx1', line(1)), true);
  r.add(2, 'rx1', line(2));
  r.add(2, 'rx2', line(2));             // the same packet at the second receiver
  r.add(3, 'rx1', line(5));             // 3 and 4 never reached rx1
  assert.strictEqual(r.add(4, 'rx1', line(6, 9)), false, 'a flight board is not the bench');
  r.add(5, 'rx1', line(0));             // started again
  r.add(6, 'rx3', line(65534));
  r.add(7, 'rx3', line(65535));
  r.add(8, 'rx3', line(1));             // wrapped past 65535, 0 never arrived
  const one = r.by.get('rx1');
  const two = r.by.get('rx2');
  const three = r.by.get('rx3');
  assert.deepStrictEqual([one.heard, one.missed, one.restarts, one.badCrc], [4, 2, 1, 0]);
  assert.deepStrictEqual([two.heard, two.missed, two.restarts], [1, 0, 0], 'a packet both heard is no restart');
  assert.deepStrictEqual([three.heard, three.missed, three.restarts], [3, 1, 0]);
  assert.deepStrictEqual(one.rssi.slice(0, 1), [-40]);
});
