// benchscreen.js against test/mock/fakepage.js
const test = require('node:test');
const assert = require('node:assert');
const { page } = require('./mock/fakepage.js');
const P = require('../web/js/packet.js');

const UID9 = '2032374555435010002C002E';
const CS9 = { uid: UID9, name: 'CS9', module: 'le', lastSeen: 1, firmware: 'bench', firmwareModule: 'le' };
const CONSOLE = { port: '/dev/ttyUSB0', key: 'usb-console-1', usb: '10c4:ea60', state: 'receiver', name: '' };
const TBEAM = { port: '/dev/ttyACM0', key: 'usb-tbeam-1', usb: '303a:1001', state: 'receiver', name: 'rx A' };

const START = (cause) => [
  'I BOOT 12 c0: start test=bench module=le csid=254 sf=7 dbm=10 txoffms=30000',
  'I RST 13 c0: cause csr=0x0C000000 lpwr=0 wwdg=0 iwdg=' + (cause === 'iwdg' ? 1 : 0) + ' sft=0 bor=' + (cause === 'bor' ? 1 : 0) + ' pin=' + (cause ? 0 : 1) + ' obl=0 rfilar=0',
  'I FLASH 41 c0: init ok=1 id=0xEF4015',
  'E GPS 900 c0: init ok=0 reason=config attempts=3',
  'I ENV 920 c0: init ok=1 ms=8',
];

async function bench(o) {
  o = Object.assign({ screen: 'bench', chips: [CS9], boards: [CONSOLE, TBEAM] }, o);
  const hub = {
    native: true, ports: [], boards: o.boards,
    chips: async () => o.chips,
    chipFlash: async (q) => { o.flashed = q; },
    benchRadio: async (port) => { o.radioed = port; },
    setBoard: async (port, read) => { o.listened = [port, read]; },
    chipJob: async () => null,
  };
  const p = page(['packet.js', 'lines.js', 'bench.js', 'benchscreen.js'], hub, o);
  await p.settle();
  p.parts.bench.show();
  await p.settle();
  const say = (key, lines, t0) => lines.forEach((l, i) => p.taps.forEach((tap) => tap(key, l, (t0 || Date.now()) + i)));
  return Object.assign(p, { o, say, sheet: () => p.html('bn-sheet'), side: () => p.html('bn-side') });
}

test('each part shows what its own lines said, verbatim, beside the raw console', async () => {
  const p = await bench();
  p.say(CONSOLE.key, [...START(), 'I ENV 1200 c1: bench n=1 ok=1 status=ok valid=1 temp=21.40 phpa=1012.07 rh=40.10 altm=10.2']);
  await p.ticks(1);
  const s = p.sheet();
  assert.match(s, /Barometer \(BME280\) <span class="mark good">ok=1<\/span>/);
  assert.match(s, /<span class="k">temp<\/span>=21.40/, 'the value as printed');
  assert.match(s, /GPS \(MAX-M10S\) <span class="mark bad">ok=0<\/span>/);
  assert.match(s, /reason=config/);
  assert.match(s, /<span class="k">id<\/span>=0xEF4015/);
  assert.match(s, /I ENV 1200 c1: bench n=1 ok=1/, 'the raw line is there too');
  assert.match(p.side(), /CS9.*sensor bench/);
});

test('a start of the board is marked with its cause, and three in a minute is a loop', async () => {
  const p = await bench();
  const t = Date.now();
  p.say(CONSOLE.key, [...START(), 'I CYC 5000 c5: beat n=5 txon=0 loopmaxms=11 sent=0 txbad=0'], t - 30000);
  p.say(CONSOLE.key, START('bor'), t - 20000);
  await p.ticks(1);
  assert.match(p.sheet(), /Starts \(2\)/);
  assert.match(p.sheet(), /after 5.0 s up/);
  assert.match(p.sheet(), /cause: bor/);
  assert.match(p.sheet(), /last before it: I CYC 5000 c5: beat/);
  assert.doesNotMatch(p.sheet(), /Restarting over and over/);
  p.say(CONSOLE.key, START('iwdg'), t - 10000);
  await p.ticks(1);
  assert.match(p.sheet(), /Restarting over and over: 3 starts in the last minute/);
});

