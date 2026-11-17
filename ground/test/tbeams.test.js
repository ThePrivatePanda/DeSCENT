// The T-Beams screen's own script, run against test/mock/fakepage.js. Only Forget is
// covered so far. It says nothing about how the page looks.
const test = require('node:test');
const assert = require('node:assert');
const { page } = require('./mock/fakepage.js');

test('forgetting a T-Beam asks first, says nothing is sent to it, and leaves it not touched', async () => {
  const o = { screen: 'tbeams', boards: [{ port: '/dev/ttyACM1', usb: '1a86:55d4', label: 'CH9102', serial: 'x', key: 'k', state: 'receiver', name: 'Left T-Beam' }] };
  const hub = {
    native: true, ports: [],
    get boards() { return o.boards; },
    candidates: async () => ({ candidates: [] }),
    forgetBoard: async (port) => { o.forgot = port; o.boards = o.boards.map((b) => Object.assign({}, b, { state: 'waiting', name: '' })); },
  };
  const p = page(['boardmodel.js', 'tbeams.js'], hub, o);
  p.parts.tbeams.show();
  await p.settle();
  assert.match(p.html('tb-sheet'), /data-forget="\/dev\/ttyACM1">Forget this board/);

  await p.fire('screen-tbeams', 'click', { 'data-forget': '/dev/ttyACM1' });
  assert.equal(o.forgot, undefined);
  assert.match(p.html('tb-sheet'), /Forget <b>Left T-Beam<\/b>\? .*Nothing is sent to the board\./);

  await p.fire('screen-tbeams', 'click', { 'data-forgetgo': '/dev/ttyACM1' });
  assert.equal(o.forgot, '/dev/ttyACM1');
  assert.equal(p.toasts[p.toasts.length - 1], 'Forgot Left T-Beam');
  assert.match(p.html('tb-list'), /not touched/);
  assert.doesNotMatch(p.html('tb-sheet'), /data-forgetgo/);
});

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

// fakepage keeps the live fleet and the app object to itself; this one line hands them to the
// hub so a test can put receivers and programs in them.
function withLive(scripts) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tbeams-'));
  const file = path.join(dir, 'live.js');
  fs.writeFileSync(file, 'DGApp.hub.live = DGApp.live; DGApp.hub.app = DGApp;');
  return ['boardmodel.js', ...scripts, path.relative(path.join(__dirname, '..', 'web', 'js'), file)];
}

// the press lands on an element inside the card, which is how the handler finds it
const pick = (p, port) => p.fire('screen-tbeams', 'click', { 'data-pick': port }, { closest() { return this; } });

const NOW = Date.now();
const SINCE = NOW - 7 * 60000;
const hm = (t) => String(new Date(t).getHours()).padStart(2, '0') + ':' + String(new Date(t).getMinutes()).padStart(2, '0');

function rig(extra) {
  const o = Object.assign({
    screen: 'tbeams',
    boards: [
      { port: '/dev/ttyUSB0', usb: '10c4:ea60', label: 'CP2102', serial: '', key: 'k0', state: 'receiver', name: '' },
      { port: '/dev/ttyUSB1', usb: '10c4:ea60', label: 'CP2102', serial: '', key: 'k1', state: 'receiver', name: '' },
      { port: '/dev/ttyACM0', usb: '1209:5741', label: 'flight controller', serial: '', key: 'k2', state: 'ignored', name: '' },
    ],
    ports: [
      { key: 'rx1', port: '/dev/ttyUSB0', usb: '10c4:ea60', status: 'open', rxId: '72E53C', rxName: 'Mast', rxFw: '1.4.2' },
      { key: 'rx2', port: '/dev/ttyUSB1', usb: '10c4:ea60', status: 'open', rxId: 'A4C1F3', rxName: 'Handheld', rxFw: '1.5.0' },
    ],
    cands: ['/dev/ttyUSB0'],
    version: '1.5.0',
    flashed: [],
  }, extra || {});
  const hub = {
    native: true,
    get ports() { return o.ports; },
    get boards() { return o.boards; },
    candidates: async () => ({ candidates: o.cands.map((port) => ({ port })), firmware: 'descent-raw-receiver-' + o.version + '.bin', firmwareVersion: o.version }),
    testPort: async () => ({ looks_like: 'something-else', lines: ['ets Jun  8 2016'] }),
    identifyStart: async () => { o.watch = true; },
    identifyState: async () => o.identify || { watching: true, seconds_left: 28 },
    identifyStop: async () => { o.watch = false; },
    flash: async (port, sf) => { o.flashed.push([port, sf]); return { backup: '' }; },
    setBoard: async (port, on) => { o.set = [port, on]; },
  };
  const p = page(withLive(['tbeams.js']), hub, o);
  const rx = (key, sf, n) => hub.live.receivers.set(key, Object.assign({
    key, id: key === 'rx1' ? '72E53C' : 'A4C1F3', firstT: SINCE, lastT: NOW - 300, lastHbT: NOW - 4000,
    packets: 2585, unique: 2418, badCrc: 4, radioErrors: 0,
    info: { id: 'x', f: '915.0', bw: '125.0', sf: String(sf), cr: '7', sync: '0x12', pre: '8' },
  }, n || {}));
  return { o, hub, p, rx };
}

