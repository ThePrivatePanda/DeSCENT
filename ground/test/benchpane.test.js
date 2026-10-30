// The sensor bench on a ChipSat's sheet: chipsats.js and benchpane.js against test/mock/fakepage.js
const test = require('node:test');
const assert = require('node:assert');
const { page } = require('./mock/fakepage.js');

const UID9 = '2032374555435010002C002E';
const PROBE = { serial: '002A00', kind: 'STLink V3', access: true };
const CS9 = { uid: UID9, name: 'CS9', module: 'le', lastSeen: 1, firmware: 'bench', firmwareModule: 'le' };
const FLIGHT9 = Object.assign({}, CS9, { firmware: 'flight' });
const CONSOLE = { port: '/dev/ttyUSB0', key: 'usb-console-1', usb: '10c4:ea60', state: 'receiver', name: '' };
const TBEAM = { port: '/dev/ttyACM0', key: 'usb-tbeam-1', usb: '303a:1001', state: 'receiver', name: 'rx A' };
const BLOCK = { layoutVersion: 3, csid: 9, module: 'le', sf: 7, dbm: 14, setBy: 'build', txIntervalMs: 10000, logBlindMs: 200, reservedClear: true };
const readOf = (kind) => ({ ok: true, uid: UID9, flashBytes: 262144, firmware: { kind, module: 'le', id: 'c4fb204d8f79e725' },
  settings: kind === 'flight' ? BLOCK : null, settingsError: null, probe: PROBE });

const START = (cause) => [
  'I BOOT 12 c0: start test=bench module=le csid=254 sf=7 dbm=10 txoffms=30000',
  'I RST 13 c0: cause csr=0x0C000000 lpwr=0 wwdg=0 iwdg=' + (cause === 'iwdg' ? 1 : 0) + ' sft=0 bor=' + (cause === 'bor' ? 1 : 0) + ' pin=' + (cause ? 0 : 1) + ' obl=0 rfilar=0',
  'I FLASH 41 c0: init ok=1 id=0xEF4015',
  'E GPS 900 c0: init ok=0 reason=config attempts=3',
  'I ENV 920 c0: init ok=1 ms=8',
];

// text() spaces out every tag, so "<b>CS9</b>." reads "CS9 ."
const tidy = (t) => t.replace(/ ([,.;:?!)])/g, '$1');

// The ChipSats screen with the bench pane loaded. No probe unless a test gives one.
async function bench(o) {
  o = Object.assign({ screen: 'chipsats', chips: [CS9], probes: [], boards: [CONSOLE, TBEAM], logs: [] }, o);
  const hub = {
    native: true, ports: o.ports || [], boards: o.boards,
    chips: async () => o.chips, chipProbes: async () => o.probes, logs: async () => o.logs, radio: async () => null, health: async () => null,
    chipRead: async () => o.read, chipPeek: async () => (o.read ? o.read.uid : null),
    chipFlash: async (q) => { if (o.refuse) throw new Error(o.refuse); o.flashed = q; },
    benchRadio: async (port) => { o.radioed = port; },
    setBoard: async (port, read) => { o.listened = [port, read]; },
    pull: async () => {}, pullStop: async () => {},
  };
  const p = page(['bench.js', 'boardmodel.js', 'datamodel.js', 'chipsats.js', 'benchpane.js'], hub, o);
  await p.settle();
  p.parts.chipsats.show();
  await p.settle();
  const say = (key, lines, t0) => lines.forEach((l, i) => p.taps.forEach((tap) => tap(key, l, (t0 || Date.now()) + i)));
  // the sheet's own timer (1 s) and the bench pane's (500 ms)
  const tick = async () => { await p.every(1000); await p.every(500); };
  return Object.assign(p, { o, say, tick, head: () => p.html('cs-sheet'), live: () => p.html('cs-bench'),
    sheet: () => p.html('cs-sheet') + p.html('cs-bench'), side: () => p.html('cs-list'),
    headWords: () => tidy(p.text('cs-sheet')), liveWords: () => tidy(p.text('cs-bench')),
    words: () => tidy(p.text('cs-sheet') + ' ' + p.text('cs-bench')) });
}

