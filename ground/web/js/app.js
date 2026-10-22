// Wires the page together: serial / log input -> fleet -> panels and charts.
(function () {
  'use strict';
  const { packet: P, lines: L, fleet: F, recorder: R, store: S, exportcsv: X, boardmodel: B } = DG;
  const $ = (id) => document.getElementById(id);
  // What the other screens reach this file through. Filled in at the bottom; the hooks
  // on it (onView, onReceivers, onTransport) are set by js/screens.js once it has loaded.
  const api = {};

  let settings = S.loadSettings();
  const live = new F.Fleet(settings);
  const recorder = new R.Recorder();
  const autosave = new S.Autosave();
  const rxMeta = new Map();   // rxKey -> {index, source}
  let rxCount = 0;

  const view = {
    sel: null, paused: false, sort: 'attention', expanded: new Set(), mode: 'live',
    sels: { live: null, replay: null },
    replay: null,   // {events, pos, clock, playing, speed, fleet, names}
  };
  const fleet = () => (view.mode === 'replay' && view.replay ? view.replay.fleet : live);
  // The replay's own clock only while the replay is what is on screen: with a log open
  // and Live showing, live units are aged against the real time.
  const now = () => (view.mode === 'replay' && view.replay ? view.replay.clock : Date.now());
  const stamp = () => new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-');

  function meta(rx) {
    let m = rxMeta.get(rx);
    if (!m) { m = { index: rxCount++ % 8, source: null }; rxMeta.set(rx, m); }
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
  function ingestLine(target, rx, text, t) {
    const ev = L.parseLine(text);
    const m = meta(rx);
    if (ev.kind === 'packet' || ev.kind === 'badlength') m.source = ev.source;
    if (ev.kind === 'rxinfo') m.source = 'raw';
    return target.ingest(ev, t, rx);
  }

  // Every line from every connected receiver is recorded, whichever unit is on screen.
  // The native app or Web Serial, depending on where the page was loaded from.
  // for screens that want every line, e.g. Bench
  const taps = [];
  const hub = DG.makeHub((rx, text) => {
    const t = Date.now();
    for (const tap of taps) tap(rx, text, t);
    const result = ingestLine(live, rx, text, t);
    recorder.add(t, rx, text, result === 'new' || result === 'duplicate' || result === 'badcrc');
    if (settings.autosave) autosave.add(R.formatLine(t, rx, text));
  }, () => renderReceivers(), (msg) => { toast(msg); serialBanner(); });

  // Called again if the transport changes under us, so it has to set both ways round.
  function serialBanner() {
    const native = hub.native;
    $('btn-connect').textContent = native ? 'Rescan' : 'Connect receiver';
    $('btn-connect').title = native
      ? 'Look for receivers again'
      : 'Pick a T-Beam serial port';
    if (api.onTransport) api.onTransport();
    if (hub.supported) return;
    $('unsupported').hidden = false; $('btn-connect').disabled = true;
  }
  serialBanner();

  $('btn-connect').addEventListener('click', async () => {
    try {
      const p = await hub.add();
      meta(p.key);
      toast(hub.native
        ? hub.ports.length + (hub.ports.length === 1 ? ' receiver' : ' receivers') + ' found'
        : 'Connected ' + p.key + (p.usb ? ' (USB ' + p.usb + ')' : ''));
    } catch (e) {
      if (e && e.name !== 'NotFoundError') toast('Could not open port: ' + (e.message || e));
      else if (hub.native) toast('No receiver found');
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

  // A log the app wrote, read back through its own server. more is what the caller knows
  // about it: {raw, dir}. A pull's log has the dump the board printed beside it.
  async function openServerLog(name, more) {
    try {
      const r = await fetch(logUrl(name));
      if (!r.ok) throw new Error('HTTP ' + r.status);
      const midnight = new Date(); midnight.setHours(0, 0, 0, 0);
      const events = R.mergeFiles([R.parseLogFile(await r.text(), name, midnight.getTime())]);
      if (!events.length) { toast('Nothing in ' + name); return; }
      const pulled = /^(.*)\/ground\.log$/.exec(name);
      startReplay(events, [name], Object.assign({ log: name, raw: pulled ? pulled[1] + '/dump.txt' : null, dir: null }, more));
    } catch (e) {
      toast('Could not open ' + name + ': ' + (e.message || e));
    }
  }

  // Which build this page is, for when someone asks whether the hosted copy is current.
  // Written by the deploy; absent when the page came off a disk or out of the app.
  fetch('version.json').then((r) => (r.ok ? r.json() : null)).then((v) => {
    if (v && v.commit) {
      document.querySelector('.brand').title = 'build ' + v.commit + ', deployed ' + v.built;
    }
  }).catch(() => { /* not deployed from the repo */ });

  // ---------- recording ----------
  $('rec-start').addEventListener('click', async () => {
    try {
      await recorder.start(settings.fileTag);
      toast(recorder.toFile ? 'Recording to ' + recorder.fileName : 'Recording. This browser saves the file when you press Stop.');
    } catch (e) {
      if (e && e.name !== 'AbortError') toast('Could not start recording: ' + (e.message || e));
    }
    renderRecorder();
  });
  $('rec-pause').addEventListener('click', () => {
    if (recorder.state === 'recording') recorder.pause(); else recorder.resume();
    renderRecorder();
  });
  $('rec-stop').addEventListener('click', async () => {
    const lines = recorder.rec.lines;
    const blob = await recorder.stop();
    if (blob) download(blob, recorder.fileName);
    toast('Saved ' + lines + ' lines to ' + recorder.fileName);
    renderRecorder();
  });

  function renderRecorder() {
    const st = recorder.state;
    const label = { idle: 'Not recording', recording: 'Recording', paused: 'Paused' }[st];
    $('rec-state').textContent = label;
    $('rec-state').className = 'rec-state ' + st;
    $('rec-start').disabled = st !== 'idle';
    $('rec-pause').disabled = st === 'idle';
    $('rec-pause').textContent = st === 'paused' ? 'Resume' : 'Pause';
    $('rec-stop').disabled = st === 'idle';
    const r = recorder.rec;
    const rxs = Object.keys(r.perRx);
    if (st === 'idle') {
      $('rec-info').innerHTML = recorder.fileName ? 'Last: ' + esc(recorder.fileName) : '';
      $('rec-info').title = '';
      return;
    }
    const secs = Math.floor(recorder.elapsedMs(Date.now()) / 1000);
    const hms = [Math.floor(secs / 3600), Math.floor(secs / 60) % 60, secs % 60].map((n) => String(n).padStart(2, '0')).join(':');
    $('rec-info').innerHTML = '<b>' + hms + '</b>  ' + r.packets + ' packets from ' + rxs.length + (rxs.length === 1 ? ' receiver' : ' receivers');
    $('rec-info').title = 'File: ' + recorder.fileName + '\n' + rxs.map((k) => rxName(k) + ': ' + r.perRx[k].packets + ' packets, ' + r.perRx[k].lines + ' lines').join('\n') +
      (recorder.error ? '\nWrite error: ' + recorder.error : '');
  }

  $('btn-save').addEventListener('click', () => {
    const name = settings.fileTag + '-session-' + stamp() + '.log';
    download(recorder.sessionBlob(), name);
    toast('Saved ' + recorder.lines + ' lines to ' + name);
  });

  window.addEventListener('beforeunload', (e) => {
    if (recorder.state !== 'idle' || live.total > 0) { e.preventDefault(); e.returnValue = ''; }
  });

  // ---------- crash autosave ----------
  function armAutosave() {
    clearInterval(armAutosave.timer);
    if (settings.autosave) armAutosave.timer = setInterval(() => autosave.flush().catch(() => {}), settings.autosaveS * 1000);
  }
  armAutosave();

  let recovering = null;
  autosave.previous().then((list) => {
    const withLines = list.filter((s) => s.lines > 0);
    if (!withLines.length) return;
    recovering = withLines[0];
    $('recover-text').textContent = 'Unsaved session from ' + new Date(recovering.started).toLocaleString() +
      ' (' + recovering.lines + ' lines) is still in this browser.' + (withLines.length > 1 ? ' ' + (withLines.length - 1) + ' older.' : '');
    $('recover').hidden = false;
  }).catch(() => {});

  const recoveredText = async () => R.HEADER + ' recovered ' + recovering.started + '\n' + await autosave.text(recovering.id);
  $('recover-open').addEventListener('click', async () => {
    const events = R.parseLogFile(await recoveredText(), 'recovered', 0);
    if (events.length) startReplay(events, ['recovered session']);
  });
  $('recover-download').addEventListener('click', async () => {
    const name = settings.fileTag + '-recovered-' + recovering.started.slice(0, 19).replace(/[:T]/g, '-') + '.log';
    download(new Blob([await recoveredText()], { type: 'text/plain' }), name);
    toast('Saved ' + name);
  });
  $('recover-discard').addEventListener('click', async () => {
    await autosave.remove(recovering.id);
    $('recover').hidden = true;
    toast('Discarded the stored session');
  });

  // ---------- replay ----------
  $('file-open').addEventListener('change', async (e) => {
    const files = [...e.target.files];
    e.target.value = '';
    if (!files.length) return;
    const midnight = new Date(); midnight.setHours(0, 0, 0, 0);
    const parsed = [];
    for (const f of files) parsed.push(R.parseLogFile(await f.text(), f.name, midnight.getTime()));
    const events = R.mergeFiles(parsed);
    if (!events.length) { toast('No lines found in ' + files.map((f) => f.name).join(', ')); return; }
    startReplay(events, files.map((f) => f.name));
  });

  // ?log=<url> replays a file without anyone clicking Open log, so pulling a chip can
  // be one command that ends with the dashboard already showing it. Repeatable:
  // ?log=a&log=b merges them the way opening both files does.
  async function openFromUrl() {
    const urls = new URLSearchParams(location.search).getAll('log');
    if (!urls.length) return;
    const midnight = new Date(); midnight.setHours(0, 0, 0, 0);
    const parsed = [];
    const names = [];
    for (const u of urls) {
      try {
        const r = await fetch(u);
        if (!r.ok) throw new Error('HTTP ' + r.status);
        const name = (u.split('?')[0].split(/[\\/]/).pop()) || 'log';
        parsed.push(R.parseLogFile(await r.text(), name, midnight.getTime()));
        names.push(name);
      } catch (err) {
        // A bad link leaves a working dashboard and says why, rather than a blank page.
        toast('Could not open ' + u + ': ' + (err.message || err));
      }
    }
    const events = R.mergeFiles(parsed);
    if (!events.length) return;
    startReplay(events, names);
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
    $('view-replay').textContent = view.replay ? 'Replay: ' + view.replay.names.join(', ') : 'Replay';
    $('view-replay').title = view.replay ? view.replay.names.join(', ') : 'Open a log, or pull one off a ChipSat';
    $('replay').hidden = mode !== 'replay';
    renderReplayFile();
    if (api.onView) api.onView(mode);
    render(true);
  }

  // from is only given for a log the app serves: {log, raw, dir}, see servedLog.
  function startReplay(events, names, from) {
    stopPlay();
    // Scrubbing backwards used to throw the fleet away and feed every event in again.
    // Copies are kept along the way so a rewind restarts from the nearest one; the cost
    // of dragging then stops depending on how long the log is.
    view.replay = {
      events, pos: 0, clock: events[0].t, playing: false, speed: +$('rp-speed').value,
      // A log is finite and already on disk. Capping its history would quietly drop the
      // start of a long flight from every graph, which is the part worth looking at.
      fleet: new F.Fleet(Object.assign({}, settings, { maxPoints: Infinity, retainMin: 0 })), names,
      checks: [], every: Math.max(250, Math.ceil(events.length / 12)),
      from: from || null,
    };
    view.sel = null;
    $('rp-pos').max = events.length;
    showView('replay');
    // Feeding a long log in one go blocks the page for seconds with nothing on screen.
    // It goes in a slice at a time, the count keeps moving, and the browser stays awake.
    fillTo(events.length, () => {
      toast('Opened ' + names.join(', ') + ': ' + events.length + ' lines, ' +
        view.replay.fleet.total + ' packets');
    });
  }

  function seek(pos) {
    const r = view.replay;
    // The nearest copy at or before the target, whichever direction the handle moved: a
    // long jump forward is as expensive as a rewind if it replays from where we happen
    // to be standing.
    let from = null;
    for (const c of r.checks) if (c.pos <= pos && (!from || c.pos > from.pos)) from = c;
    if (pos < r.pos && !from) { r.fleet = new F.Fleet(settings); r.pos = 0; }
    if (from && from.pos > r.pos) { r.fleet.restore(from.state); r.pos = from.pos; }
    else if (pos < r.pos && from) { r.fleet.restore(from.state); r.pos = from.pos; }
    while (r.pos < pos) {
      const e = r.events[r.pos++];
      ingestLine(r.fleet, e.rx, e.text, e.t);
      if (r.pos % r.every === 0 && !r.checks.some((c) => c.pos === r.pos)) {
        r.checks.push({ pos: r.pos, state: r.fleet.snapshot() });
      }
    }
    r.clock = r.pos ? r.events[r.pos - 1].t : r.events[0].t;
    $('rp-pos').value = r.pos;
    $('rp-time').textContent = new Date(r.clock).toTimeString().slice(0, 8);
    render(true);
  }

  // Reading a log in slices, so a long one arrives visibly instead of freezing the page.
  // Twelve milliseconds a slice leaves the rest of the frame to draw.
  function fillTo(target, done) {
    const r = view.replay;
    if (!r) return;
    const total = r.events.length;
    const step = () => {
      if (view.replay !== r) return;   // the log was closed under us
      const until = performance.now() + 12;
      while (r.pos < target && performance.now() < until) {
        const e = r.events[r.pos++];
        ingestLine(r.fleet, e.rx, e.text, e.t);
        if (r.pos % r.every === 0 && !r.checks.some((c) => c.pos === r.pos)) {
          r.checks.push({ pos: r.pos, state: r.fleet.snapshot() });
        }
      }
      r.clock = r.pos ? r.events[r.pos - 1].t : r.events[0].t;
      $('rp-pos').value = r.pos;
      if (r.pos < target) {
        $('rp-time').textContent = 'reading ' + r.pos.toLocaleString() + ' of ' + total.toLocaleString();
        requestAnimationFrame(step);
        return;
      }
      $('rp-time').textContent = new Date(r.clock).toTimeString().slice(0, 8);
      render(true);
      if (done) done();
    };
    step();
  }

  // Dragging fires far faster than a rebuild can finish, so the work is coalesced to one
  // per frame and the time under the handle keeps up on its own.
  let seekWanted = null;
  function seekSoon(pos) {
    const r = view.replay;
    if (!r) return;
    $('rp-time').textContent = new Date(r.events[Math.max(0, Math.min(pos, r.events.length) - 1)].t)
      .toTimeString().slice(0, 8);
    if (seekWanted !== null) { seekWanted = pos; return; }
    seekWanted = pos;
    requestAnimationFrame(() => {
      const want = seekWanted;
      seekWanted = null;
      if (want !== null) seek(want);
    });
  }


  function stopPlay() {
    if (view.replay) view.replay.playing = false;
    clearInterval(view.playTimer);
    $('rp-play').textContent = 'Play';
  }

  $('rp-play').addEventListener('click', () => {
    const r = view.replay;
    if (r.playing) { stopPlay(); return; }
    if (r.pos >= r.events.length) seek(0);
    r.playing = true;
    $('rp-play').textContent = 'Pause';
    view.playTimer = setInterval(() => {
      r.clock += 100 * r.speed;
      let pos = r.pos;
      while (pos < r.events.length && r.events[pos].t <= r.clock) pos++;
      const keepClock = r.clock;
      if (pos !== r.pos) seek(pos);
      r.clock = keepClock;
      // seek() already redrew if the position moved, so there is nothing to throttle
      // here. What matters is not asking for the next tick until this one is done.
      if (r.pos >= r.events.length) stopPlay();
    }, 100);
  });
  $('rp-speed').addEventListener('change', (e) => { if (view.replay) view.replay.speed = +e.target.value; });
  // Each rebuild costs hundreds of milliseconds on a long log, and input fires far
  // faster than that, so dragging queued work the page could never finish. The time
  // under the handle follows the drag; the fleet catches up when it is let go.
  $('rp-pos').addEventListener('input', (e) => {
    stopPlay();
    const r = view.replay;
    if (!r) return;
    const at = Math.max(0, Math.min(+e.target.value, r.events.length) - 1);
    $('rp-time').textContent = new Date(r.events[at].t).toTimeString().slice(0, 8);
  });
  $('rp-pos').addEventListener('change', (e) => { stopPlay(); seek(+e.target.value); });
  $('rp-exit').addEventListener('click', () => {
    stopPlay(); view.replay = null; view.sels.replay = null; showView('live');
  });

  // ---------- saving what is open ----------
  // A finished pull lands in replay, where Save session and the rest of the live chrome
  // are hidden, so the replay bar carries its own ways out.

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
    const r = view.replay;
    const where = r && r.from ? onDisk(r.from) : '';
    // The receiver strip is live chrome and is hidden here, so this line is where a
    // replay says its records came off a chip and which of them have a real clock.
    const src = r ? R.sources(r.events) : { flash: 0, dated: 0 };
    const undated = src.flash - src.dated;
    const chip = !src.flash ? '' : 'Off a chip, not a radio: ' + src.flash + (src.flash === 1 ? ' boot, ' : ' boots, ') +
      src.dated + ' with GPS time' + (undated ? ', ' + undated + ' not dated' : '') + '.';
    const file = !r ? '' : where ? r.from.log.split('/').pop() + ' in ' + where : r.names.join(', ');
    $('rp-file').textContent = [file, chip].filter(Boolean).join('. ');
    $('rp-raw').hidden = !(r && r.from && r.from.raw);
  }

  // The open file's own name less its extension. Several files merged get a new one.
  function replayBase() {
    const r = view.replay;
    if (r.from) return flatName(r.from.log).replace(/\.[^.]*$/, '');
    if (r.names.length === 1) return flatName(r.names[0]).replace(/\.[^.]*$/, '');
    return settings.fileTag + '-merged-' + stamp();
  }

  async function served(name) {
    const r = await fetch(logUrl(name));
    if (!r.ok) throw new Error('HTTP ' + r.status);
    return r.blob();
  }

  $('rp-save').addEventListener('click', async () => {
    const r = view.replay;
    if (!r) return;
    const name = replayBase() + '.log';
    if (r.from) {
      try {
        download(await served(r.from.log), name);
        toast('Saved ' + name);
        return;
      } catch (e) {
        // The app may have been closed since the log was opened. Its lines are still
        // here, so they are written out below; only the file's comment lines are lost.
      }
    }
    const text = R.HEADER + ' saved ' + new Date().toISOString() + '\n' +
      r.events.map((e) => R.formatLine(e.t, e.rx, e.text)).join('');
    download(new Blob([text], { type: 'text/plain' }), name);
    toast((r.from ? 'The app did not answer. ' : '') + 'Saved ' + r.events.length.toLocaleString() + ' lines to ' + name);
  });

  $('rp-csv').addEventListener('click', () => {
    const r = view.replay;
    if (!r) return;
    const csv = X.toCsv(r.events);
    const rows = csv.split('\n').length - 2;   // less the header and the last newline
    if (rows < 1) { toast('Nothing to save: no packet in this log passes its CRC.'); return; }
    const name = replayBase() + '.csv';
    download(new Blob([csv], { type: 'text/csv' }), name);
    toast('Saved ' + rows.toLocaleString() + ' rows to ' + name);
  });

  $('rp-raw').addEventListener('click', async () => {
    const r = view.replay;
    if (!r || !r.from || !r.from.raw) return;
    const name = flatName(r.from.raw);
    try {
      download(await served(r.from.raw), name);
      toast('Saved ' + name);
    } catch (e) {
      toast('Could not get ' + r.from.raw + ': ' + (e.message || e));
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
  // Everything is on the page: tick what should go, press Clear. Nothing here touches
  // lines already written to disk unless a new log file is asked for.
  const clearBoxes = { units: 'cl-units', sel: 'cl-sel', rx: 'cl-rx', hidden: 'cl-hidden', log: 'cl-log' };
  const clearWanted = () => Object.fromEntries(Object.entries(clearBoxes).map(([k, id]) => [k, $(id).checked]));

  function renderClear() {
    const w = clearWanted();
    const bits = [];
    if (w.sel && view.sel) bits.push('CSID ' + view.sel);
    else if (w.units) bits.push('all units');
    if (w.rx) bits.push('receiver counts');
    if (w.hidden) bits.push('hidden');
    if (w.log) bits.push('new log file');
    $('clear-what').textContent = bits.length ? bits.join(', ') : 'nothing ticked';
    $('btn-clear').disabled = !bits.length;
  }
  for (const id of Object.values(clearBoxes)) $(id).addEventListener('change', renderClear);
  // The ticks stay folded away: the line beside the button already says what will go,
  // and the unit's own readings need the room more than five tick boxes do.
  $('cl-toggle').addEventListener('click', (e) => {
    const box = $('clear-opts');
    box.hidden = !box.hidden;
    e.target.setAttribute('aria-expanded', String(!box.hidden));
  });

  $('btn-clear').addEventListener('click', async () => {
    const w = clearWanted();
    const f = fleet();
    const done = [];
    if (w.sel && view.sel) {
      const label = view.sel;
      if (f.clearUnit(label)) { done.push('CSID ' + label); view.sel = null; }
    } else if (w.units) {
      f.clearUnits(); view.sel = null; view.expanded.clear(); done.push('every unit');
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

  $('fleet').querySelector('tbody').addEventListener('click', (e) => {
    const merge = e.target.getAttribute('data-merge');
    if (merge) {
      const into = fleet().mergeRun(merge);
      if (into) {
        toast('Merged ' + merge + ' into ' + into);
        if (view.sel === merge) view.sel = into;
      }
      render(true);
      return;
    }
    const mergeAll = e.target.getAttribute('data-mergeall');
    if (mergeAll) {
      const n = fleet().mergeAllRuns(Number(mergeAll));
      toast(n ? 'Joined ' + n + (n === 1 ? ' earlier run' : ' earlier runs') + ' into CSID ' + mergeAll : 'Nothing to join');
      view.sel = String(mergeAll);
      render(true);
      return;
    }
    const drop = e.target.getAttribute('data-drop');
    if (drop) {
      if (fleet().clearUnit(drop)) toast('Deleted run ' + drop);
      if (view.sel === drop) view.sel = null;
      render(true);
      return;
    }
    const more = e.target.getAttribute('data-gens');
    if (more) {
      // Folding away a board that opened itself has to be remembered, or it reopens.
      const auto = fleet().earlierRuns(Number(more)).length <= AUTO_OPEN_RUNS;
      const isOpen = view.expanded.has(more) || (auto && !view.expanded.has('shut' + more));
      view.expanded.delete(more); view.expanded.delete('shut' + more);
      if (isOpen) view.expanded.add('shut' + more); else view.expanded.add(more);
      render(true);
      return;
    }
    const tr = e.target.closest('tr');
    if (tr && tr.dataset.label) { view.sel = tr.dataset.label; render(true); }
  });

  // ---------- settings ----------
  const MIN = (n) => [n, n ? 'Last ' + n + ' min' : 'All data kept'];
  const FIELDS = [
    { section: 'Graphs' },
    { key: 'graphWindowMin', label: 'Time shown', type: 'select', options: [0, 1, 5, 15, 30, 60].map(MIN),
      hint: 'Counted back from the newest packet.' },
    { key: 'retainMin', label: 'Drop graph data older than', type: 'select',
      options: [[0, 'Never'], [5, '5 min'], [15, '15 min'], [30, '30 min'], [60, '1 hour'], [180, '3 hours']],
      hint: 'Graph history only. Packet counts, misses and resets are kept.' },
    { key: 'maxPoints', label: 'Max graph points per unit', type: 'number', min: 100, step: 100 },
    { key: 'lineWidth', label: 'Line width', type: 'select', options: [[1, 'Thin'], [1.25, 'Normal'], [2, 'Thick']] },
    { section: 'Status' },
    { key: 'lostMin', label: 'Lost after silence of (min)', type: 'number', min: 1, step: 1 },
    { key: 'staleFactor', label: 'Stale after missed intervals', type: 'number', min: 1, step: 0.5 },
    { key: 'staleMinS', label: 'Stale no sooner than (s)', type: 'number', min: 1, step: 1 },
    { key: 'lowBatteryV', label: 'Low battery below (V)', type: 'number', min: 2.5, max: 4.5, step: 0.05 },
    { key: 'weakRssiDbm', label: 'Weak RF below (dBm)', type: 'number', max: 0, step: 1 },
    { key: 'saturationMps2', label: 'Accel saturation at (m/s²)', type: 'number', min: 1, step: 1,
      hint: 'BNO085 range is ±8 g ≈ 78 m/s².' },
    { key: 'dedupeMs', label: 'Same packet across receivers within (ms)', type: 'number', min: 100, step: 100 },
    { key: 'alarmSound', label: 'Beep when a unit goes lost', type: 'checkbox' },
    { section: 'Saving' },
    { key: 'autosave', label: 'Autosave session in this browser', type: 'checkbox',
      hint: 'Offered back on the next visit after a crash.' },
    { key: 'autosaveS', label: 'Autosave every (s)', type: 'number', min: 1, step: 1 },
    { key: 'fileTag', label: 'File name prefix', type: 'text' },
  ];

  function buildSettingsForm() {
    const form = $('settings-form');
    form.innerHTML = '';
    for (const f of FIELDS) {
      if (f.section) { const h = document.createElement('h3'); h.textContent = f.section; form.appendChild(h); continue; }
      const row = document.createElement('div'); row.className = 'field';
      const id = 'set-' + f.key;
      let input;
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
        if (f.type === 'checkbox') input.checked = settings[f.key]; else input.value = settings[f.key];
      }
      input.id = id; input.dataset.key = f.key;
      const label = document.createElement('label'); label.htmlFor = id; label.textContent = f.label;
      row.append(label, input);
      if (f.hint) { const h = document.createElement('div'); h.className = 'hint'; h.textContent = f.hint; row.appendChild(h); }
      form.appendChild(row);
    }
  }

  function applySettings(next) {
    settings = next;
    S.saveSettings(settings);
    live.setConfig(settings);
    if (view.replay) view.replay.fleet.setConfig(settings);
    $('window').value = settings.graphWindowMin;
    armAutosave();
    render(true);
  }

  $('settings-form').addEventListener('change', (e) => {
    const k = e.target.dataset.key;
    if (!k) return;
    const def = S.defaults()[k];
    let v = e.target.type === 'checkbox' ? e.target.checked : e.target.value;
    if (typeof def === 'number') { v = Number(v); if (!Number.isFinite(v)) { e.target.value = settings[k]; return; } }
    applySettings(Object.assign({}, settings, { [k]: v }));
    const f = FIELDS.find((x) => x.key === k);
    const shown = e.target.type === 'checkbox' ? (v ? 'on' : 'off')
      : e.target.tagName === 'SELECT' ? e.target.options[e.target.selectedIndex].text : v;
    toast(f.label + ': ' + shown);
  });
  $('settings-reset').addEventListener('click', () => { applySettings(S.defaults()); buildSettingsForm(); toast('Settings reset to defaults'); });
  $('btn-settings').addEventListener('click', () => { buildSettingsForm(); $('settings').hidden = !$('settings').hidden; });
  $('settings-close').addEventListener('click', () => { $('settings').hidden = true; });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') $('settings').hidden = true; });

  for (const [v, label] of [0, 1, 5, 15, 30, 60].map(MIN)) $('window').add(new Option(label, v));
  $('window').value = settings.graphWindowMin;
  buildSettingsForm();   // also corrects values an older version saved
  $('window').addEventListener('change', (e) => { applySettings(Object.assign({}, settings, { graphWindowMin: +e.target.value })); buildSettingsForm(); });

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
  const fmt = (v, d) => (v === null || v === undefined || Number.isNaN(v) ? '—' : Number(v).toFixed(d));
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  function age(ms) {
    if (!Number.isFinite(ms)) return '—';
    const s = ms / 1000;
    if (s < 60) return s.toFixed(1) + 's';
    if (s < 3600) return Math.floor(s / 60) + 'm' + String(Math.floor(s % 60)).padStart(2, '0');
    return Math.floor(s / 3600) + 'h' + String(Math.floor((s % 3600) / 60)).padStart(2, '0');
  }

  function bits(mask) {
    return '<span class="bits">' + P.VALIDITY_BITS.map((b) => {
      const on = (mask >> b.bit) & 1;
      return '<span class="' + (on ? '' : 'off') + '" title="' + b.name + (on ? ' valid' : ' NOT valid') + '">' + b.short + '</span>';
    }).join('') + '</span>';
  }

  const STATE_LABEL = { OK: 'OK', STALE: 'Stale', LOST: 'Lost' };
  const stateHtml = (st) => '<span class="state ' + st + '">' + STATE_LABEL[st] + '</span>';

  function renderSummary(f, t) {
    const s = f.summary(t);
    // Always: how many units and packets. Problems: only when there are some.
    const base = [['heard', s.heard], ['active', s.active], ['packets', s.packets]]
      .map(([k, v]) => '<span><b>' + v + '</b> ' + k + '</span>');
    const issues = [
      ['stale', s.stale, ''], ['lost', s.lost, 'bad'], ['bad CRC', s.badCrc, 'bad'], ['low battery', s.lowBattery, ''],
      ['with invalid sensors', s.validityIssues, ''], ['weak RF', s.weakRf, ''], ['saturated', s.saturated, ''], ['resets', s.resets, ''],
    ].filter(([, v]) => v > 0).map(([k, v, c]) => '<span class="issue ' + c + '"><b>' + v + '</b> ' + k + '</span>');
    $('summary').innerHTML = base.concat(issues).join('');
  }

  function ageCell(f, u, a) {
    const limit = f.staleAfterMs(u);
    const frac = Math.min(1, a / limit);
    const cls = a > limit ? 'over' : frac > 0.66 ? 'near' : '';
    return '<span class="age" title="Stale after ' + fmt(limit / 1000, 1) + ' s without a packet">' + age(a) +
      '<i><b class="' + cls + '" style="width:' + (a > limit ? 100 : Math.max(2, frac * 100)).toFixed(0) + '%"></b></i></span>';
  }

  // The bar spans 3.0 V (empty) to 4.2 V (a full lithium cell); the number is the voltage
  function battCell(d) {
    if (!d.valid.soc) return '<span class="muted">—</span>';
    const v = d.values.battery;
    const pct = Math.max(0, Math.min(100, (v - 3.0) / 1.2 * 100));
    return '<span class="batt' + (v < settings.lowBatteryV ? ' low' : '') + '"><i><b style="width:' + pct.toFixed(0) + '%"></b></i>' + fmt(v, 2) + ' V</span>';
  }

  // An earlier run of a board, shown under the live one. Deliberately thin: it is
  // history, and the row that matters is the one still transmitting.
  const AUTO_OPEN_RUNS = 3;

  // The Runs tab. A board's counter starts again from 0 every time it restarts, so its
  // data is kept as separate runs. This is where they are put back together when the
  // restart was an accident, and thrown away when the run was rubbish.
  function renderRuns(el, unit) {
    if (!unit) { el.innerHTML = '<p class="empty">Select a unit in the fleet list.</p>'; return; }
    const f = fleet();
    const csid = unit.csid;
    const live = f.units.get(csid);
    const earlier = f.earlierRuns(csid);
    const all = earlier.concat(live ? [live] : []);
    const t = now();

    const head = '<p class="note">CSID ' + csid + ' has ' + (all.length === 1 ? 'one run' : all.length + ' runs') +
      '. A run ends when the board restarts and its counter goes back to 0. ' +
      (earlier.length ? 'If a restart was an accident, join the runs back together.' : 'Nothing to join.') + '</p>' +
      (earlier.length > 1 ? '<p><button id="runs-joinall" class="primary">Join all ' + all.length + ' runs into one</button></p>' : '');

    const rows = all.map((u) => {
      const span = u.firstT === null ? '—' : age(u.lastT - u.firstT);
      const when = u.firstT === null ? '' : new Date(u.firstT).toLocaleTimeString() + ' to ' + new Date(u.lastT).toLocaleTimeString();
      const joinable = !u.live;
      return '<div class="runrow' + (u.label === view.sel ? ' sel' : '') + '">' +
        '<div class="runhead"><b data-pick="' + u.label + '">' + u.label + '</b>' +
        (u.boot !== null && u.boot !== undefined ? '<span class="muted">boot ' + esc(u.boot) + '</span>' : '') +
        (u.live ? '<span class="st good">transmitting now</span>' : '<span class="muted">earlier run</span>') + '</div>' +
        '<div class="note">' + u.packets + ' packets, ' + u.missed + ' missed, ran for ' + span +
          (when ? ' — ' + esc(when) : '') + '</div>' +
        '<div class="setup-actions">' +
          (joinable ? '<button data-merge="' + u.label + '" class="primary">Join into the next run</button>' : '') +
          '<button data-drop="' + u.label + '">Delete this run</button>' +
        '</div></div>';
    }).join('');
    el.innerHTML = '<div class="runs">' + head + rows + '</div>';
  }

  function earlierRow(f, u, t, rxTotal) {
    const span = u.firstT === null ? '—' : age(u.lastT - u.firstT);
    return '<tr class="gen' + (u.label === view.sel ? ' sel' : '') + '" data-label="' + u.label + '">' +
      // Actions go in the first cell. The fleet table is fourteen columns wide and
      // scrolls sideways, so anything in the last cell is off the edge of the window.
      '<td class="csid">' + u.label +
        ' <button data-merge="' + u.label + '" class="act" title="Join this run to the one after it">join</button>' +
        ' <button data-drop="' + u.label + '" class="act" title="Delete this run">delete</button></td>' +
      '<td><span class="muted">earlier run</span></td>' +
      '<td class="r muted" title="Ended ' + esc(new Date(u.lastT).toLocaleString()) + '">' + age(t - u.lastT) + ' ago</td>' +
      '<td class="r muted">' + u.lastCounter + '</td>' +
      '<td class="r">' + u.packets + '</td>' +
      '<td class="r' + (u.missed ? '' : ' muted') + '">' + u.missed + '</td>' +
      '<td class="r muted">' + fmt(f.rxPercent(u), 1) + '</td>' +
      '<td class="r muted">—</td>' +
      '<td class="muted">ran for ' + span + '</td>' +
      '<td colspan="5" class="muted">ended ' + esc(new Date(u.lastT).toLocaleTimeString()) + '</td>' +
      '</tr>';
  }

  function renderFleet(f, t) {
    const rows = f.rows(t, view.sort);
    if (view.sel === null || !f.unit(view.sel) || f.hidden.has(Number(view.sel))) view.sel = rows.length ? rows[0].unit.label : null;
    $('fleet-empty').hidden = rows.length > 0;
    const rxTotal = [...f.receivers.values()].filter((r) => r.packets > 0).length;
    $('fleet').querySelector('tbody').innerHTML = rows.map(({ unit: u, state, age: a }) => {
      const d = u.latest;
      const rssiCls = u.bestRssi < settings.weakRssiDbm ? ' serious' : '';
      const earlier = f.earlierRuns(u.csid);
      // One or two earlier runs is the bench case: a knocked cable, and the operator
      // wants to join them. Show them without making anyone find a button first.
      // Thirty of them is a night of logs, which stays folded.
      const open = view.expanded.has(String(u.csid)) ||
        (earlier.length <= AUTO_OPEN_RUNS && !view.expanded.has('shut' + u.csid));
      return '<tr data-label="' + u.label + '"' + (u.label === view.sel ? ' class="sel"' : '') + '>' +
        '<td class="csid">' + u.label +
          (earlier.length ? ' <button class="act" data-gens="' + u.csid + '" title="Runs of this board that ended when it restarted">' +
            (open ? 'hide' : earlier.length + ' earlier') + '</button>' : '') +
          // While they are on screen each run has its own join, so this one would be a
          // second way to do the same thing. It is for the folded case.
          (earlier.length && !open ? ' <button class="act" data-mergeall="' + u.csid + '" title="Join every earlier run into this one">' +
            (earlier.length === 1 ? 'join it' : 'join all') + '</button>' : '') + '</td>' +
        '<td>' + stateHtml(state) + (d.saturated ? '<span class="flag" title="Acceleration near the accelerometer limit">saturated</span>' : '') + '</td>' +
        '<td class="r">' + ageCell(f, u, a) + '</td>' +
        '<td class="r">' + u.lastCounter + '</td>' +
        '<td class="r">' + u.packets + '</td>' +
        // Off a chip there is no link and no transmission counter to reason from, so
        // these say nothing rather than saying something wrong.
        '<td class="r' + (u.fromChip || !u.missed ? ' muted' : '') + '">' + (u.fromChip ? '—' : u.missed) + '</td>' +
        '<td class="r' + (u.fromChip ? ' muted' : '') + '">' + (u.fromChip ? '—' : fmt(f.rxPercent(u), 1)) + '</td>' +
        '<td class="r' + (u.fromChip || !u.resets ? ' muted' : ' serious') + '">' + (u.fromChip ? '—' : u.resets) + '</td>' +
        '<td>' + battCell(d) + '</td>' +
        '<td>' + bits(d.values.validity) + '</td>' +
        '<td class="r' + rssiCls + '">' + fmt(u.bestRssi, 1) + '</td>' +
        '<td class="r">' + fmt(u.bestSnr, 1) + '</td>' +
        '<td class="r">' + (Number.isFinite(u.intervalMs) ? fmt(u.intervalMs / 1000, u.intervalMs < 10000 ? 2 : 0) + ' s' : '—') + '</td>' +
        '<td class="r" title="' + esc(u.receptions.map((r) => rxName(r.rx) + ': ' + fmt(r.rssi, 1) + ' dBm').join('\n')) + '">' + u.receptions.length + ' of ' + rxTotal + '</td>' +
        '</tr>' + (open ? earlier.map((g) => earlierRow(f, g, t, rxTotal)).join('') : '');
    }).join('');
    const sel = $('sel-unit');
    const want = [...f.allUnits().values()]
      .sort((a, b) => (a.csid - b.csid) || (b.live - a.live) || (a.gen - b.gen))
      .map((u) => '<option value="' + u.label + '">CSID ' + u.label + (u.live ? '' : ' (earlier run)') + '</option>')
      .join('');
    if (sel.dataset.sig !== want) { sel.innerHTML = want; sel.dataset.sig = want; }
    if (view.sel !== null) sel.value = view.sel;
  }

  // One list row. A field whose validity bit is clear shows the last trusted
  // value in muted ink, or — if there never was one.
  function fieldRow(u, label, key, dec, unit) {
    const fd = P.FIELDS.find((x) => x.key === key);
    const d = u.latest;
    let v;
    if (d.valid[fd.group]) {
      const sat = fd.group === 'accel' && Math.abs(d.values[key]) >= settings.saturationMps2;
      v = '<dd class="v' + (sat ? ' sat' : '') + '"' + (sat ? ' title="Near the accelerometer limit"' : '') + '>' + fmt(d.values[key], dec) + '</dd>';
    } else {
      const lv = u.lastValid[key];
      v = '<dd class="v stale" title="Not valid in this packet' + (lv !== undefined ? '. Last trusted value shown.' : '') + '">' + (lv !== undefined ? fmt(lv, dec) : '—') + '</dd>';
    }
    return '<dt>' + label + '</dt>' + v + '<dd class="u">' + unit + '</dd>';
  }
  const plainRow = (label, value, unit, cls) => '<dt>' + label + '</dt><dd class="v ' + (cls || '') + '">' + value + '</dd><dd class="u">' + (unit || '') + '</dd>';

  function list(title, rows, group, d) {
    const note = group && !d.valid[group] ? ' <span class="note bad">not valid</span>' : '';
    return '<section class="list"><h3>' + title + note + '</h3><dl>' + rows.join('') + '</dl></section>';
  }

  // ---------- alarm ----------
  // On a flight line nobody is watching the table when a unit drops out, so a unit going
  // from heard to lost makes a noise. Synthesised rather than a sound file, because the
  // field laptop has no internet and one less asset is one less thing to ship.
  const heard = new Map();   // csid -> the state it was in when last drawn
  let audio = null;

  function beep(times) {
    if (!settings.alarmSound) return;
    try {
      audio = audio || new (window.AudioContext || window.webkitAudioContext)();
      for (let i = 0; i < times; i++) {
        const at = audio.currentTime + i * 0.22;
        const osc = audio.createOscillator();
        const gain = audio.createGain();
        osc.frequency.value = 660;
        gain.gain.setValueAtTime(0.0001, at);
        gain.gain.exponentialRampToValueAtTime(0.2, at + 0.01);
        gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.16);
        osc.connect(gain).connect(audio.destination);
        osc.start(at); osc.stop(at + 0.18);
      }
    } catch (e) { /* no audio device, or the browser wants a click first */ }
  }

  // A replay is a recording of something that already happened, so it never alarms.
  function watchForLost(f, t) {
    if (view.mode === 'replay') return;
    for (const u of f.units.values()) {
      const st = f.state(u, t);
      const was = heard.get(u.csid);
      if (was && was !== 'LOST' && st === 'LOST') {
        beep(3);
        toast('CSID ' + u.csid + ' lost after ' + age(t - u.lastT));
      }
      heard.set(u.csid, st);
    }
  }

  function renderLatest(f, t) {
    if (view.paused) return;
    const u = view.sel === null ? null : f.unit(view.sel);
    const el = $('latest');
    if (!u) { el.innerHTML = '<div class="none">Select a unit in the fleet list.</div>'; return; }
    const d = u.latest;
    const st = f.state(u, t);
    const crcOk = d.crcOk
      ? '<span class="good" title="' + (u.latestSource === 'csv' ? 'Old CSV receiver: packet rebuilt from the printed values to check the CRC' : 'CRC matches') + '">pass</span>'
      : '<span class="bad">fail</span>';
    const rx = u.receptions.map((r) => plainRow(esc(rxName(r.rx)), fmt(r.rssi, 1) + ' / ' + fmt(r.snr, 1), 'dBm/dB'));
    el.innerHTML =
      '<div class="unit-head"><span class="id"><small>CSID</small>' + u.csid + '</span>' + stateHtml(st) +
        (d.saturated ? '<span class="flag">accel saturated</span>' : '') +
        '<span class="when"><b>' + new Date(u.lastT).toTimeString().slice(0, 8) + '</b>' + age(t - u.lastT) + ' ago</span></div>' +
      '<div class="validity-line">' + bits(d.values.validity) + '<span class="hex">0x' + d.values.validity.toString(16).toUpperCase().padStart(2, '0') + '</span></div>' +
      '<div class="lists">' +
        list('Link', [
          plainRow('Counter', d.values.counter),
          plainRow('Cell voltage', d.valid.soc ? fmt(d.values.battery, 2) : '—', 'V', d.valid.soc ? '' : 'stale'),
          plainRow('Interval', Number.isFinite(u.intervalMs) ? fmt(u.intervalMs / 1000, 2) : '—', 's'),
          plainRow('CRC', crcOk + ' <span class="hex muted">' + d.crcReceived.toString(16).toUpperCase().padStart(4, '0') + '</span>'),
          plainRow('Packets', u.packets), plainRow('Missed', u.missed), plainRow('Resets', u.resets),
          plainRow('Received', fmt(f.rxPercent(u), 1), '%'),
        ].concat(rx)) +
        list('GPS', [fieldRow(u, 'Latitude', 'lat', 6, '°'), fieldRow(u, 'Longitude', 'lon', 6, '°'), fieldRow(u, 'Altitude MSL', 'gpsAlt', 1, 'm')], 'gps', d) +
        list('Environment', [fieldRow(u, 'Temperature', 'temp', 2, '°C'), fieldRow(u, 'Pressure', 'pressure', 1, 'hPa'),
          fieldRow(u, 'Humidity', 'humidity', 1, '%'), fieldRow(u, 'Pressure altitude', 'envAlt', 1, 'm')], 'env', d) +
        list('Accelerometer', ['x', 'y', 'z'].map((a) => fieldRow(u, a.toUpperCase(), 'a' + a, 2, 'm/s²')), 'accel', d) +
        list('Gyroscope', ['x', 'y', 'z'].map((a) => fieldRow(u, a.toUpperCase(), 'g' + a, 1, 'deg/s')), 'gyro', d) +
        list('Magnetometer', ['x', 'y', 'z'].map((a) => fieldRow(u, a.toUpperCase(), 'm' + a, 1, 'µT')), 'mag', d) +
        list('Orientation', [['i', 'qi'], ['j', 'qj'], ['k', 'qk'], ['Real', 'qr']].map(([l, k]) => fieldRow(u, l, k, 4, '')), 'quat', d) +
      '</div>';
  }

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

  function renderReceivers() {
    // The strip redraws every 250 ms, which would shut an open select under the operator.
    const focus = document.activeElement;
    if (focus && focus.tagName === 'SELECT' && $('receivers').contains(focus)) return;
    const f = fleet();
    const t = now();
    const keys = new Set([...f.receivers.keys(), ...hub.ports.map((p) => p.key)]);
    $('receivers').innerHTML = [...keys].map((k) => {
      const r = f.receivers.get(k);
      const port = hub.ports.find((p) => p.key === k);
      const m = meta(k);
      let st = 'no data'; let cls = 'warning';
      if (port && port.status !== 'open') { st = port.status; cls = 'critical'; } else if (r && t - r.lastT < 7000) { st = 'live'; cls = 'good'; } else if (r) { st = 'quiet for ' + age(t - r.lastT); }
      const kind = m.source === 'csv' ? 'CSV firmware' : m.source === 'raw' ? 'raw firmware' : '';
      const detail = [kind, r && r.info.sf ? 'SF' + r.info.sf : '', r ? r.unique + ' new, ' + (r.packets - r.unique - r.badCrc) + ' duplicate' : '',
        r && r.badCrc ? r.badCrc + ' bad CRC' : '', r && r.radioErrors ? r.radioErrors + ' radio errors' : '', port && port.usb ? 'USB ' + port.usb : '', port && port.error ? port.error : '']
        .filter(Boolean).join('\n');
      return '<span class="rx" title="' + esc(detail) + '">' +
        '<span class="sw" style="background:var(--series-' + (m.index + 1) + ')"></span>' +
        '<span class="name">' + esc(rxName(k)) + '</span><span class="st ' + cls + '">' + st + '</span>' +
        (r ? '<span class="meta">' + r.unique + ' packets</span>' : '') +
        (hub.native && r && r.info.sf ? sfSelect(k, r.info.sf) : '') +
        (port && port.status === 'open' ? '<button data-close="' + k + '" title="Disconnect" aria-label="Disconnect ' + esc(k) + '">✕</button>' : '') + '</span>';
    }).join('');
    // Lines that did not come off a radio. Said plainly, because an absent RSSI is an
    // absence and not a fault, and a screenshot of a replay should not read as live.
    for (const [rx, src] of fleet().sources) {
      const what = src.kind === 'flash' ? 'Flash dump' : (src.kind || 'Not a radio');
      const bits = [src.from, src.boot ? 'boot ' + src.boot : '', src.packets ? src.packets + ' records' : '']
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
      $('receivers').insertAdjacentHTML('beforeend', '<span class="rx" title="' +
        esc('No RSSI or SNR: these lines came off a chip, not a radio.' + when) +
        '"><span class="name">' + esc(rx) + '</span><span class="st warning">' + esc(what) + '</span>' +
        (bits ? '<span class="meta">' + esc(bits) + '</span>' : '') +
        (guessed ? '<span class="st serious">times not from GPS</span>' : '') + '</span>');
    }
    if (api.onReceivers) api.onReceivers();
    renderClear();
    // Two spreading factors is the normal setup for a test: the LE boards are on SF9, the HP
    // boards on SF12, and one receiver hears only one of them.
    const onSf = [...f.receivers.entries()].filter(([, r]) => r.info.sf);
    if (new Set(onSf.map(([, r]) => r.info.sf)).size > 1) {
      $('receivers').insertAdjacentHTML('beforeend', '<span class="rx"><span class="meta">' +
        esc(onSf.map(([k, r]) => rxName(k) + ' on SF' + r.info.sf).join(', ')) + '</span></span>');
    }
    $('mode').textContent = view.replay ? '' : (recorder.lines ? recorder.lines + ' lines this session' : '');
  }
  $('receivers').addEventListener('click', (e) => {
    const k = e.target.dataset.close;
    const p = k && hub.ports.find((x) => x.key === k);
    if (p) { toast('Disconnected ' + rxName(k)); hub.close(p); }
  });
  $('receivers').addEventListener('change', (e) => {
    const k = e.target.dataset.sf;
    if (!k) return;
    const sf = Number(e.target.value);
    // Back to whatever the board last reported; the select settles when its next header arrives.
    e.target.blur();
    renderReceivers();
    hub.setSf(k, sf).then(() => toast('Asked ' + rxName(k) + ' for SF' + sf))
      .catch((err) => toast('Could not set SF: ' + (err.message || err)));
  });

  const charts = new DG.Charts($('charts'), $('tabs'), (k) => ({ label: rxName(k), index: meta(k).index }));
  charts.custom = renderRuns;

  // The Runs tab owns its own buttons; the fleet table's copies do the same work.
  $('charts').addEventListener('click', (e) => {
    const merge = e.target.getAttribute('data-merge');
    const drop = e.target.getAttribute('data-drop');
    const pick = e.target.getAttribute('data-pick');
    const f = fleet();
    if (e.target.id === 'runs-joinall') {
      const u = view.sel === null ? null : f.unit(view.sel);
      if (u) { const n = f.mergeAllRuns(u.csid); view.sel = String(u.csid); toast('Joined ' + n + ' earlier ' + (n === 1 ? 'run' : 'runs')); }
    } else if (merge) {
      const into = f.mergeRun(merge);
      if (into) { toast('Joined ' + merge + ' into ' + into); if (view.sel === merge) view.sel = into; }
    } else if (drop) {
      if (f.clearUnit(drop)) toast('Deleted run ' + drop);
      if (view.sel === drop) view.sel = null;
    } else if (pick) {
      view.sel = pick;
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
      watchForLost(f, t);
      renderReceivers();
      renderRecorder();
      return;
    }
    renderSummary(f, t);
    renderFleet(f, t);
    renderLatest(f, t);
    watchForLost(f, t);
    renderReceivers();
    renderRecorder();
    if (force || Date.now() - lastChart > 500) {
      lastChart = Date.now();
      const u = view.sel === null ? null : f.unit(view.sel);
      $('graph-title').textContent = u ? 'CSID ' + u.csid + ' history' : 'History';
      charts.update(u, { windowMin: settings.graphWindowMin, lineWidth: settings.lineWidth, saturationMps2: settings.saturationMps2 });
    }
  }

  // Live needs redrawing because time passes: ages grow, units go stale. A paused replay
  // is a still picture of a moment that has already happened, and redrawing it four times
  // a second over thousands of points is what made the page impossible to click.
  setInterval(() => {
    if (view.mode === 'replay') return;
    render();
  }, 250);
  render(true);

  // Offline copy for the hosted site. file:// pages don't need it.
  if ('serviceWorker' in navigator && location.protocol === 'https:') {
    navigator.serviceWorker.register('sw.js').catch(() => {});
  }

  // Handle for manual checks in the console.
  // Late, because switching views draws the whole dashboard and that reaches helpers
  // defined further down this file.
  $('view-live').addEventListener('click', () => showView('live'));
  $('view-replay').addEventListener('click', () => showView('replay'));
  showView('live');

  window.DGApp = Object.assign(api, {
    live, view, hub, recorder, autosave, startReplay, seek, settings: () => settings, taps,
    showView, openServerLog, rxName,
    // The plots were sized for a hidden box while another screen was up.
    redraw: () => { render(true); charts.resize(); },
    ui: { $, esc, toast, download, logUrl, flatName, served, age, sfSelect },
  });
})();
