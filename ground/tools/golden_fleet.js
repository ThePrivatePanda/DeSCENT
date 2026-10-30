// fleet.js over a list of logs, replayed as app.js replays an opened log, for holding
// against app/src/fleet.rs (fleet::tests::golden writes the same lines).
//   node golden_fleet.js dump <file of paths> <out.jsonl>
//   node golden_fleet.js compare <js.jsonl> <rust.jsonl>
// Each file goes through recorder.js parseLogFile and mergeFiles, then lines.js and
// fleet.js event by event, once with the replay settings (replayCfg: no cap, no
// retention) and once with a small cap and retention, so trimming is exercised too.
// Rows, retired runs, receivers and the summary are written at a quarter, half, three
// quarters and the end of each file, and every run's history at the end.
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');
const web = path.join(__dirname, '..', 'web', 'js');

// Midnight on a fixed day, so a file without times gets the same ones both sides
const BASE_MS = 1793232000000;

function configs(F) {
  return [
    ['replay', Object.assign({}, F.DEFAULTS, { maxPoints: Infinity, retainMin: 0 })],
    ['capped', Object.assign({}, F.DEFAULTS, { maxPoints: 2500, retainMin: 10 })],
  ];
}

function row(f, u, t) {
  const { history, rxHistory, ...rest } = u;
  return Object.assign(rest, {
    state: f.state(u, t), age: t - u.lastT, issues: f.issues(u), rxPercent: f.rxPercent(u),
    staleAfterMs: f.staleAfterMs(u), runs: f.earlierRuns(u.csid).length + 1,
  });
}

function dump(list, out) {
  const L = require(path.join(web, 'lines.js'));
  const R = require(path.join(web, 'recorder.js'));
  const F = require(path.join(web, 'fleet.js'));
  const fd = fs.openSync(out, 'w');
  const w = (o) => fs.writeSync(fd, JSON.stringify(o) + '\n');
  for (const p of fs.readFileSync(list, 'utf8').split('\n').filter(Boolean)) {
    const events = R.mergeFiles([R.parseLogFile(fs.readFileSync(p, 'utf8'), path.basename(p), BASE_MS)]);
    // An empty log is never replayed: app.js stops at "No lines found"
    if (!events.length) continue;
    const n = events.length;
    const at = [...new Set([1, 2, 3, 4].map((k) => Math.floor((n * k) / 4)))].filter((x) => x > 0);
    for (const [name, cfg] of configs(F)) {
      w({ file: p, cfg: name, events: n });
      const f = new F.Fleet(cfg);
      let pos = 0;
      for (const [k, stop] of at.entries()) {
        while (pos < stop) {
          const e = events[pos++];
          f.ingest(L.parseLine(e.text), e.t, e.rx);
        }
        if (name === 'capped' && k === 1) f.setConfig(cfg);
        const t = events[pos - 1].t;
        w({ at: pos, t });
        for (const r of f.rows(t)) w({ row: row(f, r.unit, t) });
        w({ attention: f.rows(t, 'attention').map((r) => r.unit.label) });
        for (const u of f.retired.values()) w({ retired: row(f, u, t) });
        for (const r of f.receivers.values()) w({ receiver: r });
        w({ summary: f.summary(t) });
        w({ totals: { total: f.total, duplicates: f.duplicates, badCrc: f.badCrc, badLength: f.badLength, lastRetired: f.lastRetired === undefined ? null : f.lastRetired } });
      }
      for (const [label, u] of f.allUnits()) w({ label, history: u.history, rxHistory: u.rxHistory });
    }
  }
  fs.closeSync(fd);
}

// Paths where a and b differ, as 'row.latest.values.lat'. Numbers must be the same double.
function diff(a, b, at, out) {
  if (a !== null && b !== null && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a); const kb = Object.keys(b);
    if (ka.join() !== kb.join()) out.push((at || '.') + ' keys');
    for (const k of ka) if (k in b) diff(a[k], b[k], at ? at + '.' + k : k, out);
  } else if (!Object.is(a, b)) {
    out.push(at || '.');
  }
  return out;
}

async function compare(jsPath, rustPath) {
  const open = (p) => readline.createInterface({ input: fs.createReadStream(p), crlfDelay: Infinity })[Symbol.asyncIterator]();
  const js = open(jsPath); const rs = open(rustPath);
  const n = { files: 0, runs: 0, snapshots: 0, rows: 0, retired: 0, receivers: 0, histories: 0, historyPoints: 0, mismatches: 0 };
  const units = new Set();
  const classes = new Map();
  let where = ''; let file = ''; let cfg = '';
  for (;;) {
    const [x, y] = await Promise.all([js.next(), rs.next()]);
    if (x.done || y.done) {
      if (x.done !== y.done) { n.mismatches++; classes.set('one output is longer', [{ where, js: x.value, rust: y.value }]); }
      break;
    }
    const a = JSON.parse(x.value); const b = JSON.parse(y.value);
    const kind = Object.keys(a)[0];
    if (kind === 'file') { n.runs++; if (a.cfg === 'replay') n.files++; file = a.file; cfg = a.cfg; where = file + ' ' + cfg; }
    if (kind === 'at') { n.snapshots++; where = file + ' ' + cfg + ' @' + a.at; }
    if (kind === 'row') n.rows++;
    if (kind === 'retired') n.retired++;
    if (kind === 'receiver') n.receivers++;
    if (kind === 'label') {
      n.histories++;
      n.historyPoints += a.history.t.length;
      if (cfg === 'replay') units.add(file + '\t' + a.label);
    }
    const d = diff(a, b, '', []);
    if (d.length) {
      n.mismatches++;
      const key = kind + ': ' + d.slice(0, 6).join(' ');
      if (!classes.has(key)) classes.set(key, []);
      if (classes.get(key).length < 3) classes.get(key).push({ where, js: x.value.slice(0, 1500), rust: y.value.slice(0, 1500) });
    }
  }
  n.units = units.size;
  console.log(JSON.stringify(n));
  for (const [key, ex] of classes) {
    console.log('\nMISMATCH ' + key);
    for (const e of ex) console.log('  ' + e.where + '\n    js   ' + e.js + '\n    rust ' + e.rust);
  }
  process.exit(n.mismatches ? 1 : 0);
}

const [mode, a, b] = process.argv.slice(2);
if (mode === 'dump') dump(a, b);
else if (mode === 'compare') compare(a, b);
else { console.error('node golden_fleet.js dump <list> <out> | compare <js> <rust>'); process.exit(2); }