test('each part shows what its own lines said, verbatim, beside the raw console', async () => {
  const p = await bench();
  p.say(CONSOLE.key, [...START(), 'I ENV 1200 c1: bench n=1 ok=1 status=ok valid=1 temp=21.40 phpa=1012.07 rh=40.10 altm=10.2']);
  await p.tick();
  const s = p.sheet();
  const w = p.words();
  assert.match(w, /Barometer \(BME280\) ok=1/);
  assert.match(w, /\btemp\s*=21\.40/, 'the value as printed');
  assert.match(w, /GPS \(MAX-M10S\) ok=0/);
  assert.match(s, /reason=config/);
  assert.match(w, /\bid\s*=0xEF4015/);
  assert.match(s, /I ENV 1200 c1: bench n=1 ok=1/, 'the raw line is there too');
  assert.match(p.side(), /CS9.*sensor bench/);
});

test('a start of the board is marked with its cause, and three in a minute is a loop', async () => {
  const p = await bench();
  const t = Date.now();
  p.say(CONSOLE.key, [...START(), 'I CYC 5000 c5: beat n=5 txon=0 loopmaxms=11 sent=0 txbad=0'], t - 30000);
  p.say(CONSOLE.key, START('bor'), t - 20000);
  await p.tick();
  assert.match(p.sheet(), /Starts \(2\)/);
  assert.match(p.sheet(), /after 5.0 s up/);
  assert.match(p.sheet(), /cause: bor/);
  assert.match(p.sheet(), /last before it: I CYC 5000 c5: beat/);
  assert.doesNotMatch(p.sheet(), /Restarting over and over/);
  p.say(CONSOLE.key, START('iwdg'), t - 10000);
  await p.tick();
  assert.match(p.sheet(), /Restarting over and over: 3 starts in the last minute/);
});

test('a board that does not say test=bench is said not to be on the bench, beside what was last read', async () => {
  const p = await bench();
  p.say(CONSOLE.key, ['I BOOT 12 c0: start log=4']);
  await p.tick();
  assert.match(p.sheet(), /Last read: sensor bench\. The board did not say test=bench/);
  assert.match(p.sheet(), /id="cs-bench"/, 'the layout goes by what was read, not by the console');
});

test('the receivers count the bench as board 254 and nothing else', async () => {
  const p = await bench();
  const pkt = (counter, csid) => ({ kind: 'packet', csid, rx: TBEAM.key, t: Date.now(), rssi: -41, snr: 9.0, crcOk: true,
    values: { counter, csid, validity: 0 }, valid: {} });
  for (const m of [pkt(1, 254), pkt(2, 254), pkt(4, 254), pkt(7, 9)]) p.packetTaps.forEach((tap) => tap(m));
  await p.tick();
  assert.match(p.words(), /usb-tbeam-1 3 1 0 0 -41\b/);
  assert.match(p.sheet(), /from any board on the sensor bench \(254\)/, 'not said to be this board');
});

test('writing the sensor bench is a row under Other images that asks first, saves nothing, and goes by the board\'s module', async () => {
  const p = await bench({ chips: [FLIGHT9], probes: [PROBE], read: readOf('flight') });
  await p.fire('screen-chipsats', 'click', { 'data-images': '1' });
  assert.match(p.headWords(), /Other images .*Write the sensor bench/);
  assert.match(p.head(), /data-benchwrite="1"[^>]*>Write the sensor bench/);
  await p.fire('screen-chipsats', 'click', { 'data-benchwrite': '1' });
  assert.match(p.head(), /Nothing is saved first/);
  assert.match(p.head(), /sends as board 254/);
  assert.strictEqual(p.o.flashed, undefined);
  await p.fire('screen-chipsats', 'click', { 'data-benchgo': '1' });
  assert.strictEqual(JSON.stringify(p.o.flashed), JSON.stringify({ uid: UID9, embedded: 'bench', module: 'le', backup: false }));
});

test('the sensor bench write keeps the save-first box', async () => {
  const p = await bench({ chips: [FLIGHT9], probes: [PROBE], read: readOf('flight') });
  await p.fire('screen-chipsats', 'change', { 'data-backup': '1' }, { checked: true });
  await p.fire('screen-chipsats', 'click', { 'data-benchwrite': '1' });
  assert.match(p.head(), /The image on it now is saved first/);
  await p.fire('screen-chipsats', 'click', { 'data-benchgo': '1' });
  assert.strictEqual(p.o.flashed.backup, true);
});

