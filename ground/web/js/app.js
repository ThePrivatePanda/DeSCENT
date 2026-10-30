// Wires the page together: serial / log input -> fleet -> panels and charts.
(function () {
  'use strict';
  const { packet: P, lines: L, fleet: F, recorder: R, store: S, exportcsv: X, boardmodel: B } = DG;
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

  let settings = S.loadSettings();
  const live = new F.Fleet(settings);
  const recorder = new R.Recorder();
  const autosave = new S.Autosave();
  const rxMeta = new Map();   // rxKey -> {index, source}
  let rxCount = 0;

  const view = {
    sel: null, paused: false, sort: 'attention', mode: 'live',
    rxSel: null,     // the receiver row opened for its SF and disconnect
    dropAsk: null,   // the run whose delete waits for its second click
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
  // for panes that want every line, e.g. the sensor bench
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
  // Written by the deploy; absent when the page came off a disk or out of the app. Not
  // asked for in the app, where it is a 404 in the console on every start.
  if (!hub.native) fetch('version.json').then((r) => (r.ok ? r.json() : null)).then((v) => {
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
    // The name hides on a narrow bar; the replay bar under it says it in full
    $('view-replay').innerHTML = view.replay ? 'Replay<span class="pill-file"> · ' + esc(view.replay.names.join(', ')) + '</span>' : 'Replay';
    $('view-replay').title = view.replay ? view.replay.names.join(', ') : 'Open a log, or pull one off a ChipSat';
    $('replay').hidden = mode !== 'replay';
    renderReplayFile();
    if (api.onView) api.onView(mode);
    render(true);
  }

  // A log is finite and already on disk. Capping its history would quietly drop the
  // start of a long flight from every graph, which is the part worth looking at. So a
  // replay keeps everything, whatever the live trimming is set to.
  const replayCfg = () => Object.assign({}, settings, { maxPoints: Infinity, retainMin: 0 });

  // from is only given for a log the app serves: {log, raw, dir}, see servedLog.
  function startReplay(events, names, from) {
    stopPlay();
    // Scrubbing backwards used to throw the fleet away and feed every event in again.
    // Copies are kept along the way so a rewind restarts from the nearest one; the cost
    // of dragging then stops depending on how long the log is.
    view.replay = {
      events, pos: 0, clock: events[0].t, playing: false, speed: +$('rp-speed').value,
      fleet: new F.Fleet(replayCfg()), names,
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
    if (pos < r.pos && !from) { r.fleet = new F.Fleet(replayCfg()); r.pos = 0; }
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
        $('rp-time').textContent = 'reading ' + V.num(r.pos) + ' of ' + V.num(total);
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

  // Where the replay is and what it has read, then which file and where it is kept. The
  // second half is worked out once a log: R.sources walks every line.
  function renderReplayFile() {
    const r = view.replay;
    $('rp-raw').hidden = !(r && r.from && r.from.raw);
    if (!r) { $('rp-file').textContent = ''; return; }
    const where = r.from ? onDisk(r.from) : '';
    // The receiver strip is live chrome and is hidden here, so this line is where a
    // replay says its records came off a chip and which of them have a real clock.
    const src = R.sources(r.events);
    const undated = src.flash - src.dated;
    const chip = !src.flash ? '' : 'Off a chip, not a radio: ' + src.flash + (src.flash === 1 ? ' boot, ' : ' boots, ') +
      src.dated + ' with GPS time' + (undated ? ', ' + undated + ' not dated' : '') + '.';
    const file = where ? r.from.log.split('/').pop() + ' in ' + where : r.names.join(', ');
    r.words = [file, chip].filter(Boolean).join('. ');
    renderReplayProgress();
  }
  function renderReplayProgress() {
    const r = view.replay;
    if (!r) return;
    const html = '<span class="rp-prog">line ' + V.num(r.pos) + ' of ' + V.num(r.events.length) + ' · ' +
      V.num(r.fleet.total) + ' packets · from ' + V.clock(r.events[0].t) + '</span>' +
      (r.words ? ' <span class="rp-where">· ' + esc(r.words) + '</span>' : '');
    paint($('rp-file'), html);
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
    let card = form;
    for (const f of FIELDS) {
      if (f.section) {
        card = document.createElement('section'); card.className = 'card';
        const h = document.createElement('h3'); h.textContent = f.section;
        card.appendChild(h); form.appendChild(card);
        continue;
      }
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
    live.setConfig(settings);
    if (view.replay) view.replay.fleet.setConfig(replayCfg());
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
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  function age(ms) {
    if (!Number.isFinite(ms)) return '—';
    const s = ms / 1000;
    if (s < 60) return s.toFixed(1) + 's';
    if (s < 3600) return Math.floor(s / 60) + 'm' + String(Math.floor(s % 60)).padStart(2, '0');
    return Math.floor(s / 3600) + 'h' + String(Math.floor((s % 3600) / 60)).padStart(2, '0');
  }

  function renderSummary(f, t) {
    const r = view.mode === 'replay' ? view.replay : null;
    paint($('summary'), V.summaryHtml(f, t, r ? r.pos : undefined));
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
    let o = null;
    if (u) {
      const all = runsOf(f, u.csid);
      o = { state: f.state(u, t), t, run: all.indexOf(u) + 1, runs: all.length, cfg: settings, replay: view.mode === 'replay',
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
      const kind = m.source === 'csv' ? 'CSV firmware' : m.source === 'raw' ? 'raw firmware' : '';
      const detail = [kind, r ? V.num(r.unique) + ' new, ' + V.num(r.packets - r.unique - r.badCrc) + ' duplicate' : '',
        r && r.badCrc ? V.num(r.badCrc) + ' bad CRC' : '', r && r.radioErrors ? V.num(r.radioErrors) + ' radio errors' : '',
        port && port.usb ? 'USB ' + port.usb : '', port && port.error ? port.error : '']
        .filter(Boolean).join('\n');
      const controls = replaying ? '' : (hub.native && r && r.info.sf ? sfSelect(k, r.info.sf) : '') +
        (port && port.status === 'open' ? '<button data-close="' + esc(k) + '" title="Disconnect" aria-label="Disconnect ' + esc(rxName(k)) + '">✕</button>' : '');
      return V.receiverRowHtml({ key: k, name: rxName(k), id: r && r.id, sf: r && r.info.sf, packets: r ? r.unique : 0,
        word: st.word, cls: st.cls, index: m.index, detail, controls, sel: view.rxSel === k });
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
    $('mode').textContent = view.replay ? '' : (recorder.lines ? recorder.lines + ' lines this session' : '');
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
    // Back to whatever the board last reported; the select settles when its next header arrives.
    e.target.blur();
    renderReceivers();
    hub.setSf(k, sf).then(() => toast('Asked ' + rxName(k) + ' for SF' + sf))
      .catch((err) => toast('Could not set SF: ' + (err.message || err)));
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
    if (view.mode === 'replay') renderReplayProgress();
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
