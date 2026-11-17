// The T-Beams screen: every USB serial board plugged in, what the app is doing about it,
// and the receiver firmware. A list on the left, the picked board's sheet on the right.
(function () {
  'use strict';
  const A = window.DGApp;
  const { $, esc, toast, age, sfSelect } = A.ui;
  const { paint, mark, track, nameWidth } = A.shell;
  const B = DG.boardmodel;
  const hub = A.hub;
  if (!hub.native) return;

  const STATE_WORDS = {
    receiver: ['good', 'reading it'],
    waiting: ['off', 'not touched'],
    dismissed: ['off', 'disconnected by you'],
    ignored: ['off', 'left alone'],
  };
  const LOOKS = {
    'descent-receiver': 'a DeSCENT receiver', 'csv-receiver': 'an old CSV receiver',
    'something-else': 'something, but not a receiver', silent: 'nothing at all',
  };

  const state = {
    sel: null,          // port of the board whose sheet is up
    cands: [],          // ports the app will flash, see flash.rs: only boards it was told to read
    firmware: null,     // name of the receiver firmware the app carries
    firmwareVersion: null,
    asked: '',          // boards the candidates were last asked for
    opts: new Map(),    // port -> {sf, backup}: how it is flashed
    heard: new Map(),   // port -> what Listen said
    looks: new Map(),   // port -> what Listen took it for, see LOOKS
    done: new Map(),    // port -> {bad, warn, text}: how the last flash of it ended
    flashAsks: new Map(), // port -> the spreading factor asked of a board after its flash, see flashHeard
    freqAsks: new Map(),  // receiver key -> the carrier frequency last asked of it, see askFreq
    lines: new Map(),   // receiver key -> its last three lines
    hb: new Map(),      // receiver key -> {up, t}: uptime its last heartbeat gave, and when
    busy: null, pct: 0, note: '',
    forget: null,       // port of the board being asked about before it is forgotten
    timer: null,
    sdrBusy: new Map(),   // program key -> what a press on its sheet is doing, until the app answers
    gainAsks: new Set(),  // program keys whose gain was picked here and not yet confirmed or refused
  };
  const optsFor = (port) => {
    if (!state.opts.has(port)) state.opts.set(port, { sf: 9, backup: false });
    return state.opts.get(port);
  };
  // A ChipSat's console (the ST-Link's own port, or one that printed the flight log) is the
  // ChipSats screen's, never a T-Beam
  const boards = () => (hub.boards || []).filter((b) => b.role !== 'console');
  // A board that should be read and could not be opened says why, not "reading it"
  const stateOf = (b) => (b.state === 'receiver' && b.error ? ['bad', 'not opened'] : STATE_WORDS[b.state] || ['off', b.state]);
  const grouped = new Intl.NumberFormat('en-US');
  const count = (n) => grouped.format(n);
  const clock = (t) => String(new Date(t).getHours()).padStart(2, '0') + ':' + String(new Date(t).getMinutes()).padStart(2, '0');
  const clockS = (t) => clock(t) + ':' + String(new Date(t).getSeconds()).padStart(2, '0');
  const upFor = (ms) => {
    const m = Math.floor(ms / 60000);
    if (m < 1) return Math.floor(ms / 1000) + ' s';
    return m < 60 ? m + ' m' : Math.floor(m / 60) + ' h ' + String(m % 60).padStart(2, '0') + ' m';
  };

  // Only the app's own taps see every line of every port; the last three of each are the
  // raw box, and a heartbeat's uptime is nowhere else (the receiver rows say when, not how long).
  A.taps.push((rx, text, t) => {
    const seen = state.lines.get(rx) || [];
    seen.push(String(text).replace(/\r$/, ''));
    if (seen.length > 3) seen.shift();
    state.lines.set(rx, seen);
    const beat = /^HB,(\d+),/.exec(text);
    if (beat) state.hb.set(rx, { up: Number(beat[1]), t });
    flashHeard(rx, text, t);
    freqHeard(rx, text, t);
  });

  // A flashed board boots on the spreading factor its firmware was built with. The app
  // asks it for the one chosen here when its first header comes in, and again until a
  // header shows it there (serial.rs, want_sf). Until then the card says asked, as the
  // Radio card does for its own asks: the board's header is the only proof (the user,
  // 2026-11-03, could not tell which factor a flashed board was on).
  const FLASH_SF_WAIT_MS = 30000;   // boot, the app's #GET and up to five asks
  const FLASH_BACK_MS = 90000;      // a board that never says anything after the flash

  // The words under the firmware card for the last flash of a port, from where its ask stands.
  function flashOutcome(port) {
    const ask = state.flashAsks.get(port);
    if (!ask) return;
    const saved = ask.backup ? ' The old firmware is saved as ' + ask.backup + '.' : '';
    let text; let bad = false; let warn = false;
    if (ask.state === 'asked') {
      warn = true;
      text = 'Flashed. Asked for SF' + ask.sf + (ask.back ? ', waiting for its header to say so.' : ', waiting for the board to come back.');
    } else if (ask.state === 'heard') {
      text = ask.chip ? 'Flashed. SF' + ask.sf + ' confirmed at ' + clockS(ask.heardAt) + '.'
        : 'Flashed. SF' + ask.sf + ' said by the board at ' + clockS(ask.heardAt) + '; its firmware does not read the radio chip back.';
    } else {
      bad = true;
      text = 'Flashed. ' + ask.said + '.';
    }
    state.done.set(port, { bad, warn, text: text + saved });
  }

  // A line from a port with a flash ask on it. The port comes back under a new receiver
  // key, so the ask is kept by port and handed to app.js under that key at the first
  // header, which puts the same words on the receiver row and the Radio card.
  function flashHeard(rx, text, t) {
    const port = hub.ports.find((p) => p.key === rx);
    const ask = port ? state.flashAsks.get(port.port) : null;
    if (!ask) return;
    const line = String(text);
    if (ask.state === 'asked' && line.startsWith('#DG,ERR,sf')) {
      ask.state = 'refused';
      ask.said = 'The board refused SF' + ask.sf;
      toast(ask.name + ' refused SF' + ask.sf);
    } else if (line.startsWith('#DG,RX,')) {
      if (!ask.back) {
        ask.back = t;
        ask.key = rx;
        if (A.sfAsks) A.sfAsks.set(rx, ask);
      }
      if (ask.state !== 'asked') { flashOutcome(port.port); return; }
      const said = B.headerSf(line);
      if (said.sf !== ask.sf || !said.listening) {
        ask.heardSf = said.sf; ask.deaf = !said.listening;
        flashOutcome(port.port);
        return;
      }
      ask.state = 'heard'; ask.heardAt = t; ask.chip = said.chip;
      ask.said = said.chip ? ask.name + ' is listening on SF' + said.sf : ask.name + ' says it is on SF' + said.sf;
      toast(ask.said);
    } else return;
    flashOutcome(port.port);
    if (A.shell.current() === 'tbeams') render();
  }

  // An ask with no answer in time is not confirmed, and the card says what the board did say.
  function flashWaits(now) {
    for (const [port, ask] of state.flashAsks) {
      if (ask.state !== 'asked') continue;
      const limit = ask.back ? FLASH_SF_WAIT_MS : FLASH_BACK_MS;
      if (now - (ask.back || ask.at) <= limit) continue;
      ask.state = 'silent';
      ask.said = !ask.back ? 'SF' + ask.sf + ' not confirmed: no header from the board in ' + (limit / 1000) + ' s'
        : ask.deaf ? 'SF' + ask.sf + ' not confirmed: the board\'s radio is not in receive'
        : Number.isFinite(ask.heardSf) ? 'SF' + ask.sf + ' not confirmed: the board\'s header still says SF' + ask.heardSf
          : 'SF' + ask.sf + ' not confirmed: no header said so in ' + (limit / 1000) + ' s';
      toast(ask.name + ': ' + ask.said);
      flashOutcome(port);
      if (A.shell.current() === 'tbeams') render();
    }
  }
  setInterval(() => { flashWaits(Date.now()); freqWaits(Date.now()); }, 1000);

  // Which physical board is /dev/ttyUSB0? Nobody can answer that from a port name, so
  // the app watches for one to be unplugged and plugged back in, and you name that one.
  const ident = { watching: false, found: null, left: 0, timer: null };

  async function pollIdentify() {
    let st;
    try { st = await hub.identifyState(); } catch (e) { return; }
    ident.watching = !!st.watching;
    ident.left = st.seconds_left || 0;
    if (st.found) {
      ident.found = st.found;
      ident.watching = false;
      clearInterval(ident.timer); ident.timer = null;
      state.sel = st.found;
      toast(st.found + ' is the board you replugged');
    } else if (!st.watching) {
      clearInterval(ident.timer); ident.timer = null;
    }
    render();
  }

  function whichHtml() {
    const say = ident.found
      ? '<b>' + esc(ident.found) + '</b> is the board you replugged. Name it on its sheet.'
      : ident.watching
        ? '<b>Unplug the board, then plug it back in.</b> Waiting ' + ident.left + ' s.'
        : 'Unplug one board and plug it back in. The app watches for it and opens its sheet so you can name it.';
    return '<h3>Which board is which?</h3><p>' + say + '</p>';
  }

  // The receiver a board is being read as, if it is: {key, port, r} with r the live
  // fleet's count for it. The live fleet whichever view is up, since a replayed log's
  // receivers are not the boards plugged in now.
  function reading(b) {
    const port = hub.ports.find((p) => p.port === b.port);
    return port ? { key: port.key, port, r: A.live.receivers.get(port.key) } : null;
  }

  // A receiver goes by its own name or its id. A board that has not said it is one goes
  // by the name given to its USB adapter, or its port.
  function nameOf(b) {
    const rx = reading(b);
    return rx ? B.receiverName(b.port, rx.port, rx.r, b) : (b.name || b.port);
  }

  const usbOf = (b) => 'USB ' + String(b.usb).toUpperCase();

  function listHtml() {
    const all = boards();
    const head = '<div class="tb-listhead"><span class="kicker">Plugged in · ' + all.length + (all.length === 1 ? ' board' : ' boards') +
      '</span><span class="note">USB serial</span></div>';
    if (!all.length) return head + '<p class="empty">Nothing plugged in.</p>';
    return head + all.map((b) => {
      const [cls, word] = stateOf(b);
      const rx = reading(b);
      const sf = rx && rx.r && rx.r.info.sf ? '<span class="tb-sf">SF' + esc(rx.r.info.sf) + '</span>' : '';
      const name = nameOf(b);
      const sub = [state.looks.has(b.port) ? LOOKS[state.looks.get(b.port)] || state.looks.get(b.port) : '',
        name === b.port ? '' : b.port, usbOf(b), b.error || ''].filter(Boolean).join(' · ');
      const on = b.port === state.sel;
      return '<button class="card' + (on ? ' sel ring' : '') + ' tb-card" data-pick="' + esc(b.port) + '">' +
        '<span class="tb-top"><b class="tb-name">' + esc(name) + '</b>' +
        (state.busy === b.port ? mark('warn', 'being flashed') : mark(cls, word)) +
        (ident.found === b.port ? mark('good', 'this one') : '') + sf + '</span>' +
        '<span class="tb-sub">' + esc(sub) + '</span></button>';
    }).join('');
  }

  // Programs the app starts (command.rs, docs/sdr-receiver.md). An SDR gets a card and a sheet of
  // its own; app.js keeps what the app says of each (A.programs).
  const progs = () => (A.programs ? A.programs.list : []);
  const portOf = (key) => hub.ports.find((p) => p.key === key);
  const sdrs = () => progs().filter((p) => B.isSdr(p, portOf(p.key)));
  function sdrName(p) {
    const name = B.receiverName(p.key, portOf(p.key), A.live.receivers.get(p.key));
    return name === p.key ? 'SDR' : name;
  }
  const sdrState = (p) => (state.sdrBusy.has(p.key) ? { cls: 'warn', word: state.sdrBusy.get(p.key), why: '' } : B.programState(p, Date.now()));

  function sdrListHtml() {
    const all = sdrs();
    if (!all.length) {
      return A.programs && !A.programs.missing
        ? '<p class="note tb-nosdr">No SDR. To have one, add <b>sdr = &lt;command&gt;</b> to descent-ground.conf beside the program.</p>' : '';
    }
    return '<div class="tb-listhead"><span class="kicker">SDR · ' + all.length + '</span><span class="note">started by the app</span></div>' +
      all.map((p) => {
        const st = sdrState(p);
        const sfs = B.headerSfs(p);
        const r = A.live.receivers.get(p.key);
        const sub = [r ? count(r.unique) + (r.unique === 1 ? ' packet' : ' packets') : 'no packets yet', st.again || ''].filter(Boolean).join(' · ');
        return '<button class="card' + (p.key === state.sel ? ' sel ring' : '') + ' tb-card" data-pick="' + esc(p.key) + '">' +
          '<span class="tb-top"><b class="tb-name">' + esc(sdrName(p)) + '</b>' + mark(st.cls, st.word) +
          (sfs.length ? '<span class="tb-sf">' + sfs.map((n) => 'SF' + n).join(' · ') + '</span>' : '') + '</span>' +
          '<span class="tb-sub">' + esc(sub) + '</span></button>';
      }).join('');
  }

  // Any other program: a line under Programs, with no sheet
  function programsHtml() {
    const sdrKeys = new Set(sdrs().map((p) => p.key));
    const progs = hub.ports.filter((p) => p.key.startsWith('cmd:') && !sdrKeys.has(p.key) && p.rxKind !== 'sdr');
    if (!progs.length) return '';
    return '<div class="tb-listhead"><span class="kicker">Programs · ' + progs.length + '</span><span class="note">receiver = in descent-ground.conf</span></div>' +
      progs.map((p) => {
        const [cls, word] = p.status === 'open' ? ['good', 'reading it'] : ['bad', p.status];
        const sub = [p.rxId ? 'kind ' + (p.rxKind || 'not said') : 'no header yet', p.rxFw ? 'fw ' + p.rxFw : '', p.key, p.port].filter(Boolean).join(' · ');
        return '<div class="card tb-card tb-prog"><span class="tb-top"><b class="tb-name">' + esc(B.receiverName(p.key, p, A.live.receivers.get(p.key))) + '</b>' +
          mark(cls, word) + '</span><span class="tb-sub">' + esc(sub) + '</span>' +
          (p.status !== 'open' && p.error ? '<span class="tb-sub">' + esc(p.error) + '</span>' : '') + '</div>';
      }).join('');
  }

  const row = (label, value, cls) => '<div class="tb-row' + (cls ? ' ' + cls : '') + '"><span class="l">' + label + '</span><div class="tb-val">' + value + '</div></div>';
  const val = (text) => '<span class="val">' + text + '</span>';
  const unsaid = '<span class="note">not reported</span>';

  // The head: who it is, what the app does about it, and the buttons that change that.
  function headHtml(b) {
    const is = b.state === 'receiver';
    const rx = reading(b);
    const fw = B.receiverFirmware(rx ? rx.port : null, state.firmwareVersion);
    const [cls, word] = stateOf(b);
    // A receiver that has announced its id is named by that id, and the name goes with
    // the board to any port. Before that, a name can only be kept for the USB adapter.
    const how = rx ? B.namedBy(rx.port, rx.r) : { by: 'port', id: '' };
    const shown = how.by === 'board' ? nameOf(b) : how.by === 'computer' ? (rx.port.name || b.name || '') : (b.name || '');
    const blank = how.id ? 'T-Beam ' + how.id : b.port;
    const kept = how.by === 'board' ? 'The name is kept on the board'
      : how.by === 'computer' ? 'The name is kept on this computer'
        : 'The name is kept for this USB adapter';
    const line = [rx ? (how.id ? 'Receiver ' + how.id : 'receiver id not announced yet') : '', b.port, usbOf(b),
      rx ? 'DeSCENT receiver' + (fw.onBoard === '—' ? '' : ' ' + fw.onBoard) : '',
      b.serial ? 'serial ' + b.serial : '', rx ? '' : b.label].filter(Boolean).join(' · ');
    const decide = is
      ? '<button data-not="' + esc(b.port) + '">Stop reading it</button>'
      : '<button data-yes="' + esc(b.port) + '" class="primary">Read this one</button>' +
        (b.state === 'ignored' ? '' : '<button data-not="' + esc(b.port) + '">Leave it alone</button>');
    const said = state.heard.get(b.port);
    return '<div class="pane tb-head"><div class="tb-headrow">' +
        '<input type="text" class="nameit" data-keep="name:' + esc(b.port) + '" data-name="' + esc(b.port) + '"' +
          (how.id ? ' data-rxid="' + esc(how.id) + '"' : '') +
          ' value="' + esc(shown) + '" placeholder="' + esc(blank) + '" style="' + nameWidth(shown || blank) +
          '" maxlength="' + (how.id ? 24 : 40) + '" aria-label="Name of this board" title="Type a name and press Enter">' +
        mark(cls, word) +
        (ident.found === b.port ? mark('good', 'the one you replugged') : '') +
        '<span class="tb-line">' + esc(line) + '</span>' +
        '<span class="tb-acts"><button data-listen="' + esc(b.port) + '">Listen 5 s</button>' + decide +
          '<button class="ghost" data-forget="' + esc(b.port) + '"' + (state.busy === b.port ? ' disabled' : '') + '>Forget this board</button></span>' +
      '</div>' +
      (b.error ? '<p class="outcome bad">' + esc(b.port + ': ' + b.error) + '</p>' : '') +
      '<p class="tb-notes note">' + kept + '. ' +
        (is ? '' : 'Opening a port can reset the board on it. ') +
        (b.state === 'dismissed' ? 'Stays shut until it is replugged. ' : '') +
        'Listen opens the port for 5 s and says what it heard.</p>' +
      // Forgetting drops what the app keeps about the board and sends it nothing.
      (state.forget === b.port
        ? '<div class="confirm"><span class="grow">Forget <b>' + esc(nameOf(b)) + '</b>? Its name on this computer and whether to read it go, ' +
            'and its port is shut. Nothing is sent to the board' + (how.by === 'board' ? ', so the name kept on it stays' : '') +
            '. It goes back to not touched.</span>' +
          '<button data-forgetgo="' + esc(b.port) + '" class="primary">Forget it</button>' +
          '<button data-forgetcancel="1">Cancel</button></div>'
        : '') +
      (said ? '<pre class="raw">' + esc(said) + '</pre>' : '') +
      '</div>';
  }

  // Receivers that hear the same spreading factor hear the same packets twice.
  function sharedSf(rx, r) {
    if (!r || !r.info.sf) return '';
    const others = hub.ports.filter((p) => p.key !== rx.key).map((p) => ({ p, r: A.live.receivers.get(p.key) }))
      .filter((o) => o.r && o.r.info.sf && Number(o.r.info.sf) === Number(r.info.sf))
      .map((o) => B.receiverName(o.p.port, o.p, o.r, boards().find((x) => x.port === o.p.port)));
    if (!others.length) return '';
    const who = others.length > 1 ? others.slice(0, -1).join(', ') + ' and ' + others[others.length - 1] : others[0];
    return '<p class="tb-shared"><b>' + esc(who) + '</b> ' + (others.length > 1 ? 'are' : 'is') + ' also on SF' + esc(r.info.sf) + '.' +
      (others.length === 1 ? ' For an LE and HP test, put one receiver on SF9 and the other on SF12.' : '') + '</p>';
  }

  // Under the spreading factor select: what the board has said about the last ask, as
  // app.js tracks it (A.sfAsks). Only its header is proof; the select shows what it reports.
  function sfSaid(key, info) {
    const ask = A.sfAsks ? A.sfAsks.get(key) : null;
    const hears = ' It hears only boards on this spreading factor.';
    const hint = (words) => '<span class="hint">' + words + hears + '</span>';
    if (String(info.rx) === '0') return mark('bad', 'not listening') + hint("The board's radio chip says it is not in receive.");
    if (info.sfchip !== undefined && Number(info.sfchip) !== Number(info.sf)) {
      return mark('bad', 'radio on SF' + esc(info.sfchip)) + hint('The firmware says SF' + esc(info.sf) + ' but its radio chip is on SF' + esc(info.sfchip) + '.');
    }
    if (!ask) return hint("The board's own report. A change is asked of it and counts only when its header says so.");
    if (ask.state === 'asked') return mark('warn', 'asked for SF' + ask.sf) + hint('Asked for SF' + ask.sf + ' at ' + clockS(ask.at) + ". Waiting for the board's header.");
    if (ask.state === 'heard') {
      if (Number(info.sf) !== ask.sf) return hint("The board's own report. A change is asked of it and counts only when its header says so.");
      if (!ask.chip) return mark('warn', 'said, chip not read') + hint("The board's header said SF" + ask.sf + ' at ' + clockS(ask.heardAt || ask.at) + '; its firmware does not read the radio chip back.');
      return mark('good', 'confirmed') + hint("The board's radio chip confirmed SF" + ask.sf + ' at ' + clockS(ask.heardAt || ask.at) + '.');
    }
    const word = ask.state === 'refused' ? 'refused SF' + ask.sf : ask.state === 'failed' ? 'SF' + ask.sf + ' not sent' : 'SF' + ask.sf + ' not confirmed';
    return mark('bad', word) + hint(esc(ask.said) + '.' + (ask.state === 'refused' ? ' The board answered #DG,ERR,sf.' : ''));
  }

  // A carrier frequency is asked of a T-Beam the way a spreading factor is: #SET,freq through
  // the app, and only the header it prints at once (f=) or its #DG,ERR,freq counts. Never of a
  // ChipSat console or an SDR, which the app's config call would pass the line to all the same.
  const FREQ_WAIT_MS = 4000;
  const tunable = (port) => !!port && port.role === 'receiver' && port.status === 'open' && port.rxKind !== 'sdr';

  function freqStop(v) {
    const f = Number(String(v).trim());
    if (String(v).trim() === '' || !Number.isFinite(f)) return 'it is not a number of MHz';
    if (f < 902 || f > 928) return 'it has to be 902.0 to 928.0 MHz, the US band';
    if (Math.abs(f * 10 - Math.round(f * 10)) > 1e-6) return 'it goes in 0.1 MHz steps';
    return '';
  }

  function askFreq(key, typed) {
    const port = hub.ports.find((p) => p.key === key);
    if (!tunable(port)) { toast('Not sent: ' + key + ' is not a T-Beam being read'); return; }
    const name = B.receiverName(port.port, port, A.live.receivers.get(key), boards().find((x) => x.port === port.port));
    const stop = freqStop(typed);
    const mhz = stop ? String(typed).trim() : Number(typed).toFixed(1);
    const ask = { mhz, at: Date.now(), state: stop ? 'stopped' : 'asked', said: stop ? mhz + ' MHz not sent: ' + stop : '', heardF: '', heardAt: 0, name };
    state.freqAsks.set(key, ask);
    render();
    if (stop) return;
    hub.setFreq(key, mhz).catch((err) => {
      ask.state = 'failed';
      ask.said = 'Could not ask ' + name + ' for ' + mhz + ' MHz: ' + (err.message || err);
      toast(ask.said);
      render();
    });
  }

  function freqHeard(rx, text, t) {
    const ask = state.freqAsks.get(rx);
    if (!ask || ask.state !== 'asked') return;
    if (text.startsWith('#DG,ERR,freq')) {
      ask.state = 'refused';
      ask.said = ask.name + ' refused ' + ask.mhz + ' MHz';
    } else if (text.startsWith('#DG,RX,')) {
      const f = /,f=([\d.]+)/.exec(text);
      // a header printed before the command landed still says the old value; the wait judges it
      if (!f || Number(f[1]).toFixed(1) !== ask.mhz) { ask.heardF = f ? f[1] : ''; return; }
      ask.state = 'heard';
      ask.heardAt = t;
      ask.said = ask.name + ' is on ' + ask.mhz + ' MHz';
    } else return;
    toast(ask.said);
    if (A.shell.current() === 'tbeams') render();
  }

  function freqWaits(now) {
    for (const ask of state.freqAsks.values()) {
      if (ask.state !== 'asked' || now - ask.at <= FREQ_WAIT_MS) continue;
      ask.state = 'silent';
      ask.said = ask.heardF ? ask.name + ' answered with ' + ask.heardF + ' MHz, not ' + ask.mhz
        : 'No header from ' + ask.name + ' in ' + (FREQ_WAIT_MS / 1000) + ' s, so ' + ask.mhz + ' MHz is not confirmed';
      toast(ask.said);
      if (A.shell.current() === 'tbeams') render();
    }
  }

  // Under the frequency box, as sfSaid is under the spreading factor
  function freqSaid(key, info) {
    const ask = state.freqAsks.get(key);
    const hint = (words) => '<span class="hint">' + words + ' It hears only boards on this frequency.</span>';
    const own = hint("The board's own report. Type 902.0 to 928.0 MHz to change it; it counts only when its header says so.");
    if (!ask) return own;
    if (ask.state === 'asked') return mark('warn', 'asked for ' + ask.mhz + ' MHz') + hint('Asked at ' + clockS(ask.at) + ". Waiting for the board's header.");
    if (ask.state === 'heard') {
      if (Number(info.f).toFixed(1) !== ask.mhz) return own;
      return mark('good', 'confirmed') + hint("The board's header said " + ask.mhz + ' MHz at ' + clockS(ask.heardAt) + '.');
    }
    const word = ask.state === 'refused' ? 'refused ' + ask.mhz + ' MHz' : ask.state === 'silent' ? ask.mhz + ' MHz not confirmed' : ask.mhz + ' MHz not sent';
    return mark('bad', esc(word)) + hint(esc(ask.said) + '.' + (ask.state === 'refused' ? ' The board answered #DG,ERR,freq.' : ''));
  }

  function radioCard(b) {
    const rx = reading(b);
    if (!rx) return '';
    const r = rx.r;
    const info = r ? r.info : {};
    const pair = (a, c) => (a || c ? val(esc([a, c].filter(Boolean).join(' · '))) : unsaid);
    const open = rx.port.status === 'open';
    return '<div class="pane tb-card2"><h3>Radio</h3>' +
      (open ? '' : row('Port', mark('bad', rx.port.status) + (rx.port.error ? ' ' + esc(rx.port.error) : ''))) +
      // The select is the board's own report of what it is on, so it stays plain ink.
      row('Spreading factor', info.sf ? sfSelect(rx.key, info.sf) + sfSaid(rx.key, info)
        : '<span class="note">not reported yet</span>', 'tb-sfrow') +
      row('Frequency', !info.f ? unsaid : !tunable(rx.port) ? val(esc(info.f) + ' MHz')
        : '<span><input type="text" inputmode="decimal" class="tb-freq" data-freq="' + esc(rx.key) + '" data-keep="freq:' + esc(rx.key) +
          '" value="' + esc(info.f) + '" aria-label="Carrier frequency of ' + esc(rx.key) + ' in MHz"> MHz</span>' + freqSaid(rx.key, info), 'tb-sfrow') +
      row('Bandwidth · CR', pair(info.bw ? String(Number(info.bw)) + ' kHz' : '', info.cr ? '4/' + info.cr : '')) +
      row('Sync · preamble', pair(info.sync, info.pre)) +
      sharedSf(rx, r) + '</div>';
  }

  function sessionCard(b) {
    const rx = reading(b);
    if (!rx) return '';
    const r = rx.r;
    const now = Date.now();
    if (!r) {
      return '<div class="pane tb-card2"><h3>This session</h3><div class="tb-body">' + mark('warn', 'nothing heard') +
        '<p class="note">Wrong spreading factor, or not a receiver. Listen to check.</p>' +
        '<p class="note">In the log as <b>' + esc(rx.key) + '</b>.</p></div></div>';
    }
    const tile = (label, n) => '<div class="stat"><span>' + label + '</span><b>' + count(n) + '</b></div>';
    const beat = state.hb.get(rx.key);
    const beatT = beat ? beat.t : r.lastHbT;
    const said = 'Last line ' + age(now - r.lastT) + ' ago. ' + (beatT
      ? 'Last heartbeat ' + age(now - beatT) + ' ago' + (beat ? ', up ' + upFor(beat.up + (now - beat.t)) : '') + '.'
      : 'No heartbeat yet.');
    const raw = state.lines.get(rx.key);
    return '<div class="pane tb-card2"><h3>This session' + (r.firstT ? ' <span class="note">since ' + clock(r.firstT) + '</span>' : '') + '</h3>' +
      '<div class="tb-body"><div class="tb-tiles">' + tile('New packets', r.unique) + tile('Duplicates', r.packets - r.unique - r.badCrc) +
        tile('Bad CRC', r.badCrc) + tile('Radio errors', r.radioErrors) + '</div>' +
      '<p class="note">' + said + ' In the log as <b>' + esc(rx.key) + '</b>.</p>' +
      (raw && raw.length ? '<pre class="raw">' + esc(raw.join('\n')) + '</pre>' : '') + '</div></div>';
  }

  function firmwareCard(b) {
    const o = optsFor(b.port);
    const busy = state.busy === b.port;
    const done = state.done.get(b.port);
    const last = done ? '<p class="outcome tb-last' + (done.bad ? ' bad' : done.warn ? ' warn' : '') + '">' + esc(done.text) + '</p>' : '';
    const rx = reading(b);
    const fw = B.receiverFirmware(rx ? rx.port : null, state.firmwareVersion);
    const name = nameOf(b);
    let body;
    if (busy) {
      body = '<div class="job">' + track(state.pct) + '<p class="step">' + esc(state.note) + '</p></div>';
    } else if (!state.cands.includes(b.port)) {
      // flash.rs offers only boards the app was told to read: writing receiver firmware
      // to somebody's ChipSat is not a recoverable mistake.
      body = '<p class="note">Press Read this one first.</p>';
    } else if (!state.firmware) {
      body = '<p class="note">This build carries no receiver firmware.</p>';
    } else {
      const sf = '<select data-flashsf="' + esc(b.port) + '" aria-label="Spreading factor to ask it for">' +
        [6, 7, 8, 9, 10, 11, 12].map((n) => '<option value="' + n + '"' + (n === o.sf ? ' selected' : '') + '>SF' + n + '</option>').join('') +
        '</select>';
      // One press writes (the user, 2026-11-01, as for the ChipSat flight write); what stands
      // in the way is said above: the board must have been claimed, and no job may be running.
      const go = '<button class="primary block" data-flash="' + esc(b.port) + '"' + (state.busy ? ' disabled' : '') + '>Flash ' + esc(name) +
          (state.firmwareVersion ? ' to ' + esc(state.firmwareVersion) : '') + '</button>';
      body = '<p>Erases what is on the board and writes ' + (state.firmwareVersion ? 'receiver firmware ' + esc(state.firmwareVersion) : esc(state.firmware)) +
          ' over the serial port, then asks it for ' + sf + ' once it is back. Reading stops until the board is back.</p>' +
        '<div class="tb-save"><label class="check"><input type="checkbox" data-backup="' + esc(b.port) + '"' + (o.backup ? ' checked' : '') +
          '> Save what is on the board first</label><span class="note">Takes a few minutes.</span></div>' + go;
    }
    return '<div class="pane tb-card2 tb-fw"><h3>Receiver firmware</h3>' +
      row('On the board', val(fw.onBoard === '—' ? '—' : 'Receiver ' + esc(fw.onBoard))) +
      row('App carries', val(esc(fw.inApp)) + (fw.newer ? mark('warn', 'newer') : '')) +
      '<div class="tb-body">' + body + '</div>' + last + '</div>';
  }

  const GAINS = ['0.0', '0.9', '1.4', '2.7', '3.7', '7.7', '8.7', '12.5', '14.4', '15.7', '16.6', '19.7', '20.7', '22.9', '25.4',
    '28.0', '29.7', '32.8', '33.8', '36.4', '37.2', '38.6', '40.2', '42.1', '43.4', '43.9', '44.5', '48.0', '49.6'];   // the R820T's steps

  // The select shows what the program last reported in its header, never what was picked
  function gainSelect(p, reported) {
    const values = GAINS.slice();
    if (reported && reported !== 'auto' && !values.some((v) => Number(v) === Number(reported))) {
      values.push(reported);
      values.sort((a, b) => a - b);
    }
    const on = (v) => (reported === v || (v !== 'auto' && reported !== 'auto' && reported !== undefined && Number(v) === Number(reported)) ? ' selected' : '');
    return '<select data-sdrgain="' + esc(p.key) + '" aria-label="Gain of ' + esc(sdrName(p)) + '"' + (state.sdrBusy.has(p.key) ? ' disabled' : '') + '>' +
      (reported === undefined ? '<option value="" selected disabled>not reported</option>' : '') +
      '<option value="auto"' + on('auto') + '>Automatic (AGC)</option>' +
      values.map((v) => '<option value="' + v + '"' + on(v) + '>' + v + ' dB</option>').join('') + '</select>';
  }

  function sdrHeadHtml(p) {
    const st = sdrState(p);
    const port = portOf(p.key);
    const id = port && port.rxId;
    const f = p.header ? p.header.fields : {};
    const busy = state.sdrBusy.has(p.key);
    const line = [id ? 'SDR ' + id : 'no header yet', f.fw ? 'fw ' + f.fw : ''].filter(Boolean).join(' · ');
    // two characters over: Caprasimo's capitals and digits run wider than the ch nameWidth counts in
    const name = id
      ? '<input type="text" class="nameit" data-keep="name:' + esc(p.key) + '" data-name="' + esc(p.key) + '" data-rxid="' + esc(id) + '"' +
        ' value="' + esc((port && port.name) || '') + '" placeholder="SDR ' + esc(id) + '" style="' + nameWidth(((port && port.name) || 'SDR ' + id) + '  ') +
        '" maxlength="24" aria-label="Name of this SDR" title="Type a name and press Enter">'
      : '<h2 class="tb-sdrname">' + esc(sdrName(p)) + '</h2>';
    const acts = (p.on ? '<button data-sdroff="' + esc(p.key) + '"' : '<button class="primary" data-sdron="' + esc(p.key) + '"') +
        (busy ? ' disabled' : '') + '>' + (p.on ? 'Turn off' : 'Turn on') + '</button>' +
      '<button data-sdrrestart="' + esc(p.key) + '"' + (p.on && !busy ? '' : ' disabled') + '>Restart</button>';
    return '<div class="pane tb-head"><div class="tb-headrow">' + name + mark(st.cls, st.word) +
        '<span class="tb-line">' + esc(line) + '</span><span class="tb-acts">' + acts + '</span></div>' +
      (st.why ? '<p class="outcome bad tb-why">' + esc(st.why) + (st.code ? ' <small>' + esc(st.code) + '</small>' : '') + '</p>' : '') +
      '<p class="tb-notes note">' + (id ? 'The name is kept on this computer. ' : '') + (p.sdrLine
        ? 'Started from the sdr = line in descent-ground.conf; whether it is on and its gain are kept in descent-ground-sdr.txt beside the program.'
        : 'Started from a receiver = line in descent-ground.conf. Turned off, it stays off until the app is started again.') + '</p></div>';
  }

  function sdrRadioCard(p) {
    const f = p.header ? p.header.fields : null;
    if (!f) return '<div class="pane tb-card2"><h3>Radio</h3><p class="note">Not reported yet: no header from the program.</p></div>';
    const sfs = B.headerSfs(p);
    const said = B.gainSaid(p, Date.now());
    const own = "The program's own report. Picking another restarts it with --gain, and counts only when its header says so.";
    const gain = p.sdrLine
      ? gainSelect(p, f.gain) + (said ? mark(said.cls, said.word) + '<span class="hint">' + esc(said.hint) + '</span>' : '<span class="hint">' + own + '</span>')
      : val(f.gain ? esc(B.gainWords(f.gain)) : '—') + '<span class="hint">Set on its receiver = line in descent-ground.conf.</span>';
    return '<div class="pane tb-card2"><h3>Radio</h3>' +
      row('Frequency', f.f ? val(esc(f.f) + ' MHz') : unsaid) +
      row('Bandwidth · CR', f.bw || f.cr ? val(esc([f.bw ? Number(f.bw) + ' kHz' : '', f.cr ? '4/' + f.cr : ''].filter(Boolean).join(' · '))) : unsaid) +
      row('Sync · preamble', f.sync || f.pre ? val(esc([f.sync, f.pre].filter(Boolean).join(' · '))) : unsaid) +
      row('Spreading factors', sfs.length ? val('Listens on ' + esc(B.sfList(sfs))) : unsaid) +
      row('Gain', gain, 'tb-sfrow') + '</div>';
  }

  function sdrSignalCard(p) {
    const b = p.stats && p.stats.beat;
    if (!b) return '<div class="pane tb-card2"><h3>Signal</h3><p class="note">No heartbeat from the program yet.</p></div>';
    const n = (x, f) => (x === null || x === undefined ? null : f(x));
    const clip = n(b.clip, (x) => val(x.toFixed(2) + ' % of samples in the last 5 s'));
    return '<div class="pane tb-card2"><h3>Signal</h3>' +
      row('Clipping', (clip || unsaid) + (b.sat === 1 ? mark('warn', 'saturated') : '')) +
      row('Noise floor', n(b.floor, (x) => val(x.toFixed(1) + ' dBFS')) || unsaid) +
      row('Samples lost', n(b.ovf, (x) => val(count(x)) + '<span class="note">since it started (ovf)</span>') || unsaid) +
      row('Lines dropped', n(b.drop, (x) => val(count(x)) + '<span class="note">since it started (drop)</span>') || unsaid) +
      '<p class="note tb-beat">From its heartbeat ' + age(Date.now() - b.t) + ' ago.</p></div>';
  }

  function sdrSessionCard(p) {
    const now = Date.now();
    const bySf = (p.stats && p.stats.bySf) || [];
    const sfs = [...new Set([...B.headerSfs(p), ...bySf.map((c) => c.sf)])].sort((a, b) => a - b);
    const none = '<span class="note">—</span>';
    const rows = sfs.map((sf) => {
      const c = bySf.find((x) => x.sf === sf) || { packets: 0, badCrc: 0, headerDamaged: 0, other: 0, lastT: null, lastSnr: null, boards: [] };
      return '<tr><td>SF' + sf + '</td><td class="r">' + count(c.packets) + '</td><td class="r">' + count(c.badCrc) + '</td><td class="r">' +
        count(c.headerDamaged) + '</td><td class="r">' + count(c.other) + '</td><td>' + (c.lastT ? age(now - c.lastT) + ' ago' : none) +
        '</td><td class="r">' + (c.lastSnr === null ? none : c.lastSnr.toFixed(2) + ' dB') + '</td><td>' + (c.boards.length ? c.boards.join(', ') : none) + '</td></tr>';
    }).join('');
    const b = p.stats && p.stats.beat;
    const up = p.state === 'running' && b ? 'Up ' + upFor(b.upMs + (now - b.t)) + ' by its heartbeat.' : '';
    const again = p.starts > 1 ? 'Started again ' + (p.starts - 1) + (p.starts === 2 ? ' time' : ' times') + ' since the app started.' : '';
    // a stop the app made is said in its words; any other keeps the program's exit status, small
    const last = !p.lastExit ? '' : p.lastExitByApp ? esc(p.lastExit) + ' at ' + clockS(p.lastExitAt) + '.'
      : 'Last stopped at ' + clockS(p.lastExitAt) + '. <small>' + esc(p.lastExit.replace(/^stopped, /, '')) + '</small>';
    return '<div class="pane tb-card2 tb-wide"><h3>This session</h3>' +
      (sfs.length
        ? '<div class="tb-sfwrap"><table class="grid plain tb-sftable"><thead><tr><th>SF</th><th class="r">Packets</th>' +
          '<th class="r">Bad CRC <small>-7</small></th><th class="r">Header damaged <small>-24</small></th><th class="r">Other <small>-1</small></th>' +
          '<th>Last packet</th><th class="r">Last SNR</th><th>Boards heard</th></tr></thead><tbody>' + rows + '</tbody></table></div>'
        : '<p class="note">Nothing heard yet.</p>') +
      '<p class="note tb-beat">' + ([esc(up), esc(again), last].filter(Boolean).join(' ') || 'Not stopped since the app started.') +
        ' Packets are the ones whose CRC passed; in the log as <b>' + esc(p.key) + '</b>.</p>' +
      '<p class="note tb-cmd">Started by ' + esc(p.command) + '</p></div>';
  }

  function sdrSaidCard(p) {
    const said = p.said || [];
    return '<div class="pane tb-card2 tb-wide"><h3>What it said' + (said.length ? ' <span class="note">its last ' + said.length + (said.length === 1 ? ' line' : ' lines') + ' on stderr</span>' : '') + '</h3>' +
      (said.length ? '<pre class="raw tb-said" data-said="' + esc(p.key) + '">' + esc(said.map((l) => clockS(l.t) + '  ' + l.text).join('\n')) + '</pre>'
        : '<p class="note">Nothing on stderr yet.</p>') + '</div>';
  }

  function sdrSheetHtml(p) {
    return sdrHeadHtml(p) + '<div class="tb-cards tb-sdrcards">' + sdrRadioCard(p) + sdrSignalCard(p) + sdrSessionCard(p) + sdrSaidCard(p) + '</div>';
  }

  // The verbs the toasts use, by action
  const DOING = { on: 'turning on', off: 'turning off', restart: 'restarting', gain: 'restarting' };
  const VERB = { on: 'turn on', off: 'turn off', restart: 'restart', gain: 'set the gain of' };

  async function programAct(key, action, gain) {
    const p = progs().find((x) => x.key === key);
    if (!p || state.sdrBusy.has(key) || !A.programDo) return;
    const name = sdrName(p);
    state.sdrBusy.set(key, action === 'gain' && !p.on ? 'keeping the gain' : DOING[action]);
    render();
    try {
      const j = await A.programDo(action, action === 'gain' ? { key, gain } : { key });
      const left = j.gone ? '' : '; something it started was still running after a kill';
      if (action === 'gain') state.gainAsks.add(key);
      toast((action === 'off' ? 'Turned ' + name + ' off' : action === 'on' ? 'Turned ' + name + ' on' : action === 'restart' ? 'Restarted ' + name
        : p.on ? 'Asked ' + name + ' for ' + B.gainWords(gain) + ', restarting it' : 'Kept ' + B.gainWords(gain) + ' for ' + name + '; it is passed when it is turned on') + left);
    } catch (err) {
      toast('Could not ' + VERB[action] + ' ' + name + ': ' + (err.message || err));
    }
    state.sdrBusy.delete(key);
    render();
  }

  // A gain picked here is toasted once its header says so, or once it cannot
  A.onPrograms = () => {
    for (const key of state.gainAsks) {
      const p = progs().find((x) => x.key === key);
      const said = p && B.gainSaid(p, Date.now());
      if (!said || said.cls === 'warn') continue;
      state.gainAsks.delete(key);
      toast(said.cls === 'good' ? sdrName(p) + ' is on ' + B.gainWords(p.gain) : sdrName(p) + ': ' + said.word + '. ' + said.hint);
    }
    if (A.shell.current() === 'tbeams') render();
  };

  function sheetHtml() {
    const sdr = sdrs().find((p) => p.key === state.sel);
    if (sdr) return sdrSheetHtml(sdr);
    const b = boards().find((x) => x.port === state.sel);
    if (!b) {
      return '<div class="pane"><p class="empty">' + (boards().length
        ? 'Pick a board on the left.'
        : sdrs().length ? 'Pick the SDR on the left, or plug a T-Beam in.' : 'Plug a T-Beam in.') +
        '</p></div>';
    }
    return headHtml(b) + '<div class="tb-cards">' + radioCard(b) + sessionCard(b) + firmwareCard(b) + '</div>';
  }

  function render() {
    const all = boards();
    const programs = sdrs();
    if (!all.some((b) => b.port === state.sel) && !programs.some((p) => p.key === state.sel)) {
      state.sel = all.length ? all[0].port : programs.length ? programs[0].key : null;
    }
    $('tb-ident').disabled = ident.watching;
    $('tb-ident').textContent = ident.watching ? 'Waiting for a replug' : 'Start watching';
    $('tb-ident-stop').hidden = !ident.watching;
    paint($('tb-which'), whichHtml());
    paint($('tb-list'), listHtml() + sdrListHtml() + programsHtml());
    // The sheet redraws each second, which would shut a select the operator has open
    const focus = typeof document === 'undefined' ? null : document.activeElement;
    if (focus && focus.tagName === 'SELECT' && $('tb-sheet').contains(focus)) return;
    // What it said follows its newest line, unless the operator has scrolled up in it; the
    // sheet is redrawn every second while a countdown runs, which makes the box anew each time
    const was = $('tb-sheet').querySelector('.tb-said');
    const keep = was && { key: was.dataset.said, top: was.scrollTop, end: was.scrollTop + was.clientHeight >= was.scrollHeight - 4 };
    paint($('tb-sheet'), sheetHtml());
    const box = $('tb-sheet').querySelector('.tb-said');
    if (box && box !== was) box.scrollTop = keep && keep.key === box.dataset.said && !keep.end ? keep.top : box.scrollHeight;
  }

  // Which boards may be flashed changes when one is claimed or let go, so it is asked
  // again whenever the list of boards does.
  async function candidates(force) {
    const sig = boards().map((b) => b.port + ':' + b.state).join('|') + '/' + hub.ports.length;
    if (!force && sig === state.asked) return;
    state.asked = sig;
    try {
      const j = await hub.candidates();
      state.cands = (j.candidates || []).map((c) => c.port);
      state.firmware = j.firmware || null;
      state.firmwareVersion = j.firmwareVersion || null;
    } catch (e) {
      state.cands = [];
    }
    render();
  }

  function tick() {
    candidates(false);
    render();
  }

  hub.onFlash = (m) => {
    // A ChipSat pull reports its own flashing the same way; that one is not ours.
    if (!state.busy) return;
    state.pct = m.pct || 0;
    state.note = m.note || '';
    A.shell.jobs.tbeam.pct = state.pct;
    A.shell.renderStrip();
    if (A.shell.current() === 'tbeams') render();
  };

  const root = $('screen-tbeams');

  $('tb-ident').addEventListener('click', async () => {
    ident.found = null;
    await hub.identifyStart();
    ident.watching = true; ident.left = 60;
    clearInterval(ident.timer);
    ident.timer = setInterval(pollIdentify, 1000);
    render();
  });

  $('tb-ident-stop').addEventListener('click', async () => {
    // A build without the call just stops being asked; its watch runs out by itself.
    try { await hub.identifyStop(); } catch (e) { /* nothing to stop it with */ }
    clearInterval(ident.timer); ident.timer = null;
    ident.watching = false;
    toast('Stopped watching for a replug');
    render();
  });

  root.addEventListener('change', async (e) => {
    const t = e.target;
    const named = t.getAttribute('data-name');
    if (named) {
      const id = t.getAttribute('data-rxid');
      try {
        let clean; let where = '';
        try {
          if (id) {
            const done = await hub.setReceiverName(id, t.value);
            clean = done.name;
            where = done.onBoard ? ', sent to the board' : ', kept on this computer';
          } else {
            clean = await hub.setName(named, t.value);
          }
        } catch (err) {
          if (!id || !err.missing) throw err;
          clean = await hub.setName(named, t.value);   // an older build names the adapter only
        }
        toast(clean ? 'Called it ' + clean + where : 'Name cleared');
        if (ident.found === named) ident.found = null;
      } catch (err) {
        toast('Could not name it: ' + (err.message || err));
      }
      t.blur();
      render();
      return;
    }
    const backup = t.getAttribute('data-backup');
    if (backup) { optsFor(backup).backup = t.checked; render(); return; }
    const gainOf = t.getAttribute('data-sdrgain');
    if (gainOf) { t.blur(); programAct(gainOf, 'gain', t.value); return; }
    const flashSf = t.getAttribute('data-flashsf');
    if (flashSf) { optsFor(flashSf).sf = Number(t.value); t.blur(); render(); return; }
    const freq = t.getAttribute('data-freq');
    if (freq) { t.blur(); askFreq(freq, t.value); return; }
    const key = t.getAttribute('data-sf');
    if (key) {
      const sf = Number(t.value);
      // Back to whatever the board last reported; the row under it follows the ask, see sfSaid
      t.blur();
      A.askSf(key, sf);
    }
  });

  root.addEventListener('click', async (e) => {
    const item = e.target.closest('[data-pick]');
    if (item) { state.sel = item.getAttribute('data-pick'); state.forget = null; render(); return; }
    const at = (name) => e.target.getAttribute(name);

    const listen = at('data-listen');
    if (listen) {
      state.heard.set(listen, 'Listening for five seconds…');
      render();
      try {
        const j = await hub.testPort(listen);
        state.looks.set(listen, j.looks_like);
        state.heard.set(listen, 'Heard ' + (LOOKS[j.looks_like] || j.looks_like) + (j.lines.length ? ':\n' + j.lines.slice(0, 6).join('\n') : '.'));
      } catch (err) {
        state.heard.set(listen, 'Could not listen: ' + (err.message || err));
      }
      render();
      return;
    }

    const yes = at('data-yes');
    const not = at('data-not');
    if (yes || not) {
      const port = yes || not;
      try {
        const b = boards().find((x) => x.port === port);
        const called = b ? nameOf(b) : port;
        await hub.setBoard(port, !!yes);
        toast(yes ? 'Reading ' + called : 'Not reading ' + called);
      } catch (err) {
        toast('Could not change ' + port + ': ' + (err.message || err));
      }
      await candidates(true);
      return;
    }

    const act = ['on', 'off', 'restart'].find((a) => at('data-sdr' + a));
    if (act) { programAct(at('data-sdr' + act), act); return; }

    if (at('data-forget')) { state.forget = at('data-forget'); render(); return; }
    if (at('data-forgetcancel')) { state.forget = null; render(); return; }
    const gone = at('data-forgetgo');
    if (gone) {
      const b = boards().find((x) => x.port === gone);
      const called = b ? nameOf(b) : gone;
      state.forget = null;
      try {
        await hub.forgetBoard(gone);
        for (const kept of [state.opts, state.heard, state.done, state.flashAsks]) kept.delete(gone);
        toast('Forgot ' + called);
      } catch (err) {
        toast('Could not forget ' + called + ': ' + (err.message || err));
      }
      await candidates(true);
      return;
    }

    const go = at('data-flash');
    if (!go || state.busy) return;
    const o = optsFor(go);
    const b = boards().find((x) => x.port === go);
    const name = b ? nameOf(b) : go;
    state.busy = go; state.pct = 0;
    state.done.delete(go);
    state.flashAsks.delete(go);
    // The whole chip is read before anything is erased. It is minutes, not seconds, so
    // say so rather than let a still line look like a board that stopped answering.
    state.note = o.backup ? "saving the board's current firmware, a few minutes" : 'writing the firmware';
    A.shell.jobs.tbeam = { name, pct: 0 };
    A.shell.renderStrip();
    render();
    try {
      const j = await hub.flash(go, o.sf, null, o.backup);
      // The app says which factor it will ask the board for; an older build says nothing
      const sf = Number(j.sf) || o.sf;
      state.flashAsks.set(go, { sf, at: Date.now(), state: 'asked', was: NaN, said: '', heardSf: NaN, heardAt: 0, back: 0, key: null, backup: j.backup || '', name });
      flashOutcome(go);
      toast('Flashed ' + name + ', asked for SF' + sf);
    } catch (err) {
      state.done.set(go, { bad: true, text: 'Flash failed: ' + (err.message || err) });
      toast('Did not flash ' + name + ': ' + (err.message || err));
    }
    state.busy = null;
    A.shell.jobs.tbeam = null;
    A.shell.renderStrip();
    await candidates(true);
  });

  A.shell.register('tbeams', {
    show() {
      candidates(true);
      render();
      clearInterval(state.timer);
      state.timer = setInterval(tick, 1000);
    },
    hide() { clearInterval(state.timer); state.timer = null; },
    which: whichHtml,
  });
})();