test('a sensor bench write that cannot go says why beside the button', async () => {
  let p = await bench({ chips: [FLIGHT9] });
  await p.fire('screen-chipsats', 'click', { 'data-images': '1' });
  assert.match(p.head(), /data-benchwrite="1"[^>]*disabled[^>]*>Write the sensor bench/);
  assert.match(p.headWords(), /Write the sensor bench Put the board on the ST-Link and press Read the board\./);
  p = await bench({ chips: [Object.assign({}, FLIGHT9, { module: null })], probes: [PROBE], read: readOf('flight') });
  await p.fire('screen-chipsats', 'click', { 'data-images': '1' });
  assert.match(p.head(), /data-benchwrite="1"[^>]*disabled[^>]*>Write the sensor bench/);
  assert.match(p.headWords(), /Write the sensor bench Choose the radio module/);
});

test('a refused sensor bench write stays on the sheet, not only in a toast', async () => {
  const p = await bench({ chips: [FLIGHT9], probes: [PROBE], read: readOf('flight'), refuse: 'no debug probe is attached' });
  await p.fire('screen-chipsats', 'click', { 'data-benchwrite': '1' });
  await p.fire('screen-chipsats', 'click', { 'data-benchgo': '1' });
  assert.ok(p.headWords().includes('Not written: no debug probe is attached'), p.headWords());
});

test('radio on/off goes to the console port of the bench board', async () => {
  const p = await bench();
  p.say(CONSOLE.key, START());
  await p.tick();
  assert.match(p.head(), /data-radio="\/dev\/ttyUSB0"[^>]*>Radio on\/off/);
  await p.fire('screen-chipsats', 'click', { 'data-radio': CONSOLE.port });
  assert.strictEqual(p.o.radioed, CONSOLE.port);
});

test('a console port nobody reads yet is offered to be read', async () => {
  const p = await bench({ boards: [Object.assign({}, CONSOLE, { state: 'waiting' }), TBEAM] });
  assert.match(p.sheet(), /data-listen="\/dev\/ttyUSB0"/);
  await p.fire('screen-chipsats', 'click', { 'data-listen': CONSOLE.port });
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
  await p.tick();
  const s = p.live();
  assert.match(p.liveWords(), /I2C bus ok=1/);
  assert.match(s, /W BUS 1234 c5: clear sda=0 scl=1 pulses=9 ok=1/);
  assert.match(s, /W BUS 1250 c5: quarantine part=env/);
  assert.ok(s.indexOf('quarantine part=env') < s.indexOf('clear sda=0'), 'newest first');
  assert.doesNotMatch(s, /<span class="l">quarantine<\/span>/);
  assert.match(p.liveWords(), /Barometer \(BME280\).*last reason \S+ reason=noack W ENV 1240 c5: recover step=reset reason=noack/);
  assert.ok(s.indexOf('I2C bus') > s.indexOf('GPS (MAX-M10S)') && s.indexOf('I2C bus') < s.indexOf('Flash (W25Q16)'));
});

test('a click on the name inside a list row picks that board', async () => {
  const other = { uid: '0038002A5553500920393444', name: '', module: null, lastSeen: 2, firmware: null };
  const p = await bench({ chips: [CS9, other] });
  assert.match(p.head(), /CS9/);
  const row = { getAttribute: (n) => (n === 'data-pick' ? other.uid : null) };
  await p.fire('screen-chipsats', 'click', {}, { tagName: 'B', closest: (sel) => (sel === '[data-pick]' ? row : null) });
  assert.match(p.head(), /Unnamed board …3444/, 'called what the list calls it');
  assert.doesNotMatch(p.head(), /id="cs-bench"/, 'a board not on the bench has no bench pane');
});

test('with no board known the list says so and there is no bench pane', async () => {
  const p = await bench({ chips: [] });
  assert.match(p.side(), /No board yet/);
  assert.doesNotMatch(p.head(), /Sensor bench|id="cs-bench"/);
});

// ---------- where the pane goes ----------

test('a flight board keeps its layout and has no bench pane', async () => {
  const p = await bench({ chips: [FLIGHT9], probes: [PROBE], read: readOf('flight') });
  const h = p.head();
  assert.doesNotMatch(h, /id="cs-bench"|<details/);
  assert.match(p.headWords(), /Identity and radio.*\bPhases\b.*\bWrite\b.*Flight logs.*Other images/);
});