test('the list has a card per board: name, state word, SF, and its port and USB id', async () => {
  const { o, p, rx } = rig();
  rx('rx1', 7); rx('rx2', 9);
  p.parts.tbeams.show();
  await p.settle();
  const list = p.text('tb-list');
  assert.match(list, /Plugged in . 3 boards/);
  assert.match(list, /Mast reading it SF7 \/dev\/ttyUSB0 . USB 10C4:EA60/i);
  assert.match(list, /Handheld reading it SF9/);
  assert.match(list, /\/dev\/ttyACM0 left alone/);
  // the picked board's card is the one with the ring
  assert.match(p.html('tb-list'), /class="card sel ring tb-card" data-pick="\/dev\/ttyUSB0"/);
  assert.doesNotMatch(p.html('tb-list'), /ring[^>]*data-pick="\/dev\/ttyUSB1"/);
  o.boards = [o.boards[0]];
  await p.ticks(1);
  assert.match(p.text('tb-list'), /Plugged in . 1 board USB serial/);
});

// 2026-11-16: the ST-Link's port was read as a T-Beam and listed here
test('a ChipSat console is not a T-Beam, and a T-Beam that could not be opened says why', async () => {
  const { o, p, rx } = rig();
  o.boards = o.boards.concat([
    { port: '/dev/ttyACM2', usb: '0483:3754', label: 'ST-Link', serial: '002F0028', key: 'k3', state: 'receiver', name: '', role: 'console' },
    { port: '/dev/ttyACM3', usb: '1a86:7523', label: 'CH340', serial: '', key: 'k4', state: 'receiver', name: '', role: 'console' },
  ]);
  o.boards[1] = Object.assign({}, o.boards[1], { role: 'receiver', error: 'held by another program, perhaps the other copy of this app' });
  o.ports = [o.ports[0]];
  rx('rx1', 7);
  p.parts.tbeams.show();
  await p.settle();
  const list = p.text('tb-list');
  assert.match(list, /Plugged in . 3 boards/);
  assert.doesNotMatch(p.html('tb-list'), /ttyACM2|ttyACM3/);
  assert.match(list, /\/dev\/ttyUSB1 not opened USB 10C4:EA60 . held by another program, perhaps the other copy of this app/i);
  await pick(p, '/dev/ttyUSB1');
  assert.match(p.text('tb-sheet'), /\/dev\/ttyUSB1: held by another program, perhaps the other copy of this app/);
});

// An SDR program as /api/programs gives it (command.rs), with the fleet's counts for it
const SDR = (more) => Object.assign({
  key: 'cmd:sdr', command: 'python3 /opt/sdr_rx.py --decoder /opt/file_lora', sdrLine: true, on: true, gain: null, gainAt: null,
  state: 'running', startedAt: NOW - 60000, nextAt: null, starts: 3, lastExit: 'stopped, exit status: 1', lastExitAt: NOW - 61000,
  fatal: null, lastWords: null,
  header: { t: NOW - 50000, fields: { id: 'EEF522', f: '915.0', bw: '125.0', cr: '7', sync: '0x12', pre: '8', kind: 'sdr', sfs: '10+11+12', gain: '0.9', fw: '0.2.0' } },
  said: [{ t: NOW - 59000, text: 'rtl_sdr: Found 1 device(s):' }, { t: NOW - 58000, text: 'rtl_sdr: Tuned to 916200000 Hz.' }],
  stats: {
    beat: { t: NOW - 2000, upMs: 58000, ovf: 0, drop: 2, clip: 12.3, sat: 1, floor: -40.2 },
    bySf: [{ sf: 12, packets: 3, badCrc: 1, headerDamaged: 1, other: 0, lastT: NOW - 4000, lastSnr: 8.4, boards: [9, 13] },
      { sf: 11, packets: 0, badCrc: 0, headerDamaged: 2, other: 1, lastT: null, lastSnr: null, boards: [] }],
  },
}, more);
const SDR_PORT = { key: 'cmd:sdr', port: 'python3 /opt/sdr_rx.py --decoder /opt/file_lora', usb: '', status: 'open', rxId: 'EEF522', rxKind: 'sdr', rxFw: '0.2.0', role: 'receiver' };

// The rig, with app.js's copy of /api/programs and its programDo on the app object
function sdrRig(list, ports) {
  const done = [];
  const r = rig({ ports: ports || [SDR_PORT] });
  const app = r.hub.app;
  app.programs = { list, sdrLine: list.some((x) => x.sdrLine), missing: false };
  app.programDo = async (action, body) => { done.push([action, body]); return { ok: true, gone: true }; };
  return Object.assign(r, { app, done });
}

