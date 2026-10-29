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
    opts: new Map(),    // port -> {sf, backup, confirm}
    heard: new Map(),   // port -> what Listen said
    done: new Map(),    // port -> {bad, text}: how the last flash of it ended
    busy: null, pct: 0, note: '',
    forget: null,       // port of the board being asked about before it is forgotten
    timer: null,
  };
  const optsFor = (port) => {
    if (!state.opts.has(port)) state.opts.set(port, { sf: 9, backup: false, confirm: false });
    return state.opts.get(port);
  };
  const boards = () => hub.boards || [];

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

  function identBanner() {
    if (ident.found) {
      return '<div class="ident found"><b>' + esc(ident.found) + '</b> is the board you replugged. Name it on its sheet.</div>';
    }
    if (ident.watching) {
      return '<div class="ident"><b>Unplug the board, then plug it back in.</b> Waiting ' + ident.left + ' s.</div>';
    }
    return '';
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

  function listHtml() {
    const all = boards();
    if (!all.length) return identBanner() + '<p class="empty">Nothing plugged in.</p>';
    return identBanner() + all.map((b) => {
      const [cls, word] = STATE_WORDS[b.state] || ['off', b.state];
      const rx = reading(b);
      const sf = rx && rx.r && rx.r.info.sf ? '<span class="val">SF' + rx.r.info.sf + '</span>' : '';
      return '<button class="item' + (b.port === state.sel ? ' on' : '') + '" data-pick="' + esc(b.port) + '">' +
        '<b>' + esc(nameOf(b)) + '</b>' +
        '<small>' + sf + mark(cls, word) + (state.busy === b.port ? ' ' + mark('warn', 'being flashed') : '') + '</small>' +
        '<small>' + esc([b.label, b.port].filter(Boolean).join(', ')) + '</small>' +
        '</button>';
    }).join('');
  }

  function readingPane(b) {
    const is = b.state === 'receiver';
    const decide = is
      ? '<button data-not="' + esc(b.port) + '">Stop reading it</button>'
      : '<button data-yes="' + esc(b.port) + '" class="primary">Read this one</button>' +
        (b.state === 'ignored' ? '' : '<button data-not="' + esc(b.port) + '">Leave it alone</button>');
    const said = state.heard.get(b.port);
    return '<div class="pane"><h3>Reading</h3><div class="rows">' +
      '<div>' + decide +
        (is ? '' : '<span class="note">Opening a port can reset the board on it.</span>') +
        (b.state === 'dismissed' ? '<span class="note">Stays shut until it is replugged.</span>' : '') + '</div>' +
      '<div><button data-listen="' + esc(b.port) + '">Listen</button>' +
        '<span class="note">Opens the port for 5 s and says what it heard.</span></div>' +
      (said ? '<pre class="testout">' + esc(said) + '</pre>' : '') +
      '</div></div>';
  }

  function receiverPane(b) {
    const rx = reading(b);
    if (!rx) return '';
    const r = rx.r;
    const row = (label, value) => '<div><span class="l">' + label + '</span>' + value + '</div>';
    const val = (text) => '<span class="val">' + text + '</span>';
    const open = rx.port.status === 'open';
    const id = B.namedBy(rx.port, r).id;
    const rows = [
      open ? '' : row('Port', mark('bad', rx.port.status) + (rx.port.error ? ' ' + esc(rx.port.error) : '')),
      // The select is the board's own report of what it is on, so it stays plain ink.
      row('Spreading factor', r && r.info.sf ? sfSelect(rx.key, r.info.sf) +
        '<span class="note">Hears only boards on this spreading factor.</span>'
        : '<span class="note">not reported yet</span>'),
      row('Packets', r ? val(r.unique + ' new, ' + (r.packets - r.unique - r.badCrc) + ' duplicate' +
        (r.badCrc ? ', ' + r.badCrc + ' bad CRC' : '') + (r.radioErrors ? ', ' + r.radioErrors + ' radio errors' : ''))
        : '<span class="note">none yet</span>'),
      row('Last heard', r ? val(age(Date.now() - r.lastT) + ' ago') : mark('warn', 'nothing heard') +
        '<span class="note">Wrong spreading factor, or not a receiver. Listen to check.</span>'),
      row('Receiver id', id ? '<span class="val hex">' + esc(id) + '</span>' : '<span class="note">not announced yet</span>'),
      row('In the log as', val(esc(rx.key))),
    ];
    return '<div class="pane"><h3>Receiver</h3><div class="rows kvrows">' + rows.join('') + '</div></div>';
  }

  function firmwarePane(b) {
    const o = optsFor(b.port);
    const busy = state.busy === b.port;
    const done = state.done.get(b.port);
    const last = done ? '<p class="outcome' + (done.bad ? ' bad' : '') + '">' + esc(done.text) + '</p>' : '';
    let body;
    if (busy) {
      body = '<div class="job">' + track(state.pct) + '<p class="step">' + esc(state.note) + '</p></div>';
    } else if (!state.cands.includes(b.port)) {
      // flash.rs offers only boards the app was told to read: writing receiver firmware
      // to somebody's ChipSat is not a recoverable mistake.
      body = '<div class="rows"><div><span class="note">Press Read this one first.</span></div></div>';
    } else if (!state.firmware) {
      body = '<div class="rows"><div><span class="note">This build carries no receiver firmware.</span></div></div>';
    } else {
      const sf = '<select data-flashsf="' + esc(b.port) + '" aria-label="Spreading factor it comes up on">' +
        [6, 7, 8, 9, 10, 11, 12].map((n) => '<option value="' + n + '"' + (n === o.sf ? ' selected' : '') + '>SF' + n + '</option>').join('') +
        '</select>';
      const go = o.confirm
        ? '<span class="grow">Erase <b>' + esc(nameOf(b)) + '</b> and write the receiver firmware?' +
            (o.backup ? '' : ' Its current firmware is not saved.') + '</span>' +
          '<button data-flash="' + esc(b.port) + '" class="primary">Overwrite it</button>' +
          '<button data-cancel="' + esc(b.port) + '">Cancel</button>'
        : '<button data-ask="' + esc(b.port) + '"' + (state.busy ? ' disabled' : '') + '>Flash the receiver firmware</button>' +
          '<span class="note">Writes ' + esc(state.firmware) + '.</span>';
      body = '<div class="rows">' +
        '<div><span class="l">Comes up on</span>' + sf + '</div>' +
        '<div><label class="check"><input type="checkbox" data-backup="' + esc(b.port) + '"' + (o.backup ? ' checked' : '') +
          '> Save the old firmware first</label><span class="note">Takes a few minutes.</span></div>' +
        '<div>' + go + '</div></div>';
    }
    const rx = reading(b);
    const fw = B.receiverFirmware(rx ? rx.port : null, state.firmwareVersion);
    const versions = '<div class="rows kvrows">' +
      '<div><span class="l">On the board</span><span class="val">' + esc(fw.onBoard) + '</span></div>' +
      '<div><span class="l">In this app</span><span class="val">' + esc(fw.inApp) + '</span>' + (fw.newer ? mark('warn', 'newer') : '') + '</div>' +
      '</div>';
    return '<div class="pane"><h3>Receiver firmware</h3>' + versions + body + last + '</div>';
  }

  function sheetHtml() {
    const b = boards().find((x) => x.port === state.sel);
    if (!b) {
      return '<div class="pane"><p class="empty">' + (boards().length
        ? 'Pick a board on the left.'
        : 'Plug a T-Beam in.') +
        '</p></div>';
    }
    const [cls, word] = STATE_WORDS[b.state] || ['off', b.state];
    const detail = [b.label, 'USB ' + b.usb, b.serial ? 'serial ' + b.serial : '', b.port].filter(Boolean).map(esc).join(', ');
    // A receiver that has announced its id is named by that id, and the name goes with
    // the board to any port. Before that, a name can only be kept for the USB adapter.
    const rx = reading(b);
    const how = rx ? B.namedBy(rx.port, rx.r) : { by: 'port', id: '' };
    const shown = how.by === 'board' ? nameOf(b) : how.by === 'computer' ? (rx.port.name || b.name || '') : (b.name || '');
    const blank = how.id ? 'T-Beam ' + how.id : b.port;
    const kept = how.by === 'board' ? 'name is on the board'
      : how.by === 'computer' ? 'name kept on this computer'
        : 'name kept for this USB adapter';
    return '<div class="pane sheet-head">' +
        '<input type="text" class="nameit" data-keep="name:' + esc(b.port) + '" data-name="' + esc(b.port) + '"' +
          (how.id ? ' data-rxid="' + esc(how.id) + '"' : '') +
          ' value="' + esc(shown) + '" placeholder="' + esc(blank) + '" style="' + nameWidth(shown || blank) +
          '" maxlength="' + (how.id ? 24 : 40) + '" aria-label="Name of this board" title="Type a name and press Enter">' +
        mark(cls, word) +
        (ident.found === b.port ? mark('good', 'the one you replugged') : '') +
        '<span class="note">' + kept + '</span>' +
        '<span class="spacer"></span><span class="note">' + detail + '</span>' +
        '<button data-forget="' + esc(b.port) + '"' + (state.busy === b.port ? ' disabled' : '') + '>Forget this board</button>' +
        // Forgetting drops what the app keeps about the board and sends it nothing.
        (state.forget === b.port
          ? '<div class="confirm"><span class="grow">Forget <b>' + esc(nameOf(b)) + '</b>? Its name on this computer and whether to read it go, ' +
              'and its port is shut. Nothing is sent to the board' + (how.by === 'board' ? ', so the name kept on it stays' : '') +
              '. It goes back to not touched.</span>' +
            '<button data-forgetgo="' + esc(b.port) + '" class="primary">Forget it</button>' +
            '<button data-forgetcancel="1">Cancel</button></div>'
          : '') +
      '</div>' +
      '<div class="two"><div class="col">' + readingPane(b) + receiverPane(b) + '</div><div class="col">' + firmwarePane(b) + '</div></div>';
  }

  function render() {
    const all = boards();
    if (!all.some((b) => b.port === state.sel)) state.sel = all.length ? all[0].port : null;
    $('tb-ident').disabled = ident.watching;
    $('tb-ident').textContent = ident.watching ? 'Waiting for a replug' : 'Which board is which?';
    $('tb-ident-stop').hidden = !ident.watching;
    paint($('tb-list'), listHtml());
    paint($('tb-sheet'), sheetHtml());
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
    const flashSf = t.getAttribute('data-flashsf');
    if (flashSf) { optsFor(flashSf).sf = Number(t.value); t.blur(); render(); return; }
    const key = t.getAttribute('data-sf');
    if (key) {
      const sf = Number(t.value);
      // Back to whatever the board last reported; the select settles when its next header arrives.
      t.blur();
      hub.setSf(key, sf).then(() => toast('Asked ' + A.rxName(key) + ' for SF' + sf))
        .catch((err) => toast('Could not set SF: ' + (err.message || err)));
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

    if (at('data-forget')) { state.forget = at('data-forget'); render(); return; }
    if (at('data-forgetcancel')) { state.forget = null; render(); return; }
    const gone = at('data-forgetgo');
    if (gone) {
      const b = boards().find((x) => x.port === gone);
      const called = b ? nameOf(b) : gone;
      state.forget = null;
      try {
        await hub.forgetBoard(gone);
        for (const kept of [state.opts, state.heard, state.done]) kept.delete(gone);
        toast('Forgot ' + called);
      } catch (err) {
        toast('Could not forget ' + called + ': ' + (err.message || err));
      }
      await candidates(true);
      return;
    }

    const ask = at('data-ask');
    const cancel = at('data-cancel');
    if (ask || cancel) { optsFor(ask || cancel).confirm = !!ask; render(); return; }

    const go = at('data-flash');
    if (!go || state.busy) return;
    const o = optsFor(go);
    const b = boards().find((x) => x.port === go);
    const name = b ? nameOf(b) : go;
    o.confirm = false;
    state.busy = go; state.pct = 0;
    state.done.delete(go);
    // The whole chip is read before anything is erased. It is minutes, not seconds, so
    // say so rather than let a still line look like a board that stopped answering.
    state.note = o.backup ? "saving the board's current firmware, a few minutes" : 'writing the firmware';
    A.shell.jobs.tbeam = { name, pct: 0 };
    A.shell.renderStrip();
    render();
    try {
      const j = await hub.flash(go, o.sf, null, o.backup);
      state.done.set(go, { bad: false, text: 'Flashed, comes up on SF' + o.sf + '.' + (j.backup ? ' The old firmware is saved as ' + j.backup + '.' : '') });
      toast('Flashed ' + name + ' on SF' + o.sf);
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
  });
})();