test('a bench board has the bench first, then Other images, then the flight panes folded with why', async () => {
  const p = await bench();
  const h = p.head();
  const at = (re) => h.search(re);
  assert.ok(at(/data-radio=/) > 0);
  assert.ok(at(/data-radio=/) < at(/id="cs-bench"/));
  assert.ok(at(/id="cs-bench"/) < at(/data-images=/));
  assert.ok(at(/data-images=/) < at(/<details class="fold" data-fold="radio">/));
  assert.ok(at(/data-fold="radio"/) < at(/<details class="fold" data-fold="log">/));
  assert.match(p.headWords(), /Identity and radio, phases Not on the board: the sensor bench has no settings block/);
  assert.match(h, /data-fold="log"/);
  assert.match(p.headWords(), /Flight logs Pull or clear the chip\./);
  // one console chooser on the sheet, in the bench pane
  assert.equal((h.match(/<select data-console=/g) || []).length, 1);
  assert.ok(at(/<select data-console=/) < at(/id="cs-bench"/));
  assert.match(h, /chosen in Sensor bench/);
});

test('a folded pane stays open across redraws once opened', async () => {
  const p = await bench();
  await p.fire('screen-chipsats', 'toggle', { 'data-fold': 'radio' }, { open: true });
  assert.match(p.head(), /data-fold="radio" open>/);
  await p.tick();
  assert.match(p.head(), /data-fold="radio" open>/);
  assert.doesNotMatch(p.head(), /data-fold="log" open>/);
});

test('the bench pane stays when the console goes quiet, and says how long', async () => {
  const p = await bench();
  p.say(CONSOLE.key, START(), Date.now() - 20000);
  await p.tick();
  assert.match(p.head(), /id="cs-bench"/);
  assert.match(p.live(), /Nothing on the console for 20 s/);
});

test('a flight board whose console says test=bench gets a line naming both, and keeps its layout', async () => {
  const p = await bench({ chips: [FLIGHT9], probes: [PROBE], read: readOf('flight') });
  p.say(CONSOLE.key, START());
  await p.tick();
  assert.match(p.headWords(), /Last read: flight software[^.]*\. The console says test=bench\. Read the board/);
  assert.match(p.head(), /data-read="002A00"/);
  assert.doesNotMatch(p.head(), /id="cs-bench"/);
});

test('flight software can be written from the bench pane, saying what it writes', async () => {
  const p = await bench({ probes: [PROBE], read: readOf('bench') });
  assert.match(p.headWords(), /Sensor bench running it, read off the board now/);
  await p.fire('screen-chipsats', 'click', { 'data-buildembed': '1', 'data-where': 'bench' });
  const h = p.head();
  assert.equal((h.match(/data-buildgo="1"/g) || []).length, 1, 'one confirm, in the bench pane');
  assert.ok(h.indexOf('data-buildgo') < h.indexOf('id="cs-bench"'));
  assert.match(p.headWords(), /LE module.*board 255, LE, SF7, 14 dBm/);
  await p.fire('screen-chipsats', 'click', { 'data-buildgo': '1' });
  assert.match(p.o.flashed.embedded, /^(built|flight)$/);
  assert.strictEqual(p.o.flashed.uid, UID9);
  assert.strictEqual(p.o.flashed.csid, '255');
});

test('the bench timer redraws only its own pane', async () => {
  const p = await bench();
  p.say(CONSOLE.key, START());
  await p.tick();
  const head = p.head();
  const live = p.live();
  p.say(CONSOLE.key, ['I ENV 1200 c1: bench n=1 ok=1 status=ok valid=1 temp=21.40']);
  await p.every(500);
  assert.strictEqual(p.head(), head);
  assert.notStrictEqual(p.live(), live);
});

test('lines are found under the open port\'s key, as the app sends them (its boards list has no key)', async () => {
  const bare = (b) => { const c = Object.assign({}, b); delete c.key; return c; };
  const p = await bench({ boards: [bare(CONSOLE), bare(TBEAM)], ports: [{ key: 'rx1', port: CONSOLE.port }, { key: 'rx2', port: TBEAM.port }] });
  p.say('rx1', [...START(), 'I ENV 1200 c1: bench n=1 ok=1 status=ok valid=1 temp=21.40']);
  await p.tick();
  assert.match(p.live(), /<span class="k">temp<\/span>=21.40/);
  assert.match(p.head(), /data-radio="\/dev\/ttyUSB0"[^>]*>Radio on\/off/);
});
