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