test('an SDR gets a card of its own and a sheet; any other program stays a line under Programs', async () => {
  const line = SDR({ key: 'cmd:1', sdrLine: false, command: 'ssh roof sdr_rx', header: { t: NOW, fields: { id: '5D4E01', kind: 'sdr', sfs: '7+8', gain: '40.2', fw: '0.1.0' } } });
  const other = { key: 'cmd:2', command: 'sdr_rx --dev 1', sdrLine: false, on: true, state: 'stopped', header: null, said: [], stats: null };
  const { p } = sdrRig([line, other, SDR()], [
    { key: 'cmd:1', port: 'ssh roof sdr_rx', usb: '', status: 'open', rxId: '5D4E01', rxKind: 'sdr', rxFw: '0.1.0' },
    { key: 'cmd:2', port: 'sdr_rx --dev 1', usb: '', status: 'stopped', error: 'stopped, exit status: 1', rxId: null },
    SDR_PORT,
  ]);
  p.parts.tbeams.show();
  await p.settle();
  const list = p.text('tb-list');
  assert.match(list, /SDR · 2 started by the app SDR 5D4E01 listening SF7 · SF8 no packets yet SDR EEF522 listening SF10 · SF11 · SF12 no packets yet/);
  assert.match(list, /Programs · 1 receiver = in descent-ground.conf cmd:2 stopped no header yet . cmd:2 . sdr_rx --dev 1/);
  assert.doesNotMatch(list, /Programs · [^1]/);
  assert.match(p.html('tb-list'), /data-pick="cmd:sdr"/);
  assert.doesNotMatch(p.html('tb-list'), /data-pick="cmd:2"/);

  // a receiver = SDR shows the gain it reports and says where it is set; it has no select
  await pick(p, 'cmd:1');
  assert.match(p.text('tb-sheet'), /Gain 40\.2 dB Set on its receiver = line in descent-ground\.conf\./);
  assert.doesNotMatch(p.html('tb-sheet'), /data-sdrgain/);
  assert.match(p.text('tb-sheet'), /Turned off, it stays off until the app is started again/);
});