test('a board that does not say test=bench is said not to be on the bench', async () => {
  const p = await bench();
  p.say(CONSOLE.key, ['I BOOT 12 c0: start log=4']);
  await p.ticks(1);
  assert.match(p.sheet(), /did not say test=bench/);
});

test('the receivers count the bench as board 254 and nothing else', async () => {
  const p = await bench();
  const pkt = (counter, csid) => {
    const v = {};
    for (const f of P.FIELDS) v[f.key] = 0;
    Object.assign(v, { counter, csid, validity: 0 });
    const b = P.encode(v, 0);
    return 'PKT,55,' + P.bytesToHex(P.encode(v, P.crc16(b, P.DATA_LENGTH))) + ',-41,9.0,100';
  };
  p.say(TBEAM.key, [pkt(1, 254), pkt(2, 254), pkt(4, 254), pkt(7, 9)]);
  await p.ticks(1);
  assert.match(p.sheet(), /<td>usb-tbeam-1<\/td><td>3<\/td><td>1<\/td><td>0<\/td><td>0<\/td><td>-41<\/td>/);
});

test('putting the bench on asks first, saves nothing, and goes by the board\'s module', async () => {
  const p = await bench({ chips: [Object.assign({}, CS9, { firmware: 'flight' })] });
  await p.fire('screen-bench', 'click', { 'data-bench': '1' });
  assert.match(p.sheet(), /Nothing is saved first/);
  assert.strictEqual(p.o.flashed, undefined);
  await p.fire('screen-bench', 'click', { 'data-benchgo': '1' });
  assert.strictEqual(JSON.stringify(p.o.flashed), JSON.stringify({ uid: UID9, embedded: 'bench', module: 'le', backup: false }));
  await p.fire('screen-bench', 'click', { 'data-radio': CONSOLE.port });
  assert.strictEqual(p.o.radioed, CONSOLE.port);
});

test('a console port nobody reads yet is offered to be read', async () => {
  const p = await bench({ boards: [Object.assign({}, CONSOLE, { state: 'waiting' }), TBEAM] });
  assert.match(p.sheet(), /data-listen="\/dev\/ttyUSB0"/);
  await p.fire('screen-bench', 'click', { 'data-listen': CONSOLE.port });
  assert.strictEqual(JSON.stringify(p.o.listened), JSON.stringify([CONSOLE.port, true]));
});

test('the I2C bus has a card of its own with its lines as the board printed them', async () => {
  const p = await bench();
  p.say(CONSOLE.key, [
    ...START(),
    'W BUS 1234 c5: clear sda=0 scl=1 pulses=9 ok=1',
    'W ENV 1240 c5: recover step=reset reason=noack',
    'W BUS 1250 c5: quarantine part=env',
  ]);
  await p.ticks(1);
  const s = p.sheet();
  assert.match(s, /<h3>I2C bus <span class="mark good">ok=1<\/span>/);
  assert.match(s, /W BUS 1234 c5: clear sda=0 scl=1 pulses=9 ok=1/);
  assert.match(s, /W BUS 1250 c5: quarantine part=env/);
  assert.ok(s.indexOf('quarantine part=env') < s.indexOf('clear sda=0'), 'newest first');
  assert.doesNotMatch(s, /<span class="l">quarantine<\/span>/);
  assert.match(s, /Barometer \(BME280\)[\s\S]*last reason<\/span> [^<]* reason=noack<div class="hex muted">W ENV 1240 c5: recover step=reset reason=noack<\/div>/);
  assert.ok(s.indexOf('I2C bus') > s.indexOf('GPS (MAX-M10S)') && s.indexOf('I2C bus') < s.indexOf('Flash (W25Q16)'));
});
