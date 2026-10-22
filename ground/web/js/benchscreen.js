// Bench screen: sensor bench console per part, raw console and restarts
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
  const FW_WORDS = { bench: 'sensor bench', flight: 'flight software', dump: 'dump firmware', unknown: 'not recognised' };

  const state = {
    chips: [], sel: null,
    sessions: new Map(),   // board key of the console port -> DG.bench.Session
    radio: new BN.Radio(),
    paused: null,          // raw lines frozen at Pause
    filter: '',            // tag, show only its raw lines
    confirm: false,        // confirm strip is up
    timer: null,
  };

  const store = {
    get(k) { try { return localStorage.getItem(k); } catch (e) { return null; } },
    set(k, v) { try { localStorage.setItem(k, v); } catch (e) { /* not kept */ } },
  };
  const boards = () => (hub.boards || []).filter((b) => b.usb !== '1209:5741');   // never the drone's flight controller
  const chip = () => state.chips.find((c) => c.uid === state.sel) || null;
  const nameOf = (c) => (c && (c.name || 'board …' + c.uid.slice(-6))) || '';
  // Same console the ChipSats screen kept, else the last port printing the log format, else any free port
  function consolePort(c) {
    const ports = boards().map((b) => b.port);
    const kept = c ? store.get('dg-console-' + c.uid) : null;
    if (kept && ports.includes(kept)) return kept;
    let talking = null;
    for (const b of boards()) {
      const s = state.sessions.get(b.key);
      if (s && s.heard > s.other && (!talking || s.lastT > talking.s.lastT)) talking = { b, s };
    }
    if (talking) return talking.b.port;
    const free = boards().find((b) => b.state !== 'receiver');
    return free ? free.port : '';
  }
  const boardOf = (port) => boards().find((b) => b.port === port) || null;
  function session(key) {
    if (!state.sessions.has(key)) state.sessions.set(key, new BN.Session());
    return state.sessions.get(key);
  }

  // Receiver packets go to the radio count, log format lines to that port's session
  A.taps.push((rx, text, t) => {
    if (state.radio.add(t, rx, text)) return;
    if (state.sessions.has(rx) || BN.parse(text)) session(rx).add(t, text);
  });

  const secs = (ms) => (ms === null || ms === undefined ? '' : ms < 10000 ? (ms / 1000).toFixed(1) + ' s' : Math.round(ms / 1000) + ' s');
  const clock = (t) => new Date(t).toLocaleTimeString([], { hour12: false });
  const fieldsOf = (f) => Object.keys(f || {}).map((k) => '<span class="fv"><span class="k">' + esc(k) + '</span>=' + esc(f[k]) + '</span>').join(' ');
  // reset cause flags that were set
  const causeWords = (c) => (c ? Object.keys(c).filter((k) => k !== 'csr' && c[k] === '1').join(', ') || 'none set' : 'not printed');

  function side() {
    if (!state.chips.length) return '<p class="empty">No ChipSat known yet. Read one on the ChipSats screen first.</p>';
    return state.chips.map((c) => '<button class="item' + (c.uid === state.sel ? ' on' : '') + '" data-pick="' + esc(c.uid) + '">' +
      '<b>' + esc(nameOf(c)) + '</b>' +
      (c.firmware === 'bench' ? '<small>' + mark('bad', 'sensor bench') + '</small>' : '') + '</button>').join('');
  }

  function head(c, port, s) {
    const b = boardOf(port);
    const reading = b && b.state === 'receiver';
    const options = boards().map((x) => '<option value="' + esc(x.port) + '"' + (x.port === port ? ' selected' : '') + '>' +
      esc(x.port + (x.name ? ' (' + x.name + ')' : '')) + '</option>').join('');
    const fw = c.firmware ? FW_WORDS[c.firmware] || c.firmware : 'not known';
    const job = A.shell.jobs.chip;
    const running = job && job.running;
    const put = state.confirm
      ? '<div class="confirm"><span class="grow">Writes the sensor bench over whatever is on <b>' + esc(nameOf(c)) +
        '</b>. Nothing is saved first. It is not flight software: write flight software from the ChipSats screen before it flies.</span>' +
        '<button data-benchgo="1" class="primary">Write the sensor bench</button><button data-benchcancel="1">Cancel</button></div>'
      : '<button data-bench="1"' + (running ? ' disabled' : '') + '>Put the sensor bench on it</button>';
    return '<div class="pane"><div class="rows">' +
      '<div><span class="l">Board</span><span class="val">' + esc(nameOf(c)) + '</span> ' +
        (c.firmware === 'bench' ? mark('bad', 'sensor bench, not flight software') : '<span class="note">last written: ' + esc(fw) + '</span>') + '</div>' +
      '<div><span class="l">Console</span><select data-console="' + esc(c.uid) + '" aria-label="Console port">' +
        (options || '<option value="">no serial port plugged in</option>') + '</select> ' +
        (port && !reading ? '<button data-listen="' + esc(port) + '">Read this port</button>' : '') +
        (reading ? mark('good', 'being read') : '') + '</div>' +
      '<div>' + put + ' <button data-radio="' + esc(port) + '"' + (reading ? '' : ' disabled') + '>Radio on/off</button>' +
        ' <button data-clear="1">Clear this record</button></div>' +
      (running ? '<div><span class="note">' + esc((job.note || 'writing') + (job.pct ? ', ' + job.pct + '%' : '')) + '</span></div>' : '') +
      '</div></div>' + banners(s);
  }

  function banners(s) {
    if (!s) return '';
    const now = Date.now();
    let out = '';
    const loop = s.looping(now);
    if (loop) out += '<p class="outcome bad inpane">Restarting over and over: ' + loop + ' starts in the last minute.</p>';
    if (s.bench === false) out += '<p class="outcome bad inpane">The board did not say test=bench when it started: this is not the sensor bench.</p>';
    if (s.lastT !== null && now - s.lastT > BN.GAP_MS) out += '<p class="outcome bad inpane">Nothing on the console for ' + esc(secs(now - s.lastT)) + '.</p>';
    return out;
  }

  function starts(s) {
    if (!s || (!s.boots.length && !s.gaps.length)) return '';
    const rows = [
      ...s.boots.map((b) => ({ t: b.t, html: '<div class="boot"><span class="when">' + esc(clock(b.t)) + '</span> ' +
        mark(b.unseen ? 'warn' : 'bad', b.unseen ? 'started again (its start line was lost)' : 'started') +
        (b.upMs !== null ? ' after ' + esc(secs(b.upMs)) + ' up' : ' (first seen)') +
        ' <span class="note">cause: ' + esc(causeWords(b.cause)) + '</span>' +
        (b.before ? '<div class="hex muted">last before it: ' + esc(b.before) + '</div>' : '') + '</div>' })),
      ...s.gaps.map((g) => ({ t: g.to, html: '<div class="boot"><span class="when">' + esc(clock(g.to)) + '</span> ' +
        mark('off', 'the port went quiet for ' + secs(g.to - g.from)) + ' <span class="note">not a restart</span></div>' })),
    ].sort((a, b) => b.t - a.t);
    return '<div class="pane"><h3>Starts (' + s.boots.length + ')</h3>' + rows.slice(0, 30).map((r) => r.html).join('') + '</div>';
  }

  function card(p, now) {
    const m = p.main();
    const q = p.quiet(now);
    const said = p.lastOk === null ? '' : p.lastOk ? mark('good', 'ok=1') : mark('bad', 'ok=0');
    const rate = m ? p.rate(m.name) : null;
    return '<div class="pane part' + (q.gone ? ' gone' : '') + '">' +
      '<h3>' + esc(NAMES[p.tag] || p.tag) + ' ' + said + (q.gone ? ' ' + mark('bad', 'no line for ' + secs(q.ms)) : '') + '</h3>' +
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

  function radioCard(now) {
    const rows = [...state.radio.by].map(([rx, r]) => {
      const last = (a) => (a.length ? a[a.length - 1] : null);
      return '<tr><td>' + esc(A.rxName ? A.rxName(rx) : rx) + '</td><td>' + r.heard + '</td><td>' + r.missed + '</td><td>' + r.restarts +
        '</td><td>' + r.badCrc + '</td><td>' + esc(String(last(r.rssi) ?? '')) + '</td><td>' + esc(String(last(r.snr) ?? '')) +
        '</td><td>' + esc(r.t ? secs(now - r.t) + ' ago' : '') + '</td></tr>';
    }).join('');
    return '<div class="pane"><h3>Heard by the receivers, as board ' + BN.BENCH_CSID + '</h3>' +
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
    return '<div class="pane"><h3>Console, as it arrived</h3><div class="line">' + pick +
      ' <label class="check"><input type="checkbox" data-pause="1"' + (state.paused ? ' checked' : '') + '> Pause</label>' +
      ' <span class="note">' + (s ? s.heard + ' lines, ' + s.other + ' not in the log format' : '') + '</span></div>' +
      '<pre class="raw">' + (text || '<span class="muted">nothing yet</span>') + '</pre></div>';
  }

  function render() {
    paint($('bn-side'), side());
    const c = chip();
    if (!c) { paint($('bn-sheet'), '<p class="empty">Pick a ChipSat.</p>'); return; }
    const port = consolePort(c);
    const b = boardOf(port);
    const s = b ? state.sessions.get(b.key) : null;
    const now = Date.now();
    const parts = s ? [...s.parts.values()].sort((x, y) => {
      const i = (p) => (ORDER.indexOf(p.tag) === -1 ? 99 : ORDER.indexOf(p.tag));
      return i(x) - i(y) || x.tag.localeCompare(y.tag);
    }) : [];
    paint($('bn-sheet'), head(c, port, s) +
      (parts.length ? '<div class="parts">' + parts.map((p) => card(p, now)).join('') + '</div>' : '') +
      radioCard(now) + starts(s) + raw(s));
  }

  async function load() {
    try { state.chips = await hub.chips(); } catch (e) { state.chips = []; }
    if (!state.sel && state.chips.length) state.sel = state.chips[0].uid;
    render();
  }

  async function writeBench() {
    const c = chip();
    if (!c) return;
    state.confirm = false;
    try {
      await hub.chipFlash({ uid: c.uid, embedded: 'bench', module: c.module, backup: false });
      toast('Writing the sensor bench on ' + nameOf(c));
      A.shell.pollJobs();
    } catch (e) {
      toast('Did not write the sensor bench: ' + (e.message || e));
    }
    render();
  }

  $('screen-bench').addEventListener('click', async (e) => {
    const t = e.target;
    const at = (n) => t.getAttribute(n);
    if (at('data-pick')) { state.sel = at('data-pick'); state.confirm = false; }
    else if (at('data-bench')) state.confirm = true;
    else if (at('data-benchcancel')) state.confirm = false;
    else if (at('data-benchgo')) { await writeBench(); return; }
    else if (at('data-listen')) {
      try { await hub.setBoard(at('data-listen'), true); } catch (err) { toast('Could not read ' + at('data-listen') + ': ' + (err.message || err)); }
    } else if (at('data-radio')) {
      try { await hub.benchRadio(at('data-radio')); toast('Sent radio on/off to the bench'); } catch (err) { toast(err.message || String(err)); }
    } else if (at('data-clear')) {
      const b = boardOf(consolePort(chip()));
      if (b) state.sessions.delete(b.key);
      state.radio = new BN.Radio();
      state.paused = null;
    } else return;
    render();
  });

  $('screen-bench').addEventListener('change', (e) => {
    const t = e.target;
    if (t.getAttribute('data-console')) store.set('dg-console-' + t.getAttribute('data-console'), t.value);
    else if (t.getAttribute('data-filter')) state.filter = t.value;
    else if (t.getAttribute('data-pause')) {
      const b = boardOf(consolePort(chip()));
      const s = b ? state.sessions.get(b.key) : null;
      state.paused = t.checked && s ? s.raw.slice() : null;
    } else return;
    render();
  });

  A.shell.register('bench', {
    show() {
      load();
      clearInterval(state.timer);
      state.timer = setInterval(render, 500);
    },
    hide() { clearInterval(state.timer); state.timer = null; },
  });
})();
