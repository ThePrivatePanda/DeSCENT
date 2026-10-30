// Sensor bench card on a ChipSat's sheet: the bench console per part, the receivers' tally,
// the starts and the raw console. chipsats.js draws its head and says which board is up.
(function () {
  'use strict';
  const A = window.DGApp;
  const { $, esc, toast } = A.ui;
  const { paint, mark } = A.shell;
  const BN = DG.bench;
  const hub = A.hub;
  if (!hub.native) return;

  // order the board starts them in
  const ORDER = ['IMU', 'ENV', 'GPS', 'SOC', 'BUS', 'FLASH', 'RADIO', 'TX', 'CYC'];
  const NAMES = {
    IMU: 'IMU (BNO085)', ENV: 'Barometer (BME280)', GPS: 'GPS (MAX-M10S)', SOC: 'Fuel gauge (MAX17048)',
    BUS: 'I2C bus', FLASH: 'Flash (W25Q16)', RADIO: 'Radio', TX: 'Transmissions', CYC: 'Heartbeat',
  };
  // event lines (bus clear, quarantine), shown as printed instead of parsed
  const VERBATIM = ['BUS'];
  const RAW_SHOWN = 400;

  const state = {
    row: null, port: '',   // the bench board on the sheet and its console, from chipsats.js
    sessions: new Map(),   // board key of the console port -> DG.bench.Session
    radio: new BN.Radio(),
    paused: null,          // raw lines frozen at Pause
    filter: '',            // tag, show only its raw lines
  };

  const boards = () => (hub.boards || []).filter((b) => b.usb !== '1209:5741');   // never the drone's flight controller
  const boardOf = (port) => boards().find((b) => b.port === port) || null;
  function session(key) {
    if (!state.sessions.has(key)) state.sessions.set(key, new BN.Session());
    return state.sessions.get(key);
  }
  // Lines come in under the open port's key (rx1...). The app's boards list has no key,
  // so it is looked up in hub.ports by the port's path.
  function keyOf(port) {
    const open = (hub.ports || []).find((p) => p.port === port);
    if (open) return open.key;
    const b = boardOf(port);
    return b ? b.key : undefined;
  }
  function sessionOn(port) {
    const key = port ? keyOf(port) : undefined;
    return key !== undefined ? state.sessions.get(key) || null : null;
  }
  // the port of these that last printed the log format
  function talking(list) {
    let best = null;
    for (const b of list) {
      const s = state.sessions.get(keyOf(b.port));
      if (s && s.heard > s.other && (!best || s.lastT > best.s.lastT)) best = { b, s };
    }
    return best ? best.b.port : '';
  }

  // Every port, whichever board is up: receiver packets go to the radio count, log format
  // lines to that port's session
  A.taps.push((rx, text, t) => {
    if (state.radio.add(t, rx, text)) return;
    if (state.sessions.has(rx) || BN.parse(text)) session(rx).add(t, text);
  });

  const secs = (ms) => (ms === null || ms === undefined ? '' : ms < 10000 ? (ms / 1000).toFixed(1) + ' s' : Math.round(ms / 1000) + ' s');
  const clock = (t) => new Date(t).toLocaleTimeString([], { hour12: false });
  const fieldsOf = (f) => Object.keys(f || {}).map((k) => '<span class="fv"><span class="k">' + esc(k) + '</span>=' + esc(f[k]) + '</span>').join(' ');
  // reset cause flags that were set
  const causeWords = (c) => (c ? Object.keys(c).filter((k) => k !== 'csr' && c[k] === '1').join(', ') || 'none set' : 'not printed');

  // test=bench or not is chipsats.js's to say, beside what the app last read
  function banners(s) {
    if (!s) return '';
    const now = Date.now();
    let out = '';
    const loop = s.looping(now);
    if (loop) out += '<p class="outcome bad inpane">Restarting over and over: ' + loop + ' starts in the last minute.</p>';
    if (s.lastT !== null && now - s.lastT > BN.GAP_MS) out += '<p class="outcome bad inpane">Nothing on the console for ' + esc(secs(now - s.lastT)) + '.</p>';
    return out;
  }

  function starts(s) {
    if (!s || (!s.boots.length && !s.gaps.length)) return '';
    const rows = [
      ...s.boots.map((b) => ({ t: b.t, html: '<div class="boot"><span class="when">' + esc(clock(b.t)) + '</span> ' +
        mark('warn', b.unseen ? 'started again (its start line was lost)' : 'started') +
        (b.upMs !== null ? ' after ' + esc(secs(b.upMs)) + ' up' : ' (first seen)') +
        ' <span class="note">cause: ' + esc(causeWords(b.cause)) + '</span>' +
        (b.before ? '<div class="hex muted">last before it: ' + esc(b.before) + '</div>' : '') + '</div>' })),
      ...s.gaps.map((g) => ({ t: g.to, html: '<div class="boot"><span class="when">' + esc(clock(g.to)) + '</span> ' +
        mark('off', 'the port went quiet for ' + secs(g.to - g.from)) + ' <span class="note">not a restart</span></div>' })),
    ].sort((a, b) => b.t - a.t);
    return '<div class="card"><h3>Starts (' + s.boots.length + ')</h3>' + rows.slice(0, 30).map((r) => r.html).join('') + '</div>';
  }

  function card(p, now) {
    const m = p.main();
    const q = p.quiet(now);
    const said = p.lastOk === null ? '' : p.lastOk ? mark('good', 'ok=1') : mark('bad', 'ok=0');
    const rate = m ? p.rate(m.name) : null;
    return '<div class="card part' + (q.gone ? ' gone' : '') + '">' +
      '<h3>' + esc(NAMES[p.tag] || p.tag) + ' ' + said + (q.gone ? ' ' + mark('off', 'no line for ' + secs(q.ms)) : '') + '</h3>' +
      (p.init ? '<div class="line"><span class="l">init</span> ' + (p.init.ok === '1' ? mark('good', 'ok=1') : p.init.ok === '0' ? mark('bad', 'ok=0') : '') + ' ' + fieldsOf(p.init) + '</div>' : '') +
      '<div class="line"><span class="l">lines</span> ' + p.ok + ' ok=1, ' + p.failed + ' ok=0' +
        (p.errors ? ', ' + p.errors + ' E' : '') + (p.warnings ? ', ' + p.warnings + ' W' : '') +
        (rate !== null ? ', ' + rate.toFixed(1) + '/s by the board\'s clock' : '') +
        (q.ms !== null ? ', last ' + esc(secs(q.ms)) + ' ago' : '') + '</div>' +
      (p.lastFail ? '<div class="line"><span class="l">last ok=0</span> ' + esc(clock(p.lastFail.t)) + (p.lastFail.reason ? ' reason=' + esc(p.lastFail.reason) : '') +
        '<div class="hex muted">' + esc(p.lastFail.text) + '</div></div>' : '') +
      (p.lastReason && (!p.lastFail || p.lastReason.text !== p.lastFail.text) ? '<div class="line"><span class="l">last reason</span> ' +
        esc(clock(p.lastReason.t)) + ' reason=' + esc(p.lastReason.reason) + '<div class="hex muted">' + esc(p.lastReason.text) + '</div></div>' : '') +
      (m && m.name !== 'init' && !VERBATIM.includes(p.tag) ? '<div class="line"><span class="l">' + esc(m.name) + '</span> ' + fieldsOf(m.fields) + '</div>' : '') +
      (VERBATIM.includes(p.tag) && p.recent.length ? '<div class="line"><span class="l">newest</span>' +
        p.recent.slice().reverse().map((l) => '<div class="hex muted">' + esc(clock(l.t) + '  ' + l.text) + '</div>').join('') + '</div>' : '') +
      '</div>';
  }

  // Every bench board sends as 254, so this is whichever is on the air, not only this one.
  function radioCard(now) {
    const rows = [...state.radio.by].map(([rx, r]) => {
      const last = (a) => (a.length ? a[a.length - 1] : null);
      return '<tr><td>' + esc(A.rxName ? A.rxName(rx) : rx) + '</td><td>' + r.heard + '</td><td>' + r.missed + '</td><td>' + r.restarts +
        '</td><td>' + r.badCrc + '</td><td>' + esc(String(last(r.rssi) ?? '')) + '</td><td>' + esc(String(last(r.snr) ?? '')) +
        '</td><td>' + esc(r.t ? secs(now - r.t) + ' ago' : '') + '</td></tr>';
    }).join('');
    return '<div class="card"><h3>Heard by the receivers from any board on the sensor bench (' + BN.BENCH_CSID + ')</h3>' +
      (rows ? '<table class="grid"><thead><tr><th>Receiver</th><th>Heard</th><th>Missed</th><th>Counter restarts</th><th>Bad CRC</th><th>RSSI</th><th>SNR</th><th>Last</th></tr></thead><tbody>' + rows + '</tbody></table>'
        : '<p class="empty">No receiver has heard the bench yet. It keeps its radio off for the first 30 s after it starts.</p>') + '</div>';
  }

  function raw(s) {
    const lines = state.paused || (s ? s.raw : []);
    const shown = lines.filter((r) => !state.filter || (r.line && r.line.tag === state.filter)).slice(-RAW_SHOWN);
    const tags = s ? [...new Set(s.raw.filter((r) => r.line).map((r) => r.line.tag))].sort() : [];
    const pick = '<select data-filter="1" aria-label="Show only"><option value="">every line</option>' +
      tags.map((t) => '<option value="' + esc(t) + '"' + (t === state.filter ? ' selected' : '') + '>' + esc(t) + '</option>').join('') + '</select>';
    const text = shown.map((r) => {
      const cls = r.line && r.line.tag === 'BOOT' && r.line.event === 'start' ? 'boot' : r.line && r.line.level === 'E' ? 'bad' : r.line && r.line.level === 'W' ? 'warn' : '';
      return '<span class="' + cls + '">' + esc(clock(r.t) + '  ' + r.text) + '</span>';
    }).join('\n');
    return '<div class="card"><h3>Console, as it arrived</h3><div class="line">' + pick +
      ' <label class="check"><input type="checkbox" data-pause="1"' + (state.paused ? ' checked' : '') + '> Pause</label>' +
      ' <span class="note">' + (s ? s.heard + ' lines, ' + s.other + ' not in the log format' : '') + '</span></div>' +
      '<pre class="raw">' + (text || '<span class="muted">nothing yet</span>') + '</pre></div>';
  }

  function render() {
    const el = $('cs-bench');
    if (!el || !state.row) return;
    const s = sessionOn(state.port);
    const now = Date.now();
    const parts = s ? [...s.parts.values()].sort((x, y) => {
      const i = (p) => (ORDER.indexOf(p.tag) === -1 ? 99 : ORDER.indexOf(p.tag));
      return i(x) - i(y) || x.tag.localeCompare(y.tag);
    }) : [];
    paint(el, banners(s) +
      (parts.length ? '<div class="parts">' + parts.map((p) => card(p, now)).join('') + '</div>' : '') +
      radioCard(now) + starts(s) + raw(s));
  }

  const root = $('screen-chipsats');
  root.addEventListener('click', async (e) => {
    const at = (n) => e.target.getAttribute(n);
    if (at('data-listen')) {
      try { await hub.setBoard(at('data-listen'), true); } catch (err) { toast('Could not read ' + at('data-listen') + ': ' + (err.message || err)); }
    } else if (at('data-radio')) {
      try { await hub.benchRadio(at('data-radio')); toast('Sent radio on/off to the bench'); } catch (err) { toast(err.message || String(err)); }
    } else if (at('data-clear')) {
      const key = state.port ? keyOf(state.port) : undefined;
      if (key !== undefined) state.sessions.delete(key);
      state.radio = new BN.Radio();
      state.paused = null;
    } else return;
    render();
  });

  root.addEventListener('change', (e) => {
    const t = e.target;
    if (t.getAttribute('data-filter')) state.filter = t.value;
    else if (t.getAttribute('data-pause')) {
      const s = sessionOn(state.port);
      state.paused = t.checked && s ? s.raw.slice() : null;
    } else return;
    render();
  });

  // chipsats.js says which board is up after each of its own redraws, which can put a new,
  // empty #cs-bench on the page, so this draws at once as well as on the timer
  A.benchPane = {
    show(row, port) {
      if (!row || (state.row && state.row.uid !== row.uid)) state.paused = null;
      state.row = row;
      state.port = port || '';
      render();
    },
    sessionOn, talking,
  };
  setInterval(() => { if (state.row && A.shell.current() === 'chipsats') render(); }, 500);
})();
