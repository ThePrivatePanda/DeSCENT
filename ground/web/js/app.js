// Wires the page together: the app's stream (live) or a replay's -> panels and charts.
(function () {
  'use strict';
  const { store: S, boardmodel: B, replay: RP } = DG;
  const $ = (id) => document.getElementById(id);
  // What the other screens reach this file through. Filled in at the bottom; the hooks
  // on it (onView, onReceivers, onTransport) are set by js/screens.js once it has loaded.
  const api = {};
  const V = window.DGLive;
  // Regions redrawn on the timer go through the shell's paint, which holds a repaint while
  // a press is down so the press and its release land on one node. The shell loads after
  // this file; until it is there, the same skip when nothing changed.
  function paint(el, html) {
    if (api.shell) { api.shell.paint(el, html); return; }
    if (el.dgHtml === html) return;
    el.innerHTML = html;
    el.dgHtml = html;
  }

  // Display settings are this browser's; the model's (stale, lost, limits) are the app's
  let settings = S.loadSettings();
  const model = new S.ModelSettings({ fetch: (u, i) => fetch(u, i), onChange: () => applyModel() });
  const rxMeta = new Map();   // rxKey -> {index}
  let rxCount = 0;

  const view = {
    sel: null, paused: false, sort: 'attention', mode: 'live',
    rxSel: null,     // the receiver row opened for its SF and disconnect
    dropAsk: null,   // the run whose delete waits for its second click
    sels: { live: null, replay: null },
    replay: null,    // {r: DG.replay.Replay, names, from: {log, raw, dir}, dirty, toasted}
  };
  const graphCfg = () => ({ maxPoints: settings.maxPoints, retainMin: settings.retainMin });

  // Every line from every receiver, for panes that want them all, e.g. the sensor bench
  const taps = [];       // (rx, text, t)
  const packetTaps = []; // (packet message)
  const hub = DG.makeHub({
    cfg: graphCfg(),
    onLine: (m) => { const t = Number(m.t) || Date.now(); for (const tap of taps) tap(String(m.rx), String(m.text), t); },
    onPacket: (m) => { for (const tap of packetTaps) tap(m); },
    onSettings: (m) => model.fromStream(m),
    onChange: () => renderReceivers(),
    note: (msg) => toast(msg),
  });
  const live = hub.stream.fleet;

  // A receiver's spreading factor is the board's own word, never the page's. The firmware
  // answers a #SET,sf with its header line (#DG,RX,… sf=) or with #DG,ERR,sf; until a header
  // says the new value the ask is only an ask, and the rows say so (the user, 2026-11-01:
  // "I need confirmation from the tbeam that it's listening to the new SF").
  const sfAsks = new Map();   // rx key -> {sf, at, state: asked | heard | refused | silent | failed, was, said, heardSf, heardAt}
  const SF_WAIT_MS = 4000;    // the firmware answers at once; its 30 s header is not waited for
  function askSf(k, sf) {
    const r = live.receivers.get(k);
    const ask = { sf, at: Date.now(), state: 'asked', was: r && r.info.sf ? Number(r.info.sf) : NaN, said: '', heardSf: NaN, heardAt: 0 };
    sfAsks.set(k, ask);
    renderReceivers();
    return hub.setSf(k, sf).then(() => {
      if (ask.state === 'asked') toast('Asked ' + rxName(k) + ' for SF' + sf + ', waiting for its header');
      setTimeout(() => {
        if (ask.state !== 'asked') return;
        ask.state = 'silent';
        ask.said = Number.isFinite(ask.heardSf) ? rxName(k) + ' answered with SF' + ask.heardSf + ', not SF' + sf
          : 'No header from ' + rxName(k) + ' in ' + (SF_WAIT_MS / 1000) + ' s, so SF' + sf + ' is not confirmed';
        toast(ask.said);
        renderReceivers();
      }, SF_WAIT_MS);
    }).catch((err) => {
      ask.state = 'failed';
      ask.said = 'Could not ask ' + rxName(k) + ' for SF' + sf + ': ' + (err.message || err);
      toast(ask.said);
      renderReceivers();
    });
  }
  taps.push((rx, text, t) => {
    const ask = sfAsks.get(rx);
    if (!ask || ask.state !== 'asked') return;
    if (text.startsWith('#DG,ERR,sf')) {
      ask.state = 'refused';
      ask.said = rxName(rx) + ' refused SF' + ask.sf + (Number.isFinite(ask.was) ? ' and stays on SF' + ask.was : '');
    } else if (text.startsWith('#DG,RX,')) {
      const m = /,sf=(\d+)/.exec(text);
      const sf = m ? Number(m[1]) : NaN;
      // A header printed before the command landed still says the old value; the answer
      // itself follows at once, so a header that disagrees is left to the wait to judge.
      if (sf !== ask.sf) { ask.heardSf = sf; return; }
      ask.state = 'heard'; ask.heardAt = t;
      ask.said = rxName(rx) + ' is listening on SF' + sf;
    } else return;
    toast(ask.said);
    renderReceivers();
  });
  // The mark a receiver row carries for its last ask; none once the board has said the same
  function sfMark(k, r) {
    const ask = sfAsks.get(k);
    if (!ask) return '';
    if (ask.state === 'asked') return V.mark('warn', 'asked for SF' + ask.sf);
    if (ask.state === 'heard') return r && Number(r.info.sf) === ask.sf ? V.mark('good', 'SF' + ask.sf + ' confirmed') : '';
    return V.mark('bad', ask.state === 'refused' ? 'refused SF' + ask.sf : ask.state === 'failed' ? 'SF' + ask.sf + ' not sent' : 'SF' + ask.sf + ' not confirmed');
  }

  const fleet = () => (view.mode === 'replay' && view.replay ? view.replay.r.fleet : live);
  const client = () => (view.mode === 'replay' && view.replay ? view.replay.r.stream : hub.stream);
  // The replay's own clock only while the replay is what is on screen: with a log open
  // and Live showing, live units are aged against the real time.
  const now = () => {
    if (!(view.mode === 'replay' && view.replay)) return Date.now();
    const c = view.replay.r.clock();
    return c === null ? NaN : c;
  };
  const stamp = () => new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-');

  function meta(rx) {
    let m = rxMeta.get(rx);
    if (!m) { m = { index: rxCount++ % 8 }; rxMeta.set(rx, m); }
    return m;
  }
  // A receiver goes by its own name or its id, which stay with the board. Its key (rx1,
  // rx2) is only the order the ports were found in. A log's keys are the log's own, so
  // in a replay the boards plugged in now have no say.
  function rxName(rx) {
    const replaying = view.mode === 'replay';
    const port = replaying ? null : hub.ports.find((p) => p.key === rx);
    const board = port && hub.boards ? hub.boards.find((b) => b.port === port.port) : null;
    const name = B.receiverName(rx, port, fleet().receivers.get(rx), board);
    return name.length > 22 ? name.slice(0, 20) + '…' : name;
  }

  function toast(msg) {
    const t = $('toast'); t.textContent = msg; t.hidden = false;
    clearTimeout(toast.timer); toast.timer = setTimeout(() => { t.hidden = true; }, 3000);
  }

  function download(blob, name) {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 5000);
  }

  // ---------- input ----------
  // The app finds the receivers itself, so this is a rescan
  $('btn-connect').addEventListener('click', async () => {
    try {
      await hub.add();
      toast(hub.ports.length + (hub.ports.length === 1 ? ' receiver' : ' receivers') + ' found');
    } catch (e) {
      toast(e && e.name === 'NotFoundError' ? 'No receiver found' : 'Could not look for receivers: ' + (e.message || e));
    }
  });

  // A pull's files sit one folder down in the app's data folder, so their names carry a
  // slash. It has to stay a slash in the address, and cannot be one in the name a file
  // is saved as.
  function logUrl(name) {
    return 'api/log/' + String(name).split('/').map(encodeURIComponent).join('/');
  }
  function flatName(name) {
    return String(name).replace(/[\\/]+/g, '-');
  }

  // A log the app wrote, opened in the app by its name. more is what the caller knows
  // about it: {raw, dir}. A pull's log has the dump the board printed beside it.
  function openServerLog(name, more) {
    const pulled = /^(.*)\/ground\.log$/.exec(name);
    return openReplay({ path: name }, [name], Object.assign({ log: name, raw: pulled ? pulled[1] + '/dump.txt' : null, dir: null }, more));
  }

  window.addEventListener('beforeunload', (e) => {
    if (live.total > 0) { e.preventDefault(); e.returnValue = ''; }
  });

  // ---------- replay ----------
  // The input itself is hidden, so its two labels take the focus and the keys
  for (const l of document.querySelectorAll('label[tabindex]')) {
    if (l.control !== $('file-open')) continue;
    l.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); $('file-open').click(); }
    });
  }
  $('file-open').addEventListener('change', (e) => {
    const files = [...e.target.files];
    e.target.value = '';
    if (!files.length) return;
    openReplay({ files: files.map((f) => ({ name: f.name, size: f.size, body: f })) }, files.map((f) => f.name));
  });

  // ?log=<url> replays a file without anyone clicking Open log, so pulling a chip can
  // be one command that ends with the dashboard already showing it. Repeatable:
  // ?log=a&log=b merges them the way opening both files does.
  async function openFromUrl() {
    const urls = new URLSearchParams(location.search).getAll('log');
    if (!urls.length) return;
    const files = [];
    for (const u of urls) {
      try {
        const r = await fetch(u);
        if (!r.ok) throw new Error('HTTP ' + r.status);
        const body = await r.blob();
        files.push({ name: (u.split('?')[0].split(/[\\/]/).pop()) || 'log', size: body.size, body });
      } catch (err) {
        // A bad link leaves a working dashboard and says why, rather than a blank page.
        toast('Could not open ' + u + ': ' + (err.message || err));
      }
    }
    if (files.length) openReplay({ files }, files.map((f) => f.name));
  }
  openFromUrl();

  // Replaying and listening are different jobs and the screen says which one it is doing.
  // Live chrome goes away rather than sitting there looking clickable next to a log from
  // last week.
  // Two views over two different fleets. Switching keeps both: the receivers carry on
  // while a log is being read, and the log stays where it was while live is watched.
  function showView(mode) {
    if (mode === 'replay' && !view.replay) return;
    view.sels[view.mode] = view.sel;
    view.mode = mode;
    view.sel = view.sels[mode];
    document.body.dataset.mode = mode;
    $('view-live').setAttribute('aria-selected', String(mode === 'live'));
    $('view-replay').setAttribute('aria-selected', String(mode === 'replay'));
    $('view-replay').disabled = !view.replay;
    // The name hides on a narrow bar; the replay bar under it says it in full
    $('view-replay').innerHTML = view.replay ? 'Replay<span class="pill-file"> · ' + esc(view.replay.names.join(', ')) + '</span>' : 'Replay';
    $('view-replay').title = view.replay ? view.replay.names.join(', ') : 'Open a log, or pull one off a ChipSat';
    $('replay').hidden = mode !== 'replay';
    renderReplayFile();
    if (api.onView) api.onView(mode);
    render(true);
  }

  const blobOf = (parts) => new Blob(parts);
  function replayOpts() {
    return {
      fetch: (u, i) => fetch(u, i), location, join: blobOf,
      cfg: { model: model.values },
      // The replay's stream redraws on the next look at the replay's state, not per message
      onChange: () => { if (view.replay) view.replay.dirty = true; },
      log: (s) => console.warn(s),
    };
  }

  // The app reads the file and runs its own fleet over it. from is only given for a log
  // the app serves: {log, raw, dir}.
  async function openReplay(what, names, from) {
    let r;
    try {
      r = await RP.Replay.open(replayOpts(), what);
    } catch (e) {
      toast('Could not open ' + names.join(', ') + ': ' + (e.message || e));
      return;
    }
    closeReplay();
    view.replay = { r, names, from: from || null, dirty: true, toasted: false, polls: 0, busy: false, dragging: false };
    view.sel = null;
    view.sels.replay = null;
    showView('replay');
    pollReplay(true);
  }

  function closeReplay() {
    const rp = view.replay;
    if (!rp) return;
    rp.r.close();
    view.replay = null;
    view.sels.replay = null;
  }

  // Where the replay is, asked of the app. Four times a second while it plays or until it
  // has said, then every two seconds: a paused replay does not move.
  async function pollReplay(now) {
    const rp = view.replay;
    if (!rp || rp.busy) return;
    const st = rp.r.state;
    if (!now && !st.playing && rp.r.stream.ready && rp.toasted && rp.polls++ % 8) {
      if (rp.dirty && view.mode === 'replay') { rp.dirty = false; render(true); }
      return;
    }
    rp.busy = true;
    const was = st;
    try {
      await rp.r.poll();
    } catch (e) {
      return;
    } finally {
      rp.busy = false;
    }
    if (view.replay !== rp) return;   // closed under us
    const s = rp.r.state;
    if (!rp.from && s.kept) rp.from = s.kept;
    $('rp-pos').max = s.lines;
    if (!rp.dragging) $('rp-pos').value = s.position;
    $('rp-play').textContent = s.playing ? 'Pause' : 'Play';
    const t = rp.r.clock();
    if (!rp.dragging) $('rp-time').textContent = Number.isFinite(t) && t !== null ? V.clock(t) : '';
    if (!rp.toasted && rp.r.stream.ready) {
      rp.toasted = true;
      toast('Opened ' + rp.names.join(', ') + ': ' + V.num(s.lines) + ' lines, ' + V.num(s.packets) + ' packets');
    }
    const moved = was.position !== s.position || was.packets !== s.packets;
    if ((moved || rp.dirty) && view.mode === 'replay') { rp.dirty = false; render(true); }
  }
  setInterval(() => pollReplay(false), 250);

  async function replayDo(what) {
    const rp = view.replay;
    if (!rp) return;
    try {
      await what(rp.r);
    } catch (e) {
      toast('The replay did not answer: ' + (e.message || e));
    }
    pollReplay(true);
  }

  $('rp-play').addEventListener('click', () => replayDo(async (r) => {
    if (r.state.playing) { $('rp-play').textContent = 'Play'; await r.pause(); return; }
    $('rp-play').textContent = 'Pause';
    await r.speed($('rp-speed').value);
    if (r.state.lines && r.state.position >= r.state.lines) await r.seek(0);
    await r.play();
  }));
  $('rp-speed').addEventListener('change', (e) => replayDo((r) => r.speed(e.target.value)));
  // The app rebuilds its fleet on a seek, so it is asked once the handle is let go. Under
  // the handle meanwhile: which line it is on.
  $('rp-pos').addEventListener('input', (e) => {
    const rp = view.replay;
    if (!rp) return;
    rp.dragging = true;
    $('rp-time').textContent = 'line ' + V.num(+e.target.value) + ' of ' + V.num(rp.r.state.lines);
  });
  $('rp-pos').addEventListener('change', (e) => {
    const rp = view.replay;
    if (!rp) return;
    rp.dragging = false;
    replayDo((r) => r.seek(+e.target.value));
  });
  $('rp-exit').addEventListener('click', () => { closeReplay(); showView('live'); });

  // ---------- saving what is open ----------
  // A finished pull lands in replay, where the live chrome is hidden, so the replay bar
  // carries its own ways out.

  // Where a log the app wrote is on this computer, when the app has said. Session logs
  // and pull folders both sit beside the log being written now, which it reports.
  function onDisk(from) {
    if (from.dir) return from.dir;
    const cur = hub.log ? String(hub.log) : '';
    const cut = Math.max(cur.lastIndexOf('/'), cur.lastIndexOf('\\'));
    if (cut <= 0) return '';
    const slash = from.log.lastIndexOf('/');
    return cur.slice(0, cut) + (slash > 0 ? cur[cut] + from.log.slice(0, slash) : '');
  }

  function renderReplayFile() {
    const rp = view.replay;
    $('rp-raw').hidden = !(rp && rp.from && rp.from.raw);
    if (!rp) { $('rp-file').textContent = ''; return; }
    renderReplayProgress();
  }

  // Where the replay is and what it has read, then which file and where it is kept. The
  // receiver strip is live chrome and is hidden here, so this line is where a replay says
  // its records came off a chip and which of them have a real clock.
  function replayWords(rp) {
    const where = rp.from ? onDisk(rp.from) : '';
    let flash = 0; let dated = 0;
    for (const src of rp.r.fleet.sources.values()) {
      if (src.kind !== 'flash') continue;
      flash++;
      if (src.time_source === 'anchor') dated++;
    }
    const undated = flash - dated;
    const chip = !flash ? '' : 'Off a chip, not a radio: ' + flash + (flash === 1 ? ' boot, ' : ' boots, ') +
      dated + ' with GPS time' + (undated ? ', ' + undated + ' not dated' : '') + '.';
    const file = where ? rp.from.log.split('/').pop() + ' in ' + where : rp.names.join(', ');
    return [file, chip].filter(Boolean).join('. ');
  }
  function renderReplayProgress() {
    const rp = view.replay;
    if (!rp) return;
    const s = rp.r.state;
    const words = replayWords(rp);
    const html = '<span class="rp-prog">line ' + V.num(s.position) + ' of ' + V.num(s.lines) + ' · ' +
      V.num(s.packets) + ' packets · from ' + (s.from !== null ? V.clock(s.from) : '—') + '</span>' +
      (words ? ' <span class="rp-where">· ' + esc(words) + '</span>' : '');
    paint($('rp-file'), html);
  }

  // The open file's own name less its extension. Several files merged get a new one.
  function replayBase() {
    const rp = view.replay;
    if (rp.from) return flatName(rp.from.log).replace(/\.[^.]*$/, '');
    if (rp.names.length === 1) return flatName(rp.names[0]).replace(/\.[^.]*$/, '');
    return settings.fileTag + '-merged-' + stamp();
  }

  async function served(name) {
    const r = await fetch(logUrl(name));
    if (!r.ok) throw new Error('HTTP ' + r.status);
    return r.blob();
  }

  $('rp-save').addEventListener('click', async () => {
    const rp = view.replay;
    if (!rp) return;
    const name = replayBase() + '.log';
    try {
      if (rp.from) download(await served(rp.from.log), name);
      else {
        const r = await fetch(rp.r.logUrl);
        if (!r.ok) throw new Error('HTTP ' + r.status);
        download(await r.blob(), name);
      }
      toast('Saved ' + name);
    } catch (e) {
      toast('Could not get the log from the app: ' + (e.message || e));
    }
  });

  // The app writes the table, the same columns 1.3 wrote
  $('rp-csv').addEventListener('click', async () => {
    const rp = view.replay;
    if (!rp) return;
    let csv;
    try {
      const r = await fetch(rp.r.csvUrl);
      if (!r.ok) throw new Error('HTTP ' + r.status);
      csv = await r.text();
    } catch (e) {
      toast('Could not get the table from the app: ' + (e.message || e));
      return;
    }
    const rows = csv.split('\n').length - 2;   // less the header and the last newline
    if (rows < 1) { toast('Nothing to save: no packet in this log passes its CRC.'); return; }
    const name = replayBase() + '.csv';
    download(new Blob([csv], { type: 'text/csv' }), name);
    toast('Saved ' + V.num(rows) + ' rows to ' + name);
  });

  $('rp-raw').addEventListener('click', async () => {
    const rp = view.replay;
    if (!rp || !rp.from || !rp.from.raw) return;
    const name = flatName(rp.from.raw);
    try {
      download(await served(rp.from.raw), name);
      toast('Saved ' + name);
    } catch (e) {
      toast('Could not get ' + rp.from.raw + ': ' + (e.message || e));
    }
  });

  // ---------- fleet controls ----------
  $('sort').addEventListener('change', (e) => { view.sort = e.target.value; render(); });
  $('btn-pause').addEventListener('click', (e) => {
    view.paused = !view.paused;
    e.target.textContent = view.paused ? 'Resume' : 'Pause';
    e.target.classList.toggle('on', view.paused);
  });
  $('sel-unit').addEventListener('change', (e) => { view.sel = e.target.value; render(true); });
  $('btn-hide').addEventListener('click', () => {
    if (view.sel === null) return;
    fleet().hidden.add(Number(view.sel)); toast('Hid CSID ' + view.sel); view.sel = null; render(true);
  });
  $('btn-restore').addEventListener('click', () => {
    const n = fleet().hidden.size;
    fleet().hidden.clear();
    toast(n ? n + (n === 1 ? ' hidden unit' : ' hidden units') + ' shown again' : 'No hidden units');
    render(true);
  });
  // ---------- clearing ----------
  // Tick what should go, press Clear. The app keeps its own counts and every line on disk:
  // this clears what this page shows, and a unit still transmitting comes back with its
  // next packet. Only the new log file is asked of the app. Clear, join and delete run
  // page-side in 1.4.0; the app gets them in 1.4.1.
  const clearBoxes = { units: 'cl-units', sel: 'cl-sel', rx: 'cl-rx', hidden: 'cl-hidden', log: 'cl-log' };
  const clearWanted = () => Object.fromEntries(Object.entries(clearBoxes).map(([k, id]) => [k, $(id).checked]));

  function renderClear() {
    const w = clearWanted();
    const bits = [];
    // Only the unit shown means only that one: with none shown no unit goes, rather than
    // every unit, which is what the other box is for.
    if (w.sel) { if (view.sel) bits.push('CSID ' + view.sel); }
    else if (w.units) bits.push('all units');
    if (w.rx) bits.push('receiver counts');
    if (w.hidden) bits.push('hidden');
    if (w.log) bits.push('new log file');
    $('clear-what').textContent = bits.length ? bits.join(', ') : w.sel ? 'no unit shown' : 'nothing ticked';
    $('btn-clear').disabled = !bits.length;
  }
  for (const id of Object.values(clearBoxes)) $(id).addEventListener('change', renderClear);
  // Clear… opens one row under the head: the ticks, what will go, and the Clear button.
  // Folded the rest of the time, since the unit's readings need the room more than five ticks do.
  $('cl-toggle').addEventListener('click', (e) => {
    const box = $('clear-opts');
    box.hidden = !box.hidden;
    e.target.setAttribute('aria-expanded', String(!box.hidden));
  });

  $('btn-clear').addEventListener('click', async () => {
    const w = clearWanted();
    const f = fleet();
    const done = [];
    if (w.sel) {
      const label = view.sel;
      if (label && f.clearUnit(label)) { done.push('CSID ' + label); view.sel = null; }
    } else if (w.units) {
      f.clearUnits(); view.sel = null; done.push('every unit');
    }
    if (w.rx) { f.clearReceivers(); done.push('receiver counts'); }
    if (w.hidden) { f.hidden.clear(); done.push('hidden units'); }
    if (w.log) {
      try {
        const j = await hub.rotateLog();
        done.push('new log ' + (j.log || '').split(/[\\/]/).pop());
      } catch (err) {
        toast('Could not start a new log: ' + (err.message || err));
      }
    }
    toast(done.length ? 'Cleared ' + done.join(', ') : 'Nothing to clear');
    render(true);
  });

  // A card is the click target; its runs pill opens the Runs tab for that board.
  function pickCard(e) {
    const runs = e.target.closest('[data-runs]');
    const tr = e.target.closest('tr');
    if (!tr || !tr.dataset.label) return;
    view.sel = tr.dataset.label;
    if (runs) charts.showTab('runs');
    render(true);
  }
  $('fleet').querySelector('tbody').addEventListener('click', pickCard);
  $('fleet').querySelector('tbody').addEventListener('keydown', (e) => {
    if ((e.key === 'Enter' || e.key === ' ') && e.target.classList.contains('ucard')) { e.preventDefault(); pickCard(e); }
  });

  // ---------- settings ----------
  function buildSettingsForm() {
    const form = $('settings-form');
    form.innerHTML = '';
    let card = form;
    for (const f of S.FIELDS) {
      if (f.section) {
        card = document.createElement('section'); card.className = 'card';
        const h = document.createElement('h3'); h.textContent = f.section;
        card.appendChild(h); form.appendChild(card);
        if (f.note) { const n = document.createElement('div'); n.className = 'hint'; n.textContent = f.note; card.appendChild(n); }
        continue;
      }
      const row = document.createElement('div'); row.className = 'field';
      const id = 'set-' + f.key;
      let input;
      const value = S.shown(f, settings, model.values);
      if (f.type === 'select') {
        input = document.createElement('select');
        for (const [v, label] of f.options) input.add(new Option(label, v));
        // A value saved by an older version may no longer be offered: use the nearest option.
        if (!f.options.some(([v]) => v === settings[f.key])) {
          const near = f.options.reduce((a, b) => (Math.abs(b[0] - settings[f.key]) < Math.abs(a[0] - settings[f.key]) ? b : a))[0];
          settings = Object.assign({}, settings, { [f.key]: near });
          S.saveSettings(settings);
        }
        input.value = settings[f.key];
      } else {
        input = document.createElement('input');
        input.type = f.type;
        for (const a of ['min', 'max', 'step']) if (f[a] !== undefined) input[a] = f[a];
        if (f.type === 'checkbox') input.checked = value; else input.value = value;
      }
      input.id = id; input.dataset.key = f.key;
      const label = document.createElement('label'); label.htmlFor = id; label.textContent = f.label;
      // The checkbox stays the input that change listens to; On and Off are drawn round it
      let control = input;
      if (f.type === 'checkbox') {
        control = document.createElement('label'); control.className = 'toggle';
        control.innerHTML = '<span class="t-on">On</span><span class="t-off">Off</span>';
        control.prepend(input);
      }
      row.append(label, control);
      if (f.hint) { const h = document.createElement('div'); h.className = 'hint'; h.textContent = f.hint; row.appendChild(h); }
      card.appendChild(row);
    }
  }

  function applySettings(next) {
    settings = next;
    S.saveSettings(settings);
    live.setConfig(graphCfg());
    $('window').value = settings.graphWindowMin;
    render(true);
  }

  // The app's settings changed, from this screen or another: the fleets judge nothing, but
  // the cards and charts draw the limits, and an open drawer shows the new values
  function applyModel() {
    live.setConfig({ model: model.values });
    if (view.replay) view.replay.r.fleet.setConfig({ model: model.values });
    if (!$('settings').hidden && !$('settings').contains(document.activeElement)) buildSettingsForm();
    render(true);
  }

  $('settings-form').addEventListener('change', (e) => {
    const k = e.target.dataset.key;
    if (!k) return;
    const f = S.field(k);
    const ask = S.change(k, e.target.type === 'checkbox' ? e.target.checked : e.target.value);
    if (!ask) { e.target.value = S.shown(f, settings, model.values); return; }
    const shown = e.target.type === 'checkbox' ? (e.target.checked ? 'on' : 'off')
      : e.target.tagName === 'SELECT' ? e.target.options[e.target.selectedIndex].text : e.target.value;
    if (ask.display) {
      applySettings(Object.assign({}, settings, ask.display));
      toast(f.label + ': ' + shown);
      return;
    }
    model.set(ask.model).then(() => toast(f.label + ': ' + shown), (err) => {
      e.target.value = S.shown(f, settings, model.values);
      toast('The app kept ' + f.label.toLowerCase() + ' as it was: ' + (err.message || err));
    });
  });
  $('settings-reset').addEventListener('click', async () => {
    applySettings(S.defaults());
    try {
      await model.set(S.MODEL_DEFAULTS);
      toast('Settings reset to defaults');
    } catch (err) {
      toast('Display settings reset. The app kept its status settings: ' + (err.message || err));
    }
    buildSettingsForm();
  });
  // Focus goes into the drawer and back, or a keyboard user tabs through the whole page to reach it
  function closeSettings() {
    const inside = $('settings').contains(document.activeElement);
    $('settings').hidden = true;
    if (inside) $('btn-settings').focus();
  }
  $('btn-settings').addEventListener('click', () => {
    buildSettingsForm();
    $('settings').hidden = !$('settings').hidden;
    if (!$('settings').hidden) $('settings-close').focus();
  });
  $('settings-close').addEventListener('click', closeSettings);
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && !$('settings').hidden) closeSettings(); });

  for (const [v, label] of [0, 1, 5, 15, 30, 60].map(S.MIN)) $('window').add(new Option(label, v));
  $('window').value = settings.graphWindowMin;
  buildSettingsForm();   // also corrects values an older version saved
  $('window').addEventListener('change', (e) => { applySettings(Object.assign({}, settings, { graphWindowMin: +e.target.value })); buildSettingsForm(); });
  // Asked, never sent: another screen may have changed them
  model.load().catch(() => { /* the drawer shows the defaults until the app answers or the stream says */ });

  // ---------- theme ----------
  try { const th = localStorage.getItem('dg-theme'); if (th) document.documentElement.dataset.theme = th; } catch (e) { /* storage blocked */ }
  $('btn-theme').addEventListener('click', () => {
    const cur = document.documentElement.dataset.theme
      || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    const next = cur === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem('dg-theme', next); } catch (e) { /* storage blocked */ }
    charts.restyle();
  });

  // ---------- rendering ----------
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  function age(ms) {
    if (!Number.isFinite(ms)) return '—';
    const s = ms / 1000;
    if (s < 60) return s.toFixed(1) + 's';
    if (s < 3600) return Math.floor(s / 60) + 'm' + String(Math.floor(s % 60)).padStart(2, '0');
    return Math.floor(s / 3600) + 'h' + String(Math.floor((s % 3600) / 60)).padStart(2, '0');
  }

  function renderSummary(f, t) {
    const rp = view.mode === 'replay' ? view.replay : null;
    paint($('summary'), V.summaryHtml(f, t, rp ? rp.r.state.position : undefined));
    // With nothing heard there are no issues to list, but bad CRCs still say so
    const words = V.issuesText(f, t);
    paint($('issues'), esc(f.summary(t).heard || words !== 'No issues' ? words : ''));
  }

  function renderFleet(f, t) {
    const rows = f.rows(t, view.sort);
    if (view.sel === null || !f.unit(view.sel) || f.hidden.has(Number(view.sel))) view.sel = rows.length ? rows[0].unit.label : null;
    $('fleet-empty').hidden = rows.length > 0;
    paint($('fleet').querySelector('tbody'), V.fleetHtml(f, t, { sort: view.sort, sel: view.sel, rxName }));
    const n = f.hidden.size;
    $('btn-restore').hidden = !n;
    $('btn-restore').textContent = 'Show hidden (' + n + ')';
    const sel = $('sel-unit');
    const want = [...f.allUnits().values()]
      .sort((a, b) => (a.csid - b.csid) || (b.live - a.live) || (a.gen - b.gen))
      .map((u) => '<option value="' + u.label + '">CSID ' + u.label + (u.live ? '' : ' (earlier run)') + '</option>')
      .join('');
    if (sel.dataset.sig !== want) { sel.innerHTML = want; sel.dataset.sig = want; }
    if (view.sel !== null) sel.value = view.sel;
  }

  // Every run of a board, oldest first, the one transmitting now last
  function runsOf(f, csid) {
    const live = f.units.get(csid);
    return f.earlierRuns(csid).concat(live ? [live] : []);
  }

  // The Runs tab, drawn into the charts box in place of the plots
  function renderRuns(el, unit) {
    if (!unit) { paint(el, '<p class="empty">Select a unit in the list.</p>'); return; }
    paint(el, V.runsHtml(unit.csid, runsOf(fleet(), unit.csid), { sel: view.sel, ask: view.dropAsk }));
  }

  // ---------- alarm ----------
  // On a flight line nobody is watching the table when a unit drops out, so a unit going
  // from heard to lost makes a noise. Synthesised rather than a sound file, because the
  // field laptop has no internet and one less asset is one less thing to ship.
  const heard = new Map();   // csid -> the state it was in when last drawn
  let audio = null;

  // The browser will not start audio before the page has been clicked or typed on once: a
  // context made before that is suspended, and the click handler below resumes it, so the
  // first sound is the one after the first click. In a background tab the 250 ms redraw is
  // throttled to once a second (once a minute after five minutes hidden), so a sound can
  // come that late; it does play in the background once the context is running.
  // One tone: freq in Hz, len in seconds. A sine eases in and out; hard is a square wave
  // switched on and off, which is what reads as a beep.
  function tone(at, freq, len, hard) {
    const osc = audio.createOscillator();
    const gain = audio.createGain();
    osc.type = hard ? 'square' : 'sine';
    osc.frequency.setValueAtTime(freq, at);
    if (hard) {
      gain.gain.setValueAtTime(0.08, at);
      gain.gain.setValueAtTime(0.0001, at + len);
    } else {
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(0.2, at + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + len);
    }
    osc.connect(gain).connect(audio.destination);
    osc.start(at); osc.stop(at + len + (hard ? 0 : 0.02));
  }
  function sound() {
    audio = audio || new (window.AudioContext || window.webkitAudioContext)();
    if (audio.state === 'suspended' && audio.resume) audio.resume().catch(() => {});
    return audio;
  }
  // Lost: three low notes
  function beep(times) {
    if (!settings.alarmSound) return;
    try {
      sound();
      for (let i = 0; i < times; i++) tone(audio.currentTime + i * 0.22, 660, 0.16);
    } catch (e) { /* no audio device, or the browser wants a click first */ }
  }
  // A run starting: one plain beep, a 1 kHz square for 150 ms, nothing like the lost alarm
  // (2026-11-01: the user wanted a beep, not a rising chirp)
  function runBeep() {
    if (!settings.alarmSound) return;
    try { sound(); tone(audio.currentTime, 1000, 0.15, true); } catch (e) { /* as above */ }
  }
  for (const ev of ['click', 'keydown']) document.addEventListener(ev, () => { if (audio && audio.state === 'suspended' && audio.resume) audio.resume().catch(() => {}); });

  // A replay is a recording of something that already happened, so it never alarms.
  // The state is the app's; the page only notices it change.
  function watchForLost(t) {
    if (view.mode === 'replay') return;
    for (const u of live.units.values()) {
      const st = live.state(u);
      const was = heard.get(u.csid);
      if (was && was !== 'LOST' && st === 'LOST') {
        beep(3);
        toast('CSID ' + u.csid + ' lost after ' + age(t - u.lastT));
      }
      heard.set(u.csid, st);
    }
  }

  // A run starting makes one short sound, so nobody has to watch the table to know a board
  // came up (the user, 2026-11-01: "every time it hears a chipsat ... not on every
  // transmission"). A run is the app's: a csid heard for the first time, or a live row
  // whose first packet time moved, which is the app's own restart (a missed packet 0 still
  // counts). Nothing sounds for the rows a snapshot brings, on load or after a reconnect:
  // the watcher starts over silently whenever the fleet takes a snapshot. Packets, repeats
  // from a second receiver and replays change nothing here. Several boards up in one pass
  // or within two seconds are one sound, not a burst.
  const runsSeen = new Map();   // csid -> firstT of the live run last seen
  let runsAdopt = -1;
  let lastRunBeep = 0;
  function watchForRuns() {
    const silent = runsAdopt !== live.adopts || view.mode === 'replay';
    runsAdopt = live.adopts;
    let fresh = 0;
    for (const u of live.units.values()) {
      const was = runsSeen.get(u.csid);
      if (was === undefined || (u.firstT !== null && was !== null && u.firstT !== was)) fresh++;
      if (was === undefined || u.firstT !== null) runsSeen.set(u.csid, u.firstT);
    }
    for (const csid of runsSeen.keys()) if (!live.units.has(csid)) runsSeen.delete(csid);
    if (!fresh || silent) return;
    const t = Date.now();
    if (t - lastRunBeep < 2000) return;
    lastRunBeep = t;
    runBeep();
  }

  function renderLatest(f, t) {
    if (view.paused) return;
    const u = view.sel === null ? null : f.unit(view.sel);
    let o = null;
    if (u) {
      const all = runsOf(f, u.csid);
      o = { state: f.state(u, t), t, run: all.indexOf(u) + 1, runs: all.length, cfg: f.cfg, replay: view.mode === 'replay',
        rx: u.receptions.map((r) => ({ name: rxName(r.rx), rssi: r.rssi, snr: r.snr })) };
    }
    paint($('latest'), V.detailHtml(u, o));
  }
  $('latest').addEventListener('click', (e) => {
    if (!e.target.closest('[data-joinruns]')) return;
    charts.showTab('runs');
    render(true);
  });

  // SF6 through 12. Six only works because the packet is a fixed 55 bytes, which the
  // receiver is told at boot; the radio cannot read a header at that rate. A board
  // reporting something outside the range keeps its own number rather than showing the
  // nearest option, since this select is the board's truth.
  function sfSelect(k, sf) {
    const values = [6, 7, 8, 9, 10, 11, 12];
    if (!values.includes(Number(sf))) values.push(Number(sf));
    const opts = values.sort((a, b) => a - b)
      .map((n) => '<option value="' + n + '"' + (n === Number(sf) ? ' selected' : '') + '>SF' + n + '</option>').join('');
    return '<select data-sf="' + esc(k) + '" aria-label="Spreading factor on ' + esc(k) + '">' + opts + '</select>';
  }

  // The moment a boot's clock was pinned to GPS. A log exported before anchor_utc was
  // written carries only the epoch number.
  function anchorTime(src) {
    if (src.anchor_utc) return src.anchor_utc;
    const d = new Date(Number(src.anchor_unix_ms));
    return src.anchor_unix_ms && !Number.isNaN(d.getTime()) ? d.toISOString() : '';
  }

  // In a replay the panel lists the log's receivers as they were recorded, not the ports
  // plugged in now. Its head says so; the note sits where Rescan is in live.
  function rxHead(replaying) {
    const head = $('rxpanel').querySelector('.pane-head');
    let note = head.querySelector('.asrec');
    if (!note) {
      note = document.createElement('span'); note.className = 'note asrec'; note.textContent = 'as recorded';
      head.appendChild(note);
    }
    note.hidden = !replaying;
    head.querySelector('.kicker').textContent = replaying ? 'Receivers in this log' : 'Receivers · T-Beams';
  }

  function renderReceivers() {
    // The strip redraws every 250 ms, which would shut an open select under the operator.
    const focus = document.activeElement;
    if (focus && focus.tagName === 'SELECT' && $('receivers').contains(focus)) return;
    const f = fleet();
    const t = now();
    const replaying = view.mode === 'replay';
    rxHead(replaying);
    const keys = new Set([...f.receivers.keys(), ...(replaying ? [] : hub.ports.map((p) => p.key))]);
    const rows = [...keys].map((k) => {
      const r = f.receivers.get(k);
      const port = replaying ? null : hub.ports.find((p) => p.key === k);
      const m = meta(k);
      const st = V.receiverState(r, port, t);
      const source = f.rxSource.get(k);
      const kind = source === 'csv' ? 'CSV firmware' : source === 'raw' ? 'raw firmware' : '';
      const detail = [kind, r ? V.num(r.unique) + ' new, ' + V.num(r.packets - r.unique - r.badCrc) + ' duplicate' : '',
        r && r.badCrc ? V.num(r.badCrc) + ' bad CRC' : '', r && r.radioErrors ? V.num(r.radioErrors) + ' radio errors' : '',
        port && port.usb ? 'USB ' + port.usb : '', port && port.error ? port.error : '']
        .filter(Boolean).join('\n');
      const controls = replaying ? '' : (r && r.info.sf ? sfSelect(k, r.info.sf) : '') +
        (port && port.status === 'open' ? '<button data-close="' + esc(k) + '" title="Disconnect" aria-label="Disconnect ' + esc(rxName(k)) + '">✕</button>' : '');
      return V.receiverRowHtml({ key: k, name: rxName(k), id: r && r.id, sf: r && r.info.sf, packets: r ? r.unique : 0,
        word: st.word, cls: st.cls, index: m.index, detail, controls, extra: sfMark(k, r), sel: view.rxSel === k });
    });
    // Lines that did not come off a radio. Said plainly, because an absent RSSI is an
    // absence and not a fault, and a screenshot of a replay should not read as live.
    for (const [rx, src] of f.sources) {
      const what = src.kind === 'flash' ? 'Flash dump' : (src.kind || 'Not a radio');
      const bits = [src.from, src.boot ? 'boot ' + src.boot : '', src.packets ? V.num(src.packets) + ' records' : '']
        .filter(Boolean).join(', ');
      // Anything not anchored to GPS is a derived time, however it was derived, and
      // once it is epoch milliseconds it looks exactly like a real one. Tested the
      // other way round on purpose: a new kind of derived time must not slip through
      // by not being on a list.
      const guessed = !!src.time_source && src.time_source !== 'anchor';
      const anchor = anchorTime(src);
      const when = guessed ? '\nTimes are not from GPS (' + src.time_source + '), so the time axis is derived.'
        : (anchor ? '\nTime anchored at ' + anchor +
            (src.anchor_tacc_ns ? ' (receiver accuracy ' + src.anchor_tacc_ns + ' ns)' : '') : '');
      const key = 'src:' + rx;
      rows.push(V.receiverRowHtml({ key, name: rx, meta: bits, word: what, cls: 'off', sel: view.rxSel === key,
        detail: 'No RSSI or SNR: these lines came off a chip, not a radio.' + when,
        extra: guessed ? V.mark('warn', 'times not from GPS') : '' }));
    }
    // Two spreading factors is the normal setup for a test: the LE boards are on SF9, the HP
    // boards on SF12, and one receiver hears only one of them.
    const onSf = [...f.receivers.entries()].filter(([, r]) => r.info.sf);
    if (new Set(onSf.map(([, r]) => r.info.sf)).size > 1) {
      rows.push('<p class="rx-note note">' + esc(onSf.map(([k, r]) => rxName(k) + ' on SF' + r.info.sf).join(', ')) + '</p>');
    }
    paint($('receivers'), rows.join(''));
    if (api.onReceivers) api.onReceivers();
    renderClear();
  }
  $('receivers').addEventListener('click', (e) => {
    const k = e.target.dataset.close;
    const p = k && hub.ports.find((x) => x.key === k);
    if (p) { toast('Disconnected ' + rxName(k)); hub.close(p); return; }
    // A row opens for its SF and disconnect; touching the open select must not shut it
    if (e.target.closest('.rx-more')) return;
    const row = e.target.closest('.rx[data-rx]');
    if (!row) return;
    view.rxSel = view.rxSel === row.dataset.rx ? null : row.dataset.rx;
    renderReceivers();
  });
  $('receivers').addEventListener('change', (e) => {
    const k = e.target.dataset.sf;
    if (!k) return;
    const sf = Number(e.target.value);
    // Back to whatever the board last reported; the row says "asked for SFn" until the board's header says SFn
    e.target.blur();
    askSf(k, sf);
  });

  const charts = new DG.Charts($('charts'), $('tabs'), (k) => ({ label: rxName(k), index: meta(k).index }));
  charts.custom = renderRuns;

  // The Runs tab's buttons. Delete asks first, in a row under the run.
  $('charts').addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    const at = (n) => b.getAttribute(n);
    const f = fleet();
    if (b.id === 'runs-joinall') {
      const u = view.sel === null ? null : f.unit(view.sel);
      if (u) { const n = f.mergeAllRuns(u.csid); view.sel = String(u.csid); toast('Joined ' + n + ' earlier ' + (n === 1 ? 'run' : 'runs')); }
      view.dropAsk = null;
    } else if (at('data-merge')) {
      const merge = at('data-merge');
      const into = f.mergeRun(merge);
      if (into) { toast('Joined ' + merge + ' into ' + into); if (view.sel === merge) view.sel = into; }
      view.dropAsk = null;
    } else if (at('data-drop')) {
      view.dropAsk = at('data-drop');
    } else if (at('data-dropgo')) {
      const drop = at('data-dropgo');
      if (f.clearUnit(drop)) toast('Deleted run ' + drop);
      if (view.sel === drop) view.sel = null;
      view.dropAsk = null;
    } else if (at('data-dropno') !== null) {
      view.dropAsk = null;
    } else if (at('data-pick')) {
      view.sel = at('data-pick');
    } else {
      return;
    }
    render(true);
  });
  let lastChart = 0;

  function render(force) {
    const f = fleet();
    const t = now();
    // Another screen is up. The alarm and the bar carry on; the dashboard is not drawn,
    // and is drawn in full when it comes back.
    if ($('dash').hidden) {
      watchForLost(Date.now());
      watchForRuns();
      renderReceivers();
      return;
    }
    renderSummary(f, t);
    renderFleet(f, t);
    // The selected run's graph from before this page opened, asked once per run and connection
    client().wantHistory(view.sel);
    renderLatest(f, t);
    watchForLost(Date.now());
    watchForRuns();
    renderReceivers();
    if (view.mode === 'replay') renderReplayProgress();
    if (force || Date.now() - lastChart > 500) {
      lastChart = Date.now();
      const u = view.sel === null ? null : f.unit(view.sel);
      $('graph-title').textContent = u ? 'CSID ' + u.csid + ' history' : 'History';
      charts.update(u, { windowMin: settings.graphWindowMin, lineWidth: settings.lineWidth, saturationMps2: f.cfg.saturationMps2 });
    }
  }

  // Live needs redrawing because time passes: ages grow, units go stale. A replay redraws
  // when the app says it moved; a paused one is a still picture, and redrawing it four
  // times a second over thousands of points is what made the page impossible to click.
  setInterval(() => {
    if (view.mode === 'replay') return;
    render();
  }, 250);
  render(true);

  // Late, because switching views draws the whole dashboard and that reaches helpers
  // defined further down this file.
  $('view-live').addEventListener('click', () => showView('live'));
  $('view-replay').addEventListener('click', () => showView('replay'));
  showView('live');

  window.DGApp = Object.assign(api, {
    live, view, hub, model, openReplay, closeReplay, settings: () => settings, taps, packetTaps,
    showView, openServerLog, rxName, askSf, sfAsks,
    // The plots were sized for a hidden box while another screen was up.
    redraw: () => { render(true); charts.resize(); },
    ui: { $, esc, toast, download, logUrl, flatName, served, age, sfSelect },
  });
})();
