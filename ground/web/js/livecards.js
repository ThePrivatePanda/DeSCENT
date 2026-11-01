// Live and Replay as html: the unit cards, the summary and issues line, the detail head
// and reading columns, the receiver rows and the Runs tab. No DOM and no page state, so
// node can test it; app.js decides what is shown and paints it.
(function (root, factory) {
  // the validity bits and field groups; the packet itself is decoded in the app
  const P = typeof require === 'function' ? require('./stream.js') : root.DG.stream;
  const mod = factory(P);
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else root.DGLive = mod;
})(typeof self !== 'undefined' ? self : this, function (P) {
  'use strict';

  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const mark = (cls, word) => '<span class="mark ' + cls + '">' + esc(word) + '</span>';

  // One number format for the page, whatever the browser's locale
  const formats = new Map();
  function num(v, d) {
    if (v === null || v === undefined || !Number.isFinite(Number(v))) return '—';
    const k = d || 0;
    if (!formats.has(k)) formats.set(k, new Intl.NumberFormat('en-US', { minimumFractionDigits: k, maximumFractionDigits: k }));
    return formats.get(k).format(Number(v));
  }

  // "0.2 s", "48 s", "6m 12s", "2h 05m"
  function ago(ms) {
    if (!Number.isFinite(ms)) return '—';
    const s = Math.max(0, ms) / 1000;
    if (s < 10) return s.toFixed(1) + ' s';
    if (s < 60) return Math.floor(s) + ' s';
    if (s < 3600) return Math.floor(s / 60) + 'm ' + Math.floor(s % 60) + 's';
    return Math.floor(s / 3600) + 'h ' + String(Math.floor((s % 3600) / 60)).padStart(2, '0') + 'm';
  }
  const clock = (t) => new Date(t).toTimeString().slice(0, 8);

  const STATE = { OK: ['good', 'OK'], STALE: ['off', 'Stale'], LOST: ['bad', 'Lost'] };
  const stateMark = (st) => mark(STATE[st][0], STATE[st][1]);
  const runsWord = (n) => n + (n === 1 ? ' run' : ' runs');

  function bitsHtml(mask) {
    return '<span class="bits">' + P.VALIDITY_BITS.map((b) => {
      const on = (mask >> b.bit) & 1;
      return '<span class="' + (on ? '' : 'off') + '" title="' + b.name + (on ? ' valid' : ' NOT valid') + '">' + b.short + '</span>';
    }).join('') + '</span>';
  }

  // ---------- the unit list ----------
  // The spreading factor the last packet came in on. Each reception holds its own, so this
  // stays true after a receiver is retuned; empty until a receiver has sent its header.
  const heardSf = (u) => [...new Set(u.receptions.map((r) => r.sf).filter(Boolean))].map((n) => 'SF' + n).join(', ');

  // o: {runs, staleMs, sel, cfg: {lowBatteryV, weakRssiDbm}, heardBy}
  function unitCardHtml(u, state, age, rxTotal, o) {
    const d = u.latest;
    const sf = heardSf(u);
    const cfg = o.cfg || {};
    const frac = Math.min(1, age / o.staleMs);
    const width = age > o.staleMs ? 100 : Math.max(2, frac * 100);
    const batt = d.valid.soc ? d.values.battery : null;
    const low = batt !== null && batt < cfg.lowBatteryV;
    const weak = Number.isFinite(u.bestRssi) && u.bestRssi < cfg.weakRssiDbm;
    return '<div class="ucard' + (o.sel ? ' card sel' : '') + '" tabindex="0" data-keep="card-' + u.csid + '">' +
      '<div class="u1"><span class="csid"><small>CSID</small>' + u.csid + '</span>' + stateMark(state) +
        '<button class="runs-pill" data-runs="' + u.csid + '" data-keep="runs-' + u.csid + '" title="A run ends when the board restarts and its counter starts again. Opens the Runs tab">' +
          runsWord(o.runs) + '</button>' +
        (d.saturated ? mark('warn', 'saturated') : '') +
        '<span class="ago" title="Stale after ' + num(o.staleMs / 1000, 1) + ' s without a packet">' + ago(age) + ' ago</span></div>' +
      '<div class="stalebar"><b class="' + state.toLowerCase() + '" style="width:' + width.toFixed(0) + '%"></b></div>' +
      '<div class="u3"><span' + (low ? ' class="low" title="Below ' + num(cfg.lowBatteryV, 2) + ' V, the low battery limit"' : '') + '><b>' + num(batt, 2) + '</b> V</span><i>·</i>' +
        '<span' + (weak ? ' class="low" title="Below ' + cfg.weakRssiDbm + ' dBm, the weak RF limit"' : '') + '><b>' + num(u.bestRssi, 1) + '</b> dBm</span><i>·</i>' +
        (sf ? '<span title="Last packet came in on this"><b>' + sf + '</b></span><i>·</i>' : '') +
        '<span><b>' + num(u.packets, 0) + '</b> pkts</span>' +
        '<span class="rxn"' + (o.heardBy ? ' title="' + esc(o.heardBy) + '"' : '') + '>' + u.receptions.length + ' of ' + rxTotal + ' rx</span></div>' +
      '<div class="u4">' + bitsHtml(d.values.validity) + '</div>' +
    '</div>';
  }

  // One row per board, worst first, a card in each. o: {sort, sel, rxName}
  function fleetHtml(f, t, o) {
    const rxTotal = [...f.receivers.values()].filter((r) => r.packets > 0).length;
    // An earlier run picked on the Runs tab still marks its board's card
    const selCsid = o.sel === null || o.sel === undefined ? NaN : parseInt(o.sel, 10);
    return f.rows(t, o.sort).map(({ unit: u, state, age }) => {
      const heardBy = o.rxName ? u.receptions.map((r) => o.rxName(r.rx) + ': ' + num(r.rssi, 1) + ' dBm').join('\n') : '';
      return '<tr data-label="' + u.label + '"><td>' + unitCardHtml(u, state, age, rxTotal, {
        runs: f.earlierRuns(u.csid).length + 1, staleMs: f.staleAfterMs(u), sel: u.csid === selCsid, cfg: f.cfg, heardBy,
      }) + '</td></tr>';
    }).join('');
  }

  function issuesText(f, t) {
    const s = f.summary(t);
    const out = [[s.lost, 'lost'], [s.stale, 'stale'], [s.lowBattery, 'low battery'], [s.badCrc, 'bad CRC'],
      [s.weakRf, 'weak RF'], [s.saturated, 'saturated'], [s.validityIssues, 'with invalid sensors'],
      [s.resets, s.resets === 1 ? 'reset' : 'resets']]
      .filter(([n]) => n > 0).map(([n, w]) => num(n, 0) + ' ' + w);
    return out.length ? out.join(' · ') : 'No issues';
  }

  // linesRead is given in a replay, where the packets are counted on the bar
  function summaryHtml(f, t, linesRead) {
    const s = f.summary(t);
    const more = linesRead === undefined ? num(s.packets, 0) + ' packets' : num(linesRead, 0) + ' lines read';
    return '<b>' + s.heard + (s.heard === 1 ? ' unit' : ' units') + '</b> <span>· ' + more + '</span>';
  }

  // ---------- the detail panel ----------
  const row = (label, value, unit, cls) => '<div class="row' + (cls ? ' ' + cls : '') + '"><span class="l">' + label +
    '</span><span class="v">' + value + (unit ? ' <small>' + unit + '</small>' : '') + '</span></div>';

  // A field whose validity bit is clear shows the last trusted value, or a dash
  function field(u, cfg, label, key, dec, unit) {
    const fd = P.FIELDS.find((x) => x.key === key);
    const d = u.latest;
    if (d.valid[fd.group]) {
      const sat = fd.group === 'accel' && Math.abs(d.values[key]) >= cfg.saturationMps2;
      return row(label, sat ? '<span class="sat" title="Near the accelerometer limit">' + num(d.values[key], dec) + '</span>' : num(d.values[key], dec), unit);
    }
    const lv = u.lastValid[key];
    return row(label, '<span title="Not valid in this packet' + (lv !== undefined ? '. Last trusted value shown.' : '') + '">' +
      (lv !== undefined ? num(lv, dec) : '—') + '</span>', unit);
  }

  function group(title, rows, off) {
    return '<section class="group' + (off ? ' off' : '') + '"><h3>' + title + (off ? '<span class="kicker bad">Not valid</span>' : '') + '</h3>' +
      rows.join('') + '</section>';
  }

  // o: {state, t, run, runs, rx: [{name, rssi, snr}], cfg: {saturationMps2}, replay}
  function detailHtml(u, o) {
    if (!u) return '<p class="none">Select a unit in the list.</p>';
    const d = u.latest;
    const cfg = o.cfg || {};
    const v = d.valid;
    const when = o.replay ? 'Packet at <b>' + clock(u.lastT) + '</b>'
      : 'Last packet <b>' + clock(u.lastT) + '</b> · ' + ago(o.t - u.lastT) + ' ago';
    const head = '<div class="uhead"><span class="csid"><small>CSID</small>' + u.csid + '</span>' + stateMark(o.state) +
      (d.saturated ? mark('warn', 'accel saturated') : '') +
      '<span class="when">' + when + ' · run ' + o.run + ' of ' + o.runs +
        (o.runs > 1 ? ' <button class="ghost" data-joinruns="' + u.csid + '" title="Opens the Runs tab">join runs</button>' : '') + '</span>' +
      '<div class="validity-line">' + bitsHtml(d.values.validity) +
        '<span class="hex">0x' + d.values.validity.toString(16).toUpperCase().padStart(2, '0') + '</span></div></div>';
    const crc = d.crcOk
      ? '<span class="good" title="' + (u.latestSource === 'csv' ? 'Old CSV receiver: packet rebuilt from the printed values to check the CRC' : 'CRC matches') + '">pass</span>'
      : '<span class="bad">fail</span>';
    const expected = u.packets + u.missed;
    const link = [
      row('Counter', num(d.values.counter, 0)),
      row('Spreading factor', heardSf(u) || '—'),
      row('Cell voltage', v.soc ? num(d.values.battery, 2) : '—', 'V'),
      row('Interval', Number.isFinite(u.intervalMs) ? num(u.intervalMs / 1000, 2) : '—', 's'),
      row('CRC', crc, Number.isFinite(d.crcReceived) ? d.crcReceived.toString(16).toUpperCase().padStart(4, '0') : ''),
      row('Packets', num(u.packets, 0)), row('Missed', num(u.missed, 0)), row('Resets', num(u.resets, 0)),
      row('Received', expected ? num(100 * u.packets / expected, 1) : '—', '%'),
    ].concat((o.rx || []).map((r) => row(esc(r.name), num(r.rssi, 1) + ' / ' + num(r.snr, 2), 'dBm/dB')));
    const axes = (pre, dec, unit) => ['x', 'y', 'z'].map((a) => field(u, cfg, a.toUpperCase(), pre + a, dec, unit));
    const cols = [
      [group('Link', link)],
      [group('GPS', [field(u, cfg, 'Latitude', 'lat', 6, '°'), field(u, cfg, 'Longitude', 'lon', 6, '°'), field(u, cfg, 'Altitude MSL', 'gpsAlt', 1, 'm')], !v.gps),
        group('Environment', [field(u, cfg, 'Temperature', 'temp', 2, '°C'), field(u, cfg, 'Pressure', 'pressure', 1, 'hPa'),
          field(u, cfg, 'Humidity', 'humidity', 1, '%'), field(u, cfg, 'Pressure altitude', 'envAlt', 1, 'm')], !v.env)],
      [group('Accelerometer', axes('a', 2, 'm/s²'), !v.accel), group('Gyroscope', axes('g', 1, 'deg/s'), !v.gyro)],
      [group('Magnetometer', axes('m', 1, 'µT'), !v.mag),
        group('Orientation', [['i', 'qi'], ['j', 'qj'], ['k', 'qk'], ['real', 'qr']].map(([l, k]) => field(u, cfg, l, k, 4, '')), !v.quat)],
    ];
    return head + '<div class="readings">' + cols.map((c) => '<div class="col">' + c.join('') + '</div>').join('') + '</div>';
  }

  // ---------- receivers ----------
  // The word for a receiver: its port's own trouble first, then whether lines arrive
  function receiverState(r, port, t) {
    if (port && port.status !== 'open') return { word: port.status, cls: 'bad' };
    if (r && r.lastT !== null && t - r.lastT < 7000) return { word: 'live', cls: 'good' };
    if (r && r.lastT !== null) return { word: 'quiet ' + ago(t - r.lastT), cls: 'off' };
    return { word: 'nothing heard', cls: 'warn' };
  }

  // x: {key, name, id, sf, packets, word, cls, index, detail, sel, controls, meta, extra}
  // The detail is the tooltip, and is written out under the selected row for a touch screen.
  function receiverRowHtml(x) {
    const meta = x.meta !== undefined ? esc(x.meta)
      : [x.sf ? 'SF' + x.sf : '', num(x.packets || 0, 0) + ' pkts'].filter(Boolean).join(' · ');
    return '<div class="rx' + (x.sel ? ' sel' : '') + '" data-rx="' + esc(x.key) + '"' + (x.detail ? ' title="' + esc(x.detail) + '"' : '') + '>' +
      '<button class="rx-name" data-rx="' + esc(x.key) + '" aria-pressed="' + !!x.sel + '">' +
        (x.index === undefined ? '' : '<span class="sw" style="background:var(--series-' + (x.index + 1) + ')"></span>') +
        '<b>' + esc(x.name) + '</b>' + (x.id && !String(x.name).includes(x.id) ? ' <small>' + esc(x.id) + '</small>' : '') + '</button>' +
      '<span class="rx-meta">' + meta + '</span>' + mark(x.cls, x.word) + (x.extra || '') +
      (x.sel && (x.detail || x.controls) ? '<div class="rx-more">' +
        (x.detail ? '<span class="note">' + esc(x.detail).replace(/\n/g, ' · ') + '</span>' : '') + (x.controls || '') + '</div>' : '') +
    '</div>';
  }

  // ---------- runs ----------
  // A board's counter starts again from 0 every time it restarts, so its data is kept as
  // separate runs. This is where they are put back together when the restart was an
  // accident, and thrown away when the run was rubbish. runs: oldest first, live last.
  // o: {sel, ask}; ask is the run whose delete is waiting for its second click.
  function runsHtml(csid, runs, o) {
    const earlier = runs.filter((u) => !u.live).length;
    const head = '<p class="note">CSID ' + csid + ' has ' + (runs.length === 1 ? 'one run' : runs.length + ' runs') +
      '. A run ends when the board restarts and its counter goes back to 0. ' +
      (earlier ? 'If a restart was an accident, join the runs back together.' : 'Nothing to join.') + '</p>' +
      (earlier > 1 ? '<p><button id="runs-joinall" class="primary">Join all ' + runs.length + ' runs into one</button></p>' : '');
    const cards = runs.map((u) => {
      const span = u.firstT === null ? '—' : ago(u.lastT - u.firstT);
      const when = u.firstT === null ? '' : ' · ' + clock(u.firstT) + ' to ' + clock(u.lastT);
      const asking = o.ask === u.label;
      return '<div class="runcard card' + (u.label === o.sel ? ' ring' : '') + '">' +
        '<div class="runhead"><button class="runlabel" data-pick="' + u.label + '" title="Show this run">' + u.label + '</button>' +
          (u.boot !== null && u.boot !== undefined ? '<span class="note">boot ' + esc(u.boot) + '</span>' : '') +
          (u.live ? mark('good', 'transmitting now') : mark('off', 'earlier run')) + '</div>' +
        '<div class="note">' + num(u.packets, 0) + ' packets · ' + num(u.missed, 0) + ' missed · ran for ' + span + when + '</div>' +
        '<div class="acts">' +
          (u.live ? '' : '<button data-merge="' + u.label + '">Join into the next run</button>') +
          (asking ? '' : '<button data-drop="' + u.label + '">Delete this run</button>') + '</div>' +
        (asking ? '<div class="confirm"><span class="grow">Delete run ' + u.label + '? Its ' + num(u.packets, 0) +
          (u.packets === 1 ? ' packet goes' : ' packets go') + ' from this page. The log file keeps them.</span>' +
          '<button class="primary" data-dropgo="' + u.label + '">Delete run ' + u.label + '</button>' +
          '<button data-dropno="' + u.label + '">Keep it</button></div>' : '') +
      '</div>';
    }).join('');
    return '<div class="runs">' + head + cards + '</div>';
  }

  return {
    esc, num, ago, clock, mark, stateMark, bitsHtml, unitCardHtml, fleetHtml, issuesText, summaryHtml,
    detailHtml, receiverState, receiverRowHtml, runsHtml,
  };
});
