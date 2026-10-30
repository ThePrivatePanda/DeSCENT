#!/usr/bin/env node
// Gate 2 for 1.4: the same session log through the 1.3 page's own model and through the
// app's stream, unit rows side by side.
//
//   node tools/compare_pages.js <session.log> <stream-fixture.jsonl> [--ref da42787] [--diff]
//
// Old side: packet.js, lines.js, fleet.js and recorder.js as they were at the ref (git show),
// fed the log the way 1.3's replay fed it (all of it, nothing trimmed), aged at its last line.
// New side: the fixture's messages (one JSON message a line, the app's stream for that log)
// through web/js/stream.js. Exit 0 when every compared field is equal, 1 when any differs.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const GROUND = path.join(__dirname, '..');
const S = require(path.join(GROUND, 'web', 'js', 'stream.js'));

// The 1.3 page's model, out of git into a folder of its own so its requires find each other
function oldModel(ref) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'compare-pages-'));
  for (const f of ['packet.js', 'lines.js', 'fleet.js', 'recorder.js']) {
    const src = execFileSync('git', ['show', ref + ':ground/web/js/' + f], { cwd: GROUND, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    fs.writeFileSync(path.join(dir, f), src);
  }
  const load = (f) => require(path.join(dir, f));
  return { L: load('lines.js'), F: load('fleet.js'), R: load('recorder.js'), dir };
}

// As app.js 1.3 opened a file: parseLogFile with today's midnight for time-of-day logs,
// then every line into a Fleet with the replay's config.
function oldRun(text, name, ref) {
  const { L, F, R } = oldModel(ref || 'da42787');
  const midnight = new Date(); midnight.setHours(0, 0, 0, 0);
  const events = R.mergeFiles([R.parseLogFile(text, name, midnight.getTime())]);
  const fleet = new F.Fleet(Object.assign({}, F.DEFAULTS, { maxPoints: Infinity, retainMin: 0 }));
  for (const e of events) fleet.ingest(L.parseLine(e.text), e.t, e.rx);
  const end = events.length ? events[events.length - 1].t : 0;
  return { fleet, end, lines: events.length };
}

// The fixture as a page would take it off the socket
function newRun(messages) {
  const c = new S.StreamClient('fixture', { keepHistory: false, cfg: { maxPoints: Infinity, retainMin: 0 } });
  c.opened();
  for (const m of messages) c.handle(m);
  if (c.next) c.handle({ kind: 'snapshot-end' });   // a fixture cut before its snapshot ended
  return { fleet: c.fleet, gaps: c.gaps, ready: c.ready };
}

const r6 = (v) => (Number.isFinite(v) ? Math.round(v * 1e6) / 1e6 : v);
// One unit as the fields gate 2 compares. state and issues the old side works out at the
// log's last line; the new side has them from the app.
function rowOf(f, u, end) {
  const d = u.latest || {};
  const v = d.values || {};
  return {
    csid: u.csid, live: u.live, boot: u.boot === undefined ? null : u.boot, fromChip: !!u.fromChip,
    firstT: u.firstT, lastT: u.lastT,
    packets: u.packets, missed: u.missed, resets: u.resets, repeats: u.repeats,
    lastCounter: u.lastCounter === undefined ? null : u.lastCounter,
    intervalMs: r6(u.intervalMs), rxPercent: r6(f.rxPercent(u)),
    bestRssi: r6(u.bestRssi), bestSnr: r6(u.bestSnr), saturatedCount: u.saturatedCount,
    receptions: (u.receptions || []).map((r) => r.rx + ':' + r6(r.rssi) + '/' + r6(r.snr)).join(' '),
    crcOk: d.crcOk, counter: v.counter, validity: v.validity, battery: r6(v.battery), saturated: !!d.saturated,
    lastValid: JSON.stringify(Object.fromEntries(Object.entries(u.lastValid || {}).sort().map(([k, x]) => [k, r6(x)]))),
    state: f.state(u, end), issues: (f.issues(u) || []).join(', '),
  };
}

function rxOf(r) {
  return { packets: r.packets, unique: r.unique, badCrc: r.badCrc, radioErrors: r.radioErrors };
}

const same = (a, b) => {
  const blank = (x) => x === null || x === undefined || (typeof x === 'number' && Number.isNaN(x));
  if (blank(a) && blank(b)) return true;
  if (typeof a === 'number' && typeof b === 'number') return Math.abs(a - b) <= 1e-9 * Math.max(1, Math.abs(a));
  return a === b;
};

// {units: [{label, old, new, differs: [field]}], receivers: [...], differs: n}
function compare(oldSide, newSide) {
  const of = oldSide.fleet; const nf = newSide.fleet;
  const labels = [...new Set([...of.allUnits().keys(), ...nf.allUnits().keys()])]
    .sort((a, b) => (parseInt(a, 10) - parseInt(b, 10)) || a.localeCompare(b));
  const units = labels.map((label) => {
    const ou = of.allUnits().get(label); const nu = nf.allUnits().get(label);
    const o = ou ? rowOf(of, ou, oldSide.end) : null;
    const n = nu ? rowOf(nf, nu, oldSide.end) : null;
    const keys = Object.keys(o || n);
    const differs = !o || !n ? ['(missing on the ' + (o ? 'new' : 'old') + ' side)'] : keys.filter((k) => !same(o[k], n[k]));
    return { label, old: o, new: n, differs, points: [ou ? ou.history.t.length : null, nu ? nu.history.t.length : null] };
  });
  const keys = [...new Set([...of.receivers.keys(), ...nf.receivers.keys()])].sort();
  const receivers = keys.map((rx) => {
    const o = of.receivers.get(rx) ? rxOf(of.receivers.get(rx)) : null;
    const n = nf.receivers.get(rx) ? rxOf(nf.receivers.get(rx)) : null;
    const differs = !o || !n ? ['(missing on the ' + (o ? 'new' : 'old') + ' side)'] : Object.keys(o).filter((k) => !same(o[k], n[k]));
    return { rx, old: o, new: n, differs };
  });
  const differs = units.reduce((a, u) => a + u.differs.length, 0) + receivers.reduce((a, r) => a + r.differs.length, 0);
  return { units, receivers, differs };
}

const cell = (v) => (v === null || v === undefined ? '—' : typeof v === 'number' && Number.isNaN(v) ? 'NaN' : String(v));

function print(res, onlyDiff, out) {
  const w = (s) => out.push(s);
  for (const u of res.units) {
    if (onlyDiff && !u.differs.length) continue;
    w('');
    w('unit ' + u.label + (u.differs.length ? '   DIFFERS: ' + u.differs.join(', ') : '   equal') +
      '   graph points old ' + cell(u.points[0]) + ', new ' + cell(u.points[1]));
    const keys = Object.keys(u.old || u.new);
    const width = Math.max(...keys.map((k) => k.length));
    for (const k of keys) {
      const o = u.old ? cell(u.old[k]) : '—'; const n = u.new ? cell(u.new[k]) : '—';
      const bad = u.differs.includes(k);
      if (onlyDiff && !bad) continue;
      w((bad ? '≠ ' : '  ') + k.padEnd(width) + '  ' + o.padEnd(28) + '  ' + n);
    }
  }
  w('');
  for (const r of res.receivers) {
    w('receiver ' + r.rx + (r.differs.length ? '   DIFFERS: ' + r.differs.join(', ') : '   equal') + '   old ' +
      (r.old ? JSON.stringify(r.old) : '—') + '   new ' + (r.new ? JSON.stringify(r.new) : '—'));
  }
  w('');
  const n = res.units.length;
  w(n + (n === 1 ? ' unit, ' : ' units, ') + res.receivers.length + ' receivers: ' +
    (res.differs ? res.differs + ' field(s) differ' : 'every compared field equal'));
  return out;
}

function main(argv) {
  const args = argv.filter((a) => !a.startsWith('--'));
  const at = argv.indexOf('--ref');
  const ref = at >= 0 ? argv[at + 1] : 'da42787';
  const files = args.filter((a) => a !== ref);
  if (files.length !== 2) {
    process.stderr.write('usage: node tools/compare_pages.js <session.log> <stream-fixture.jsonl> [--ref da42787] [--diff]\n');
    return 2;
  }
  const [log, fixture] = files;
  const old = oldRun(fs.readFileSync(log, 'utf8'), path.basename(log), ref);
  const msgs = fs.readFileSync(fixture, 'utf8').split(/\r?\n/).filter((l) => l.trim()).map((l, i) => {
    try { return JSON.parse(l); } catch (e) { throw new Error(fixture + ':' + (i + 1) + ': ' + e.message); }
  });
  const neu = newRun(msgs);
  const out = [
    'old: ' + log + ' through ' + ref + ' packet.js + lines.js + fleet.js, ' + old.lines + ' lines, aged at ' + new Date(old.end).toISOString(),
    'new: ' + fixture + ', ' + msgs.length + ' messages, ' + (neu.ready ? 'snapshot ended' : 'NO snapshot-end') +
      (neu.gaps ? ', ' + neu.gaps + ' seq gap(s)' : ''),
  ];
  const res = compare(old, neu);
  process.stdout.write(print(res, argv.includes('--diff'), out).join('\n') + '\n');
  return res.differs ? 1 : 0;
}

if (require.main === module) process.exitCode = main(process.argv.slice(2));

module.exports = { oldRun, newRun, compare, rowOf, print, main };
