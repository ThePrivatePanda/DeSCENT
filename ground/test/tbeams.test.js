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

// ---------- the sheet as the redesign draws it ----------
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

// fakepage keeps the live fleet to itself; this one line hands it to the hub so a test can
// put receivers in it.
function withLive(scripts) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tbeams-'));
  const file = path.join(dir, 'live.js');
  fs.writeFileSync(file, 'DGApp.hub.live = DGApp.live;');
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

test('flashing: one button named for the board, a second click that says what is erased, then progress', async () => {
  const { o, p, rx } = rig();
  rx('rx1', 7);
  p.parts.tbeams.show();
  await p.settle();
  const fw = () => p.html('tb-sheet').split('Receiver firmware')[1];
  assert.match(fw(), /class="primary[^"]*" data-ask="\/dev\/ttyUSB0"[^>]*>Flash Mast to 1\.5\.0</);
  assert.match(p.text('tb-sheet'), /Writes receiver firmware 1\.5\.0 over the serial port, then opens it again on/);
  assert.match(p.text('tb-sheet'), /Save what is on the board first/);
  await p.fire('screen-tbeams', 'click', { 'data-ask': '/dev/ttyUSB0' });
  assert.equal(o.flashed.length, 0);
  assert.match(p.text('tb-sheet'), /Erase Mast and write the receiver firmware\? Its current firmware is not saved\./);
  assert.match(fw(), /data-flash="\/dev\/ttyUSB0" class="primary">Overwrite it</);
  await p.fire('screen-tbeams', 'click', { 'data-cancel': '/dev/ttyUSB0' });
  assert.doesNotMatch(p.text('tb-sheet'), /Erase Mast/);
  await p.fire('screen-tbeams', 'click', { 'data-ask': '/dev/ttyUSB0' });
  await p.fire('screen-tbeams', 'click', { 'data-flash': '/dev/ttyUSB0' });
  assert.deepEqual(o.flashed, [['/dev/ttyUSB0', 9]]);
  assert.match(p.text('tb-sheet'), /Flashed, comes up on SF9\./);
});

test('while it flashes the progress replaces the button and the card says being flashed', async () => {
  const { o, hub, p, rx } = rig();
  rx('rx1', 7);
  let done;
  hub.flash = () => new Promise((r) => { done = r; });
  p.parts.tbeams.show();
  await p.settle();
  await p.fire('screen-tbeams', 'click', { 'data-ask': '/dev/ttyUSB0' });
  const flashing = p.fire('screen-tbeams', 'click', { 'data-flash': '/dev/ttyUSB0' });
  await p.settle();
  assert.match(p.text('tb-list'), /Mast being flashed/);
  assert.doesNotMatch(p.html('tb-sheet'), /data-ask=/);
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
  assert.doesNotMatch(p.html('tb-sheet'), /data-ask=/);
});

test('nothing plugged in', async () => {
  const { o, p } = rig({ boards: [], ports: [] });
  p.parts.tbeams.show();
  await p.settle();
  assert.match(p.text('tb-list'), /Plugged in . 0 boards USB serial Nothing plugged in\./);
  assert.match(p.text('tb-sheet'), /Plug a T-Beam in\./);
});
