// tools/compare_pages.js, the gate 2 script, on a log in test/data and a stream made from
// the 1.3 model's own rows. That shows the script reads both sides and finds a difference;
// whether the app agrees is for the master's fixture.
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const C = require('../tools/compare_pages.js');

const LOG = path.join(__dirname, 'data', 'receiver.txt');
let hasRef = true;
try { execFileSync('git', ['cat-file', '-e', 'da42787:ground/web/js/fleet.js'], { cwd: __dirname, stdio: 'ignore' }); } catch (e) { hasRef = false; }

// What the app would send at the end of the log, if it agreed with fleet.js exactly
function streamOf(old) {
  const f = old.fleet;
  const msgs = [];
  for (const [key, r] of f.receivers) {
    msgs.push({ kind: 'receiver', id: key, packets: r.packets, dups: r.packets - r.unique - r.badCrc, badCrc: r.badCrc, errors: r.radioErrors, lastLine: r.lastT });
  }
  for (const u of f.allUnits().values()) {
    const row = Object.assign({}, u, { kind: 'unit', state: f.state(u, old.end), issues: f.issues(u) });
    delete row.history; delete row.rxHistory; delete row.gaps;
    msgs.push(JSON.parse(JSON.stringify(row)));
  }
  msgs.push({ kind: 'snapshot-end' });
  return msgs.map((m, i) => Object.assign({ seq: i + 1, t: old.end }, m));
}

test('the old page and a stream that agrees with it compare equal, unit by unit', { skip: hasRef ? false : 'da42787 is not in this clone' }, () => {
  const old = C.oldRun(fs.readFileSync(LOG, 'utf8'), 'receiver.txt');
  assert.ok(old.fleet.allUnits().size >= 2, 'the log has units');
  const res = C.compare(old, C.newRun(streamOf(old)));
  assert.strictEqual(res.differs, 0, C.print(res, true, []).join('\n'));
  assert.strictEqual(res.units.length, old.fleet.allUnits().size);
  assert.match(C.print(res, false, []).join('\n'), /every compared field equal/);
});

test('a field that differs is named, with both values', { skip: hasRef ? false : 'da42787 is not in this clone' }, () => {
  const old = C.oldRun(fs.readFileSync(LOG, 'utf8'), 'receiver.txt');
  const msgs = streamOf(old);
  const one = msgs.find((m) => m.kind === 'unit');
  one.missed += 1;
  const two = msgs.filter((m) => m.kind === 'unit')[1];
  msgs.splice(msgs.indexOf(two), 1);
  const res = C.compare(old, C.newRun(msgs));
  const a = res.units.find((u) => u.label === String(one.label));
  assert.deepStrictEqual(a.differs, ['missed', 'rxPercent']);
  const b = res.units.find((u) => u.label === String(two.label));
  assert.deepStrictEqual(b.differs, ['(missing on the new side)']);
  const text = C.print(res, true, []).join('\n');
  assert.match(text, new RegExp('≠ missed\\s+' + (one.missed - 1) + '\\s+' + one.missed));
  assert.match(text, /field\(s\) differ/);
});