test("an SDR's sheet: who it is, its radio from the header, the signal, its counts by factor and what it said", async () => {
  const { p } = sdrRig([SDR()]);
  p.parts.tbeams.show();
  await p.settle();
  await pick(p, 'cmd:sdr');
  const sheet = p.text('tb-sheet');
  const html = p.html('tb-sheet');
  assert.match(html, /<input type="text" class="nameit"[^>]*data-name="cmd:sdr" data-rxid="EEF522"[^>]*placeholder="SDR EEF522"/);
  assert.match(sheet, /listening SDR EEF522 · fw 0\.2\.0 Turn off Restart/);
  // the command is said once, at the foot of This session, not in the head
  assert.equal(sheet.split('python3 /opt/sdr_rx.py').length, 2);
  assert.match(sheet, /in the log as cmd:sdr \. Started by python3 \/opt\/sdr_rx\.py --decoder \/opt\/file_lora What it said/);
  assert.match(sheet, /Frequency 915\.0 MHz Bandwidth · CR 125 kHz · 4\/7 Sync · preamble 0x12 · 8 Spreading factors Listens on SF10, SF11 and SF12/);
  // the select shows what the header says, and with no gain picked here there is no mark
  assert.match(html, /<option value="0\.9" selected>0\.9 dB<\/option>/);
  assert.match(html, /<option value="auto">Automatic \(AGC\)<\/option>/);
  assert.match(sheet, /The program's own report\. Picking another restarts it with --gain/);
  // saturated is the program's own sat=1, beside the value it gave
  assert.match(sheet, /Clipping 12\.30 % of samples in the last 5 s saturated Noise floor -40\.2 dBFS Samples lost 0 since it started \(ovf\) Lines dropped 2 since it started \(drop\)/);
  assert.match(sheet, /SF Packets Bad CRC -7 Header damaged -24 Other -1 Last packet Last SNR Boards heard SF10 0 0 0 0 — — — SF11 0 0 2 1 — — — SF12 3 1 1 0 ago 8\.40 dB 9, 13/);
  assert.match(sheet, /Started again 2 times since the app started\. Last stopped at \d\d:\d\d:\d\d\. exit status: 1 Packets/);
  assert.match(sheet, /What it said its last 2 lines on stderr \d\d:\d\d:\d\d rtl_sdr: Found 1 device\(s\): \d\d:\d\d:\d\d rtl_sdr: Tuned to 916200000 Hz\./);
});

test('saturated is not said when the program says sat=0, whatever the clip', async () => {
  const { p } = sdrRig([SDR({ stats: { beat: { t: NOW, upMs: 1, ovf: 0, drop: 0, clip: 0.4, sat: 0, floor: null }, bySf: [] } })]);
  p.parts.tbeams.show();
  await p.settle();
  await pick(p, 'cmd:sdr');
  assert.match(p.text('tb-sheet'), /Clipping 0\.40 % of samples in the last 5 s Noise floor not reported/);
  assert.doesNotMatch(p.html('tb-sheet'), /saturated/);
});

test('turning an SDR off, on and starting it again answers with a toast, and a gain is asked then confirmed by its header', async () => {
  const { p, app, done } = sdrRig([SDR()]);
  p.parts.tbeams.show();
  await p.settle();
  await pick(p, 'cmd:sdr');
  await p.fire('screen-tbeams', 'click', { 'data-sdroff': 'cmd:sdr' });
  assert.deepEqual(done.shift(), ['off', { key: 'cmd:sdr' }]);
  assert.equal(p.toasts.pop(), 'Turned SDR EEF522 off');
  await p.fire('screen-tbeams', 'click', { 'data-sdrrestart': 'cmd:sdr' });
  assert.deepEqual(done.shift(), ['restart', { key: 'cmd:sdr' }]);
  assert.equal(p.toasts.pop(), 'Restarted SDR EEF522');

  await p.fire('screen-tbeams', 'change', { 'data-sdrgain': 'cmd:sdr' }, { value: '20.7' });
  assert.deepEqual(done.shift(), ['gain', { key: 'cmd:sdr', gain: '20.7' }]);
  assert.equal(p.toasts.pop(), 'Asked SDR EEF522 for 20.7 dB, restarting it');
  // the app restarted it; its first header since says the new gain
  const asked = SDR({ gain: '20.7', gainAt: Date.now(), startedAt: Date.now() + 1, state: 'starting' });
  app.programs.list = [asked];
  app.onPrograms();
  assert.match(p.text('tb-sheet'), /asked for 20\.7 dB Started with --gain 20\.7 at \d\d:\d\d:\d\d\. Waiting for its header\./);
  assert.equal(p.toasts.length, 0);
  app.programs.list = [Object.assign({}, asked, { state: 'running', header: { t: Date.now() + 2, fields: Object.assign({}, asked.header.fields, { gain: '20.7' }) } })];
  app.onPrograms();
  assert.equal(p.toasts.pop(), 'SDR EEF522 is on 20.7 dB');
  assert.match(p.text('tb-sheet'), /20\.7 dB confirmed The program's header said so at/);
  assert.match(p.html('tb-sheet'), /<option value="20\.7" selected>/);
});

test('an SDR that stopped says why in its own words, and one that is off offers Turn on', async () => {
  const fatal = SDR({ state: 'stopped', fatal: '2', lastWords: 'No RTL-SDR plugged in', nextAt: Date.now() + 7600, lastExit: 'stopped, exit status: 1',
    said: [{ t: NOW, text: 'rtl_sdr: No supported devices found.' }, { t: NOW, text: 'No RTL-SDR plugged in' }] });
  const { p, app } = sdrRig([fatal], [Object.assign({}, SDR_PORT, { status: 'stopped' })]);
  p.parts.tbeams.show();
  await p.settle();
  await pick(p, 'cmd:sdr');
  assert.match(p.text('tb-list'), /SDR EEF522 No RTL-SDR plugged in SF10 · SF11 · SF12 no packets yet · Starts again in 8 s\./);
  assert.match(p.text('tb-sheet'), /No RTL-SDR plugged in\. Starts again in 8 s\. FATAL 2, exit status: 1/);
  assert.match(p.html('tb-sheet'), /<p class="outcome bad tb-why">/);

  app.programs.list = [SDR({ state: 'off', on: false })];
  app.onPrograms();
  assert.match(p.html('tb-sheet'), /<span class="mark off">off<\/span>/);
  assert.match(p.html('tb-sheet'), /<button class="primary" data-sdron="cmd:sdr">Turn on<\/button><button data-sdrrestart="cmd:sdr" disabled>Restart<\/button>/);
});

test('a stop the app made is said in plain words, with no exit status', async () => {
  const { p } = sdrRig([SDR({ lastExit: 'Restarted with gain 20.7 dB', lastExitByApp: true })]);
  p.parts.tbeams.show();
  await p.settle();
  await pick(p, 'cmd:sdr');
  assert.match(p.text('tb-sheet'), /Started again 2 times since the app started\. Restarted with gain 20\.7 dB at \d\d:\d\d:\d\d\. Packets/);
  assert.doesNotMatch(p.text('tb-sheet'), /SIGTERM|exit status/);
});

test('with no SDR at all the list says how to add one', async () => {
  const { p } = sdrRig([], []);
  p.parts.tbeams.show();
  await p.settle();
  assert.match(p.text('tb-list'), /No SDR\. To have one, add sdr = <command> to descent-ground\.conf beside the program\./);
});

test('a board that is not a receiver says what Listen heard, once it has', async () => {
  const { p } = rig();
  p.parts.tbeams.show();
  await p.settle();
  assert.doesNotMatch(p.text('tb-list'), /not a receiver/);
  await pick(p, '/dev/ttyACM0');
  await p.fire('screen-tbeams', 'click', { 'data-listen': '/dev/ttyACM0' });
  assert.match(p.text('tb-list'), /something, but not a receiver . USB 1209:5741/);
  assert.match(p.html('tb-sheet'), /<pre class="raw[^"]*">Heard something, but not a receiver:/);
});

test('which board is which: asks, waits with the seconds, and marks the board it found', async () => {
  const { o, p } = rig();
  p.parts.tbeams.show();
  await p.settle();
  const which = () => p.parts.tbeams.which().replace(/<[^>]*>/g, '');
  assert.match(which(), /Which board is which\?/);
  assert.match(which(), /Unplug one board and plug it back in/);
  await p.fire('tb-ident', 'click');
  assert.equal(o.watch, true);
  assert.match(which(), /Unplug the board, then plug it back in\. Waiting 60 s\./);
  await p.fire('tb-ident-stop', 'click');
  assert.equal(o.watch, false);
  assert.equal(p.toasts[p.toasts.length - 1], 'Stopped watching for a replug');
});

test('the board it found gets a "this one" pill on its card and the sheet', async () => {
  const { o, p, rx } = rig();
  rx('rx1', 7); rx('rx2', 9);
  o.identify = { watching: false, found: '/dev/ttyUSB1' };
  p.parts.tbeams.show();
  await p.settle();
  await p.fire('tb-ident', 'click');
  await p.every(1000, 1);
  assert.match(p.text('tb-list'), /Handheld reading it this one SF9/);
  assert.match(p.text('tb-sheet'), /the one you replugged/);
  assert.match(p.parts.tbeams.which().replace(/<[^>]*>/g, ''), /\/dev\/ttyUSB1 is the board you replugged/);
});

test('the head: name, state, the receiver line, and what can be done to it', async () => {
  const { p, rx } = rig();
  rx('rx1', 7); rx('rx2', 9);
  p.parts.tbeams.show();
  await p.settle();
  const head = p.html('tb-sheet').split('class="tb-cards"')[0];
  assert.match(head, /value="Mast"/);
  assert.match(head.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' '), /reading it Receiver 72E53C . \/dev\/ttyUSB0 . USB 10C4:EA60 . DeSCENT receiver 1\.4\.2/);
  assert.match(head, />Listen 5 s</);
  assert.match(head, />Stop reading it</);
  assert.match(head, />Forget this board</);
  assert.doesNotMatch(head, /Read this one/);
  // a board not being read: the yes is the primary, and the reset warning sits by it
  await pick(p, '/dev/ttyACM0');
  const other = p.html('tb-sheet').split('class="tb-cards"')[0];
  assert.match(other, /class="primary">Read this one</);
  assert.doesNotMatch(other, /Leave it alone/);
  assert.match(other.replace(/<[^>]*>/g, ' '), /Opening a port can reset the board on it\./);
});

test('Listen puts what it heard under the head', async () => {
  const { p, rx } = rig();
  rx('rx1', 7);
  p.parts.tbeams.show();
  await p.settle();
  await p.fire('screen-tbeams', 'click', { 'data-listen': '/dev/ttyUSB0' });
  assert.match(p.html('tb-sheet'), /<pre class="raw[^"]*">Heard something, but not a receiver:\nets Jun  8 2016<\/pre>/);
});

test('Radio card: the board\'s own header read-only, with the shared-SF note', async () => {
  const { p, rx } = rig();
  rx('rx1', 7); rx('rx2', 7);
  p.parts.tbeams.show();
  await p.settle();
  const t = p.text('tb-sheet');
  assert.match(t, /Radio Spreading factor .*Frequency 915\.0 MHz Bandwidth . CR 125 kHz . 4\/7 Sync . preamble 0x12 . 8/);
  assert.match(t, /Handheld is also on SF7\. For an LE and HP test, put one receiver on SF9 and the other on SF12\./);
  rx('rx2', 9);
  await p.ticks(1);
  assert.doesNotMatch(p.text('tb-sheet'), /is also on SF/);
});

// The user, 2026-11-01: "I need confirmation from the tbeam that it's listening to the new SF
// and not just 'asked tbeam for x SF'". The asking and the listening are app.js's (app.test.js);
// this screen only says what that tracker holds.
test('Radio card: the spreading factor row hands the ask to the app and says what the board answered', async () => {
  const { p, rx } = rig();
  rx('rx1', 7);
  p.parts.tbeams.show();
  await p.settle();
  assert.match(p.text('tb-sheet'), /Spreading factor .*The board's own report\. A change is asked of it and counts only when its header says so\. It hears only boards on this spreading factor\./);
  await p.fire('screen-tbeams', 'change', { 'data-sf': 'rx1' }, { value: '9' });
  assert.deepEqual(p.asks, [['rx1', 9]]);
  p.sfAsks.set('rx1', { sf: 9, at: NOW, state: 'asked' });
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /asked for SF9 Asked for SF9 at \d\d:\d\d:\d\d\. Waiting for the board's header\./);
  // the board's header said 9: the fleet's receiver now reports it too
  p.sfAsks.set('rx1', { sf: 9, at: NOW, state: 'heard', heardAt: NOW + 400, chip: true });
  rx('rx1', 9);
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /confirmed The board's radio chip confirmed SF9 at \d\d:\d\d:\d\d\./);
  // firmware before 1.2.2 only says what it set, not what the chip is on
  p.sfAsks.set('rx1', { sf: 9, at: NOW, state: 'heard', heardAt: NOW + 400, chip: false });
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /said, chip not read The board's header said SF9 at \d\d:\d\d:\d\d; its firmware does not read the radio chip back\./);
  p.sfAsks.set('rx1', { sf: 12, at: NOW, state: 'refused', said: 'Mast refused SF12 and stays on SF9' });
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /refused SF12 Mast refused SF12 and stays on SF9\. The board answered #DG,ERR,sf\./);
  p.sfAsks.set('rx1', { sf: 10, at: NOW, state: 'silent', said: 'No header from Mast in 4 s, so SF10 is not confirmed' });
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /SF10 not confirmed No header from Mast in 4 s, so SF10 is not confirmed\./);
  assert.doesNotMatch(p.text('tb-sheet'), /settles when its next header/);
});

test('Radio card: the radio chip\'s own report outranks the firmware and any ask', async () => {
  const { p, rx } = rig();
  rx('rx1', 9, { info: { id: 'x', f: '915.0', bw: '125.0', sf: '9', cr: '7', sync: '0x12', pre: '8', sfchip: '9', rx: '0' } });
  p.sfAsks.set('rx1', { sf: 9, at: NOW, state: 'heard', heardAt: NOW + 400, chip: true });
  p.parts.tbeams.show();
  await p.settle();
  assert.match(p.text('tb-sheet'), /not listening The board's radio chip says it is not in receive\./);
  assert.doesNotMatch(p.text('tb-sheet'), /confirmed/);
  rx('rx1', 9, { info: { id: 'x', f: '915.0', bw: '125.0', sf: '9', cr: '7', sync: '0x12', pre: '8', sfchip: '7', rx: '1' } });
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /radio on SF7 The firmware says SF9 but its radio chip is on SF7\./);
});

test('a header that has not come yet is said, not guessed', async () => {
  const { p, rx } = rig();
  rx('rx1', 7, { info: {} });
  p.parts.tbeams.show();
  await p.settle();
  const t = p.text('tb-sheet');
  assert.match(t, /Spreading factor .*not reported yet/);
  assert.match(t, /Frequency not reported/);
});

test('This session: four counters, the last line and heartbeat, the last three lines', async () => {
  const { p, rx } = rig();
  rx('rx1', 7, { unique: 1234567, packets: 1234567 + 163 + 4 });
  p.parts.tbeams.show();
  await p.settle();
  // lines the app hands every tap: the heartbeat's uptime is only there
  const feed = (text) => p.taps.forEach((fn) => fn('rx1', text, NOW));
  ['PKT,55,AA,-75.0,9.25,1787', 'HB,7440000,9965,20', '#DG,RX,v1,id=72E53C', 'PKT,55,BB,-75.0,9.25,1700'].forEach(feed);
  await p.ticks(1);
  const t = p.text('tb-sheet');
  assert.match(t, new RegExp('This session since ' + hm(SINCE)));
  assert.match(t, /New packets 1,234,567 Duplicates 163 Bad CRC 4 Radio errors 0/);
  assert.match(t, /Last line (.* )?ago\. Last heartbeat (.* )?ago, up 2 h 04 m\./);
  const raw = /<pre class="raw[^"]*">([^<]*)<\/pre>/.exec(p.html('tb-sheet').split('This session')[1])[1];
  assert.deepEqual(raw.split('\n'), ['HB,7440000,9965,20', '#DG,RX,v1,id=72E53C', 'PKT,55,BB,-75.0,9.25,1700']);
});

test('a receiver nothing was heard from says so, with the port when it did not open', async () => {
  const { o, p } = rig();
  o.ports[0].status = 'error: busy';
  o.ports[0].error = 'device busy';
  p.parts.tbeams.show();
  await p.settle();
  const t = p.text('tb-sheet');
  assert.match(t, /Port error: busy device busy/);
  assert.match(t, /nothing heard/);
  assert.match(t, /Wrong spreading factor, or not a receiver\. Listen to check\./);
});

test('Receiver firmware: both versions, "newer" only when the app has the newer one', async () => {
  const { o, p, rx } = rig();
  rx('rx1', 7); rx('rx2', 7);
  p.parts.tbeams.show();
  await p.settle();
  let t = p.text('tb-sheet');
  assert.match(t, /Receiver firmware On the board Receiver 1\.4\.2 App carries 1\.5\.0 newer/);
  assert.match(p.html('tb-sheet'), /1\.5\.0<\/span>\s*<span class="mark warn">newer/);
  o.version = '1.4.2';
  await p.ticks(1);
  await p.parts.tbeams.show();
  await p.settle();
  assert.doesNotMatch(p.text('tb-sheet'), /newer/);
  assert.match(p.text('tb-sheet'), /App carries 1\.4\.2/);
});

// The user, 2026-11-01: "tbeams flashing should also be same, no confirmation required"
test('flashing: one press of the button named for the board writes; the card says what is erased', async () => {
  const { o, p, rx } = rig();
  rx('rx1', 7);
  p.parts.tbeams.show();
  await p.settle();
  const fw = () => p.html('tb-sheet').split('Receiver firmware')[1];
  assert.match(fw(), /class="primary[^"]*" data-flash="\/dev\/ttyUSB0"[^>]*>Flash Mast to 1\.5\.0</);
  assert.match(p.text('tb-sheet'), /Erases what is on the board and writes receiver firmware 1\.5\.0 over the serial port, then asks it for .*once it is back/);
  assert.match(p.text('tb-sheet'), /Save what is on the board first/);
  assert.doesNotMatch(fw(), /data-ask=|data-cancel=|Overwrite it|confirm/);
  await p.fire('screen-tbeams', 'click', { 'data-flash': '/dev/ttyUSB0' });
  assert.deepEqual(o.flashed, [['/dev/ttyUSB0', 9]]);
  assert.match(p.text('tb-sheet'), /Flashed\. Asked for SF9, waiting for the board to come back\./);
  assert.equal(p.toasts[p.toasts.length - 1], 'Flashed Mast, asked for SF9');
});

// The user, 2026-11-03: the page said one factor and the board kept another, and they could
// not tell which one it was on. A flash's factor is an ask until the board's header says it.
const HEADER = (sf, chip, listening) => '#DG,RX,v1,id=72E53C,f=915.0,bw=125.0,sf=' + sf + ',cr=7,sync=0x12,pre=8,name=,fw=1.2.2,sfchip=' +
  (chip === undefined ? sf : chip) + ',rx=' + (listening === false ? 0 : 1);
const OLD_HEADER = (sf) => '#DG,RX,v1,id=72E53C,f=915.0,bw=125.0,sf=' + sf + ',cr=7,sync=0x12,pre=8,name=,fw=1.2.1';
async function flashed(sf) {
  const r = rig();
  r.rx('rx1', 7);
  r.hub.flash = async (port, asked) => { r.o.flashed.push([port, asked]); return { backup: '', sf: asked }; };
  r.p.parts.tbeams.show();
  await r.p.settle();
  await r.p.fire('screen-tbeams', 'change', { 'data-flashsf': '/dev/ttyUSB0' }, { value: String(sf) });
  await r.p.fire('screen-tbeams', 'click', { 'data-flash': '/dev/ttyUSB0' });
  // The board reboots: its port comes back under a new key, its own name wiped by the
  // flash, and the app gives back the name it last reported (serial.rs)
  r.o.ports[0] = { key: 'rx3', port: '/dev/ttyUSB0', usb: '10c4:ea60', status: 'open', rxId: '72E53C', name: 'Mast', rxFw: '1.2.1' };
  r.feed = (text, t) => r.p.taps.forEach((fn) => fn('rx3', text, t || NOW));
  return r;
}

test('after a flash the factor is asked for until the board\'s own header says it, and the board keeps its last name', async () => {
  const { o, p, feed } = await flashed(12);
  assert.deepEqual(o.flashed, [['/dev/ttyUSB0', 12]]);
  assert.match(p.text('tb-sheet'), /Flashed\. Asked for SF12, waiting for the board to come back\./);
  assert.match(p.html('tb-sheet'), /tb-last warn/);
  // the boot header: the firmware's own SF9, and the ask now also stands for the receiver row
  feed(HEADER(9));
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /Flashed\. Asked for SF12, waiting for its header to say so\./);
  assert.equal(p.sfAsks.get('rx3').state, 'asked');
  assert.match(p.text('tb-list'), /Mast/);
  assert.doesNotMatch(p.text('tb-sheet'), /comes up on|confirmed/);
  // the app asked it, and the board answers with its header
  feed(HEADER(12), NOW + 2500);
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /Flashed\. SF12 confirmed at \d\d:\d\d:\d\d\./);
  assert.doesNotMatch(p.html('tb-sheet'), /tb-last warn|tb-last bad/);
  assert.equal(p.toasts[p.toasts.length - 1], 'Mast is listening on SF12');
  assert.equal(p.sfAsks.get('rx3').state, 'heard');
});

test('a board whose header never shows the asked factor is not confirmed, with what it said', async () => {
  const { p, feed } = await flashed(12);
  feed(HEADER(9), NOW - 31000);
  await p.every(1000, 1);
  assert.match(p.text('tb-sheet'), /Flashed\. SF12 not confirmed: the board's header still says SF9\./);
  assert.match(p.html('tb-sheet'), /tb-last bad/);
  assert.equal(p.toasts[p.toasts.length - 1], "Mast: SF12 not confirmed: the board's header still says SF9");
  // a later header that does say it changes nothing: the wait is over
  feed(HEADER(12));
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /SF12 not confirmed/);
});

test('after a flash, firmware that does not read the chip back is taken at its word, and says so', async () => {
  const { p, feed } = await flashed(12);
  feed(OLD_HEADER(9));
  feed(OLD_HEADER(12), NOW + 2500);
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /Flashed\. SF12 said by the board at \d\d:\d\d:\d\d; its firmware does not read the radio chip back\./);
  assert.equal(p.toasts[p.toasts.length - 1], 'Mast says it is on SF12');
});

test('after a flash, a chip not in receive or on another factor is not confirmed', async () => {
  const { p, feed } = await flashed(12);
  feed(HEADER(12, 12, false), NOW - 31000);
  await p.every(1000, 1);
  assert.match(p.text('tb-sheet'), /Flashed\. SF12 not confirmed: the board's radio is not in receive\./);
  const again = await flashed(12);
  again.feed(HEADER(12, 9), NOW - 31000);
  await again.p.every(1000, 1);
  assert.match(again.p.text('tb-sheet'), /Flashed\. SF12 not confirmed: the board's header still says SF9\./);
});

test('a board that refuses the factor says so', async () => {
  const { p, feed } = await flashed(12);
  feed(HEADER(9));
  feed('#DG,ERR,sf');
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /Flashed\. The board refused SF12\./);
  assert.equal(p.toasts[p.toasts.length - 1], 'Mast refused SF12');
});

test('while it flashes the progress replaces the button and the card says being flashed', async () => {
  const { o, hub, p, rx } = rig();
  rx('rx1', 7);
  let done;
  hub.flash = () => new Promise((r) => { done = r; });
  p.parts.tbeams.show();
  await p.settle();
  const flashing = p.fire('screen-tbeams', 'click', { 'data-flash': '/dev/ttyUSB0' });
  await p.settle();
  assert.match(p.text('tb-list'), /Mast being flashed/);
  assert.doesNotMatch(p.html('tb-sheet'), /data-flash=/);
  assert.match(p.text('tb-sheet'), /writing the firmware/);
  done({ backup: '' });
  await flashing;
  assert.doesNotMatch(p.text('tb-list'), /being flashed/);
});

test('a board that has not been claimed is not offered a flash', async () => {
  const { p } = rig();
  p.parts.tbeams.show();
  await p.settle();
  await pick(p, '/dev/ttyUSB1');
  assert.match(p.text('tb-sheet'), /Press Read this one first\./);
  assert.doesNotMatch(p.html('tb-sheet'), /data-flash=/);
});

test('nothing plugged in', async () => {
  const { o, p } = rig({ boards: [], ports: [] });
  p.parts.tbeams.show();
  await p.settle();
  assert.match(p.text('tb-list'), /Plugged in . 0 boards USB serial Nothing plugged in\./);
  assert.match(p.text('tb-sheet'), /Plug a T-Beam in\./);
});

test('the watch card is in index.html under the list, and the head sentence is whole', async () => {
  const html = fs.readFileSync(path.join(__dirname, '..', 'web', 'index.html'), 'utf8');
  const pane = /<section id="screen-tbeams"[\s\S]*?<div id="tb-sheet"/.exec(html)[0];
  assert.ok(pane.indexOf('id="tb-list"') < pane.indexOf('Which board is which?'), 'card under the list');
  assert.ok(pane.indexOf('Which board is which?') < pane.indexOf('class="note tb-foot"'), 'card above the foot note');
  assert.match(pane, /Unplug one board and plug it back in\. The app watches for it and opens its sheet so you can name it\./);
  assert.match(pane, /<button id="tb-ident"[^>]*>Start watching<\/button>/);
  assert.match(pane, /<button id="tb-ident-stop" hidden>Stop<\/button>/);
  assert.doesNotMatch(pane, /pane-head/);

  const { p, rx } = rig();
  rx('rx1', 7);
  p.parts.tbeams.show();
  await p.settle();
  assert.match(p.text('tb-sheet'), /The name is kept on the board\. Listen opens the port for 5 s and says what it heard\./);
});

// The user, 2026-11-16: "allow changing carrier frequency on the tbeam as well"
function tunableRig() {
  const r = rig();
  r.o.ports = r.o.ports.map((p) => Object.assign({}, p, { role: 'receiver' }));
  r.o.freqs = [];
  r.hub.setFreq = async (key, mhz) => { r.o.freqs.push([key, mhz]); };
  r.rx('rx1', 9);
  r.feed = (text) => r.p.taps.forEach((fn) => fn('rx1', text, NOW + 300));
  return r;
}
const FREQ = (f) => '#DG,RX,v1,id=72E53C,f=' + f + ',bw=125.0,sf=9,cr=7,sync=0x12,pre=8,name=,fw=1.2.2,sfchip=9,rx=1';

test('Radio card: a T-Beam\'s frequency is typed, sent as an ask, and confirmed only by its header', async () => {
  const { o, p, rx, feed } = tunableRig();
  p.parts.tbeams.show();
  await p.settle();
  assert.match(p.html('tb-sheet'), /<input type="text" inputmode="decimal" class="tb-freq" data-freq="rx1" data-keep="freq:rx1" value="915.0"/);
  assert.match(p.text('tb-sheet'), /Frequency MHz The board's own report\. Type 902\.0 to 928\.0 MHz to change it/);
  await p.fire('screen-tbeams', 'change', { 'data-freq': 'rx1' }, { value: ' 903.5 ' });
  assert.deepEqual(o.freqs, [['rx1', '903.5']]);
  assert.match(p.text('tb-sheet'), /asked for 903\.5 MHz Asked at \d\d:\d\d:\d\d\. Waiting for the board's header\./);
  // a header from before the command still says the old value, and settles nothing
  feed(FREQ('915.0'));
  assert.match(p.text('tb-sheet'), /asked for 903\.5 MHz/);
  feed(FREQ('903.5'));
  rx('rx1', 9, { info: { id: 'x', f: '903.5', bw: '125.0', sf: '9', cr: '7', sync: '0x12', pre: '8' } });
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /confirmed The board's header said 903\.5 MHz at \d\d:\d\d:\d\d\./);
  assert.match(p.html('tb-sheet'), /data-freq="rx1"[^>]*value="903.5"/);
  assert.equal(p.toasts[p.toasts.length - 1], 'Mast is on 903.5 MHz');
});

test('Radio card: a frequency outside the band or off the 0.1 MHz steps is not sent, and says why', async () => {
  const { o, p } = tunableRig();
  p.parts.tbeams.show();
  await p.settle();
  for (const [typed, why] of [['868.1', 'it has to be 902.0 to 928.0 MHz, the US band'], ['915.05', 'it goes in 0.1 MHz steps'], ['abc', 'it is not a number of MHz']]) {
    await p.fire('screen-tbeams', 'change', { 'data-freq': 'rx1' }, { value: typed });
    assert.match(p.text('tb-sheet'), new RegExp(typed.replace('.', '\\.') + ' MHz not sent ' + typed.replace('.', '\\.') + ' MHz not sent: ' + why + '\\.'));
  }
  assert.deepEqual(o.freqs, []);
});

test('Radio card: a T-Beam that refuses the frequency says so plainly', async () => {
  const { p, feed } = tunableRig();
  p.parts.tbeams.show();
  await p.settle();
  await p.fire('screen-tbeams', 'change', { 'data-freq': 'rx1' }, { value: '927.9' });
  feed('#DG,ERR,freq');
  await p.ticks(1);
  assert.match(p.text('tb-sheet'), /refused 927\.9 MHz Mast refused 927\.9 MHz\. The board answered #DG,ERR,freq\./);
});

test('Radio card: the frequency of a port not sorted as a receiver, or an SDR, is shown and never sent', async () => {
  const { o, p } = tunableRig();
  o.ports[0] = Object.assign({}, o.ports[0], { role: 'unknown' });
  p.parts.tbeams.show();
  await p.settle();
  assert.doesNotMatch(p.html('tb-sheet'), /data-freq=/);
  assert.match(p.text('tb-sheet'), /Frequency 915\.0 MHz/);
  await p.fire('screen-tbeams', 'change', { 'data-freq': 'rx1' }, { value: '903.5' });
  o.ports[0] = Object.assign({}, o.ports[0], { role: 'receiver', rxKind: 'sdr' });
  await p.fire('screen-tbeams', 'change', { 'data-freq': 'rx1' }, { value: '903.5' });
  assert.deepEqual(o.freqs, []);
  assert.match(p.toasts[p.toasts.length - 1], /Not sent: rx1 is not a T-Beam being read/);
});
