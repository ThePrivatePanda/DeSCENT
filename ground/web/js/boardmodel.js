// What the ChipSats screen and the bar say about boards, in words: the list, the steps of
// a running job, the radio table and which flight image a board may be given.
// No page in here, so it can be tested.
(function (root, factory) {
  const mod = factory();
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).boardmodel = mod;
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  const MODULE = { le: 'LE', hp: 'HP' };
  const CSID_UNSET = 255;   // what a board is built with before anyone gives it an id

  function uidWords(uid) {
    return String(uid || '').replace(/(.{4})(?=.)/g, '$1 ');
  }

  // A board's id is 24 hex digits. The files the app saves carry only its last eight, so
  // one id matches another when the shorter is the end of the longer.
  function sameBoard(a, b) {
    const x = String(a || '').toLowerCase();
    const y = String(b || '').toLowerCase();
    if (x.length < 8 || y.length < 8) return false;
    return x.length >= y.length ? x.endsWith(y) : y.endsWith(x);
  }

  function chipName(chip) {
    return chip.name || 'Unnamed board …' + String(chip.uid || '').slice(-4);
  }

  // What was read is only known to be under the clip while its probe stays plugged in.
  function stillOnProbe(read, probes) {
    return !!(read && read.probe && (probes || []).some((p) => p.serial === read.probe.serial));
  }

  // The app cannot see a board being swapped under a probe that stays plugged in, so what
  // was read is dropped as soon as the probes plugged in change in any way: one appears,
  // one goes, one is replaced.
  function probesChanged(before, after) {
    const key = (list) => (list || []).map((p) => p.serial).sort().join('|');
    return key(before) !== key(after);
  }

  // Which probe a read goes to. picked is the serial of the probe whose row is selected.
  // why is what to say when there is none to read.
  function probeToRead(probes, picked) {
    const all = probes || [];
    if (!all.length) return { serial: null, why: 'No probe found' };
    const open = all.filter((p) => p.access !== false);
    if (!open.length) return { serial: null, why: 'No permission to open the probe' };
    if (open.length === 1) return { serial: open[0].serial, why: '' };
    const mine = open.find((p) => p.serial === picked);
    if (mine) return { serial: mine.serial, why: '' };
    return { serial: null, why: (open.length === 2 ? 'Two' : open.length) + ' probes are plugged in. Pick one in the list.' };
  }

  // Which probe is read without the button being pressed: one that has been plugged in
  // since the last look. handled is the probes already read, or tried, while they have
  // stayed plugged in; one that has gone is dropped, so it is new again when it comes
  // back. Several new at once are left for the operator to pick between.
  function probeToAutoRead(probes, handled) {
    const all = probes || [];
    const kept = (handled || []).filter((serial) => all.some((p) => p.serial === serial));
    const fresh = all.filter((p) => !kept.includes(p.serial) && p.access !== false);
    if (fresh.length === 1) return { serial: fresh[0].serial, handled: kept };
    return { serial: null, handled: kept.concat(fresh.map((p) => p.serial)) };
  }

  // Whether what answered under the clip calls for a read. peeked is the id it gave, or
  // null when nothing answered, which says nothing. shown is the id the sheet has for that
  // probe. tried is the id the last read nobody asked for was started on, so a board that
  // answers and then fails its read is not read again and again.
  function swapSeen(peeked, shown, tried) {
    return !!peeked && peeked !== shown && peeked !== tried;
  }

  // The button at the head of the list. It reads the board when there is a probe to read
  // it through, and otherwise only looks for one.
  function readButton(probes, busy) {
    return (probes || []).length
      ? { label: 'Read the board', title: 'Reads the board on the ST-Link. Can reset the board.', disabled: !!busy }
      : { label: 'Look for a probe', title: 'Looks for an ST-Link on USB', disabled: !!busy };
  }

  // What a read answers with. chips is the remembered boards from before the read, so a
  // board met for the first time is said to be new.
  function readToast(uid, chips) {
    const known = (chips || []).find((c) => c.uid === uid);
    return (known ? chipName(known) : 'New board …' + String(uid).slice(-4)) + ' on the ST-Link';
  }

  // An image chosen to be written, or a refusal, belongs to the board it was chosen on.
  // It goes when a different board is read. One chosen before any board was read waits.
  function dropChoice(chosenOn, readBefore, readNow) {
    return (!!chosenOn && chosenOn !== readNow) || (!!readBefore && readBefore !== readNow);
  }

  // What a board is running, in words. fw is what a read found, {kind, current, module,
  // id}, or what was remembered of it, {kind, module}. remembered is the module the
  // operator set for the board. Only the dump firmware and the flight images this app
  // carries can be told; anything else is said to be not recognised, which is not the
  // same as flight software and not the same as fine.
  // fw.built e.g. "flight LE built here 14:02 from abc1234"
  function firmwareWords(fw, remembered) {
    const none = { kind: '', head: '', value: '', note: '', mismatch: '', bad: false, built: '' };
    if (!fw || !fw.kind) return none;
    const built = fw.built && fw.built.words ? String(fw.built.words) : '';
    const cap = (t) => t.charAt(0).toUpperCase() + t.slice(1);
    if (fw.kind === 'dump') {
      // A build that does not say which dump it found is taken to mean its own. The id goes in the
      // same way the flight case carries one, so two dump images can be told apart on sight.
      const id = fw.id ? ' ' + String(fw.id).slice(0, 8) : '';
      const which = fw.current === false ? 'Earlier dump firmware' + id : 'Dump firmware' + id;
      return Object.assign({}, none, {
        kind: 'dump',
        head: 'running ' + which.charAt(0).toLowerCase() + which.slice(1) + ', not flight software',
        value: which,
        built,
        note: 'Not flight software. This board transmits nothing until flight software is on it.',
        bad: true,
      });
    }
    if (fw.kind === 'bench') {
      const forModule = MODULE[fw.module] || '';
      return Object.assign({}, none, {
        kind: 'bench',
        head: 'running the sensor bench, not flight software',
        value: 'Sensor bench' + (forModule ? ' for the ' + forModule + ' module' : ''),
        note: 'Not flight software. It tests the sensors and sends as board 254. Write flight software before it flies.',
        bad: true,
        built,
      });
    }
    if (fw.kind === 'flight') {
      const forModule = MODULE[fw.module] || '';
      return Object.assign({}, none, {
        kind: 'flight',
        value: built ? cap(built) : 'Flight software' + (fw.id ? ' ' + String(fw.id).slice(0, 8) : '') + (forModule ? ' for the ' + forModule + ' module' : ''),
        mismatch: forModule && MODULE[remembered] && remembered !== fw.module
          ? 'This image is for the ' + forModule + ' module and the board is marked ' + MODULE[remembered] + '.' : '',
        built,
      });
    }
    return Object.assign({}, none, {
      kind: 'unknown', value: 'Not recognised', note: 'Not the dump firmware and not a flight image this app carries or this computer built.',
    });
  }

  // Remembered boards in name order, so the list does not shuffle when one is read, then
  // any probe whose board has not been read yet. fw is what is known of a board's
  // firmware: the read itself while it is on the probe, else the kind and module the app
  // remembered.
  function chipList(chips, probes, read) {
    const rows = (chips || []).map((c) => {
      const on = !!read && read.uid === c.uid;
      const kept = c.firmware ? Object.assign({ kind: c.firmware }, c.firmwareModule ? { module: c.firmwareModule } : {}) : null;
      const fw = on && read.firmware ? read.firmware : kept;
      return {
        id: c.uid, uid: c.uid, name: c.name || '', module: c.module || null,
        firmware: fw ? fw.kind : null, fw,
        lastSeen: c.lastSeen || 0, onProbe: on, probe: on ? read.probe : null, unread: false,
      };
    });
    if (read && read.uid && !rows.some((r) => r.uid === read.uid)) {
      rows.push({
        id: read.uid, uid: read.uid, name: '', module: null,
        firmware: read.firmware ? read.firmware.kind : null, fw: read.firmware || null,
        lastSeen: 0, onProbe: true, probe: read.probe || null, unread: false,
      });
    }
    rows.sort((a, b) => chipName(a).localeCompare(chipName(b), 'en', { numeric: true }));
    for (const p of probes || []) {
      if (read && read.probe && read.probe.serial === p.serial) continue;
      rows.push({
        id: 'probe:' + p.serial, uid: null, name: '', module: null, firmware: null, fw: null, lastSeen: 0,
        onProbe: false, probe: p, unread: true,
      });
    }
    return rows;
  }

  // The app reports a job as the notes it has said so far. Every one but the last is
  // finished; the last is what it is doing, or what it was doing when it failed.
  function stepList(steps, running, error) {
    const said = [];
    for (const s of steps || []) if (said[said.length - 1] !== s) said.push(s);
    return said.map((text, i) => ({
      text,
      state: i < said.length - 1 ? 'done' : running ? 'now' : error ? 'failed' : 'done',
    }));
  }

  // radio is what /api/radio serves, written by the flight software build. The page
  // holds no list of its own, so it cannot offer a power the firmware would refuse.
  function powerOptions(radio, module) {
    const m = radio && module ? radio[module] : null;
    return m && Array.isArray(m.dbm) ? m.dbm.map((d) => d.dbm) : [];
  }

  function sfOptions(radio) {
    const r = radio && radio.spreadingFactor;
    if (!r || !Number.isFinite(r.min) || !Number.isFinite(r.max)) return [];
    const out = [];
    for (let n = r.min; n <= r.max; n++) out.push(n);
    return out;
  }

  const dbmWords = (n) => (n < 0 ? '−' + Math.abs(n) : '+' + n) + ' dBm';

  // How flight software gets onto a board from this screen. It follows from what the app
  // has beside it and is not a choice: with a flight software tree the image is built for
  // the board, and without one, as with a downloaded release, the image the app carries
  // for the board's module has the settings written into its block. health is
  // /api/health; a build of the app from before it said which is taken to have a tree,
  // since building was all it did. stop is why nothing can be flashed this way.
  function flashWay(health, module) {
    if (!health || health.fswTree !== false) {
      return { way: 'build', verb: 'Build and flash', column: 'Build with', stop: '',
        says: 'A flight software tree is here, so this builds the exact image.' };
    }
    const carried = { way: 'carried', verb: 'Patch and flash', column: 'Flash with', stop: '',
      says: 'No flight software tree, so this patches the image this app carries.' };
    if (!module) return carried;
    const images = Array.isArray(health.flightImages) ? health.flightImages : [];
    if (images.some((i) => i.module === module)) return carried;
    const none = 'No flight software tree, and this build carries no flight image for the ' + MODULE[module] + ' module. Flash a file instead.';
    return Object.assign(carried, { says: none, stop: none });
  }

  function intervalWords(ms) {
    if (!Number.isFinite(ms)) return '—';
    return ms === 0 ? 'back to back' : String(Number((ms / 1000).toFixed(2))) + ' s';
  }

  // The block's layout this page writes. The app refuses a page asking for any other.
  const LAYOUT = 4;
  const PHASES = [
    { key: 'ground', label: 'Ground' },
    { key: 'air', label: 'Air' },
  ];
  const BANDS = 5;
  // The thresholds the firmware is compiled with. Written only behind a battery switch that is
  // off, where the firmware never reads them; a switch that is on needs every box typed.
  const INERT_PERCENT = [75, 50, 35, 20];
  const FIXED = 'fixed';   // the row of a table that is no band: Always, or Gauge unread
  // A block from before the phases (layout 1) has one gap, used from switch-on to the end.
  const NO_PHASES = 'no phases in this firmware';

  // 0 means every IMU reading in the air, nothing written on the ground
  function logZero(phase) { return phase === 'ground' ? 'nothing written' : 'every reading'; }
  function logWords(ms, phase) {
    return Number(ms) === 0 ? logZero(phase) : 'every ' + intervalWords(ms);
  }

  // One rate as the page says it: "every 60 s", or what it follows on the battery and what it
  // falls back to. zero is what a gap of 0 means for this rate, back to back unless told.
  function rateWords(r, zero) {
    if (!r) return NO_PHASES;
    const gap = (ms) => (Number(ms) === 0 ? zero || 'back to back' : intervalWords(ms));
    if (!r.follow) return Number(r.fixedMs) === 0 ? gap(0) : 'every ' + intervalWords(r.fixedMs);
    const p = r.percent, ms = r.intervalMs;
    const parts = [gap(ms[0]) + ' above ' + p[0] + ' %'];
    for (let i = 1; i < p.length; i++) parts.push(gap(ms[i]) + ' to ' + p[i] + ' %');
    parts.push('then ' + gap(ms[ms.length - 1]));
    return 'by battery: ' + parts.join(', ') + ' (' + gap(r.fixedMs) + ' when the gauge cannot be read)';
  }

  // A row of the table as it reads: Above 75 %, To 50 %, To 35 %, To 20 %, Below 20 %.
  function bandLabel(percent, k) {
    return k === 0 ? 'Above ' + percent[0] + ' %'
      : k === BANDS - 1 ? 'Below ' + percent[BANDS - 2] + ' %'
        : 'To ' + percent[k] + ' %';
  }

  // What one cell holds: the gap the rate gives in that row.
  const gapIn = (r, k) => (k === FIXED || !r.follow ? r.fixedMs : r.intervalMs[k]);
  const secsOr = (ms, zero) => (Number(ms) === 0 ? zero : intervalWords(ms));
  const zeroOf = (column, phase) => (column === 'log' ? logZero(phase) : 'back to back');
  const typed = (v) => (/^\s*-?\d+(\.\d+)?\s*$/.test(String(v)) ? Number(v) : NaN);
  const toMs = (v) => Math.round(typed(v) * 1000);

  // The draft's rate for a phase's column. p<n>s and p<n>ms<k> are the transmit boxes, p<n>l and
  // p<n>lms<k> the log ones, in seconds. The switch and the thresholds are the phase's, one set
  // for both columns. A box that is empty or half typed is NaN, never 0.
  function draftRate(d, n, column) {
    const log = column === 'log';
    return {
      fixedMs: toMs(d['p' + n + (log ? 'l' : 's')]), follow: !!d['p' + n + 'f'],
      percent: [0, 1, 2, 3].map((k) => typed(d['p' + n + 'soc' + k])),
      intervalMs: [0, 1, 2, 3, 4].map((k) => toMs(d['p' + n + (log ? 'lms' : 'ms') + k])),
    };
  }

  // Why the draft cannot be written yet: a box that is empty or half typed, or the id 255 that
  // every board is built with. The draft has no defaults (2026-10-31, the user's call, after a
  // one-press write put 255, SF7 and 14 dBm on a real board): a board read without a block
  // starts blank, and a blank must never become 0 or a default on the way to the board.
  function draftStop(d) {
    const bad = (v) => Number.isNaN(typed(v));
    const missing = [];
    if (bad(d.csid)) missing.push('the board id');
    if (bad(d.sf)) missing.push('the spreading factor');
    if (bad(d.dbm)) missing.push('the transmit power');
    let boxes = 0;
    PHASES.forEach((ph, n) => {
      if (d['p' + n + 'f']) {
        for (let k = 0; k < BANDS - 1; k++) if (bad(d['p' + n + 'soc' + k])) boxes++;
        for (let k = 0; k < BANDS; k++) { if (bad(d['p' + n + 'ms' + k])) boxes++; if (bad(d['p' + n + 'lms' + k])) boxes++; }
      } else {
        if (bad(d['p' + n + 'l'])) boxes++;
        if (bad(d['p' + n + 's'])) boxes++;
      }
    });
    if (boxes) missing.push(boxes + (boxes === 1 ? ' box' : ' boxes') + ' in Phases');
    if (missing.length) {
      const list = missing.length === 1 ? missing[0] : missing.slice(0, -1).join(', ') + ' and ' + missing[missing.length - 1];
      return 'Fill in ' + list + ' first.';
    }
    if (typed(d.csid) === CSID_UNSET) return 'Set a board id first.';
    return '';
  }

  // A board's phase whose two columns don't share a switch or thresholds. The firmware doesn't
  // mind, the page writes them the same, so this is only said.
  function columnsApart(tx, log) {
    if (!tx || !log) return '';
    if (!!tx.follow !== !!log.follow) {
      return 'On the board ' + (tx.follow ? 'transmit follows the battery and the log does not.' : 'the log follows the battery and transmit does not.');
    }
    if (tx.percent.join() !== log.percent.join()) {
      return 'On the board the thresholds differ: log ' + log.percent.join(', ') + ' %, transmit ' + tx.percent.join(', ') + ' %.';
    }
    return '';
  }

  // What a read with no block in it is told. The dump firmware never has one; any other image
  // without one is taken to be flight software from before the block existed.
  const NO_BLOCK = 'This board\'s firmware predates the settings block, so the id, spreading factor, power and interval ' +
    'it was built with cannot be read off it. Flashing it from here gives it one.';
  const NO_BLOCK_DUMP = 'The dump firmware has no radio settings. Flashing from here puts flight software on this board.';

  // The "On the board" column. settings is the block /api/chip/read found, or null when
  // the firmware has none. remembered is the module the operator chose for this board,
  // firmware the kind the read found ('dump', 'flight', 'unknown').
  function settingsTable(settings, remembered, error, firmware) {
    const s = settings;
    const rows = [
      { key: 'csid', label: 'Board id', onBoard: s ? (Number(s.csid) === CSID_UNSET ? 'unconfigured (' + s.csid + ')' : String(s.csid)) : '—' },
      { key: 'sf', label: 'Spreading factor', onBoard: s ? 'SF' + s.sf : '—' },
      { key: 'dbm', label: 'Transmit power', onBoard: s ? dbmWords(s.dbm) : '—' },
    ];
    // A spare slot that differs (older firmware) is shown too. Layout 1 has one gap, shown
    // under air since it was the old falling gap. Layout 2 has no log gaps
    PHASES.forEach((ph, n) => {
      const tx = s && s.transmit ? s.transmit[n] : null;
      const log = s && s.log ? s.log[n] : null;
      const words = (rate, logMs) => rateWords(rate) + (logMs == null ? '' : ', log ' + logMs);
      const older = (s && s.olderSlots || []).filter((o) => o.phase === n)
        .map((o) => o.name + ' slot ' + words(o.rate, o.logMs == null ? null : logWords(o.logMs, ph.key)));
      const onBoard = !s ? '—' : tx ? [words(tx, log ? rateWords(log, logZero(ph.key)) : null)].concat(older).join('; ')
        : ph.key === 'air' ? 'every ' + intervalWords(s.txIntervalMs) + ' (' + NO_PHASES + ')' : NO_PHASES;
      rows.push({ key: ph.key, label: ph.label, onBoard, older, apart: columnsApart(tx, log) });
    });
    if (!s) {
      // What the app said about the block it could not give, as it said it.
      if (error) return { rows, module: '', setBy: '', disagree: '', line: String(error), damaged: /damaged/i.test(String(error)) };
      return { rows, module: '', setBy: '', disagree: '', damaged: false, line: firmware === 'dump' ? NO_BLOCK_DUMP : NO_BLOCK };
    }
    const onBoard = MODULE[s.module] || String(s.module || '');
    return {
      rows,
      module: onBoard,
      setBy: s.setBy === 'build' ? 'from the build, not set from here' : s.setBy === 'tool' ? 'set from this tool' : '',
      disagree: remembered && s.module && remembered !== s.module
        ? "The board's settings say " + onBoard + ' and this app has it down as ' + MODULE[remembered] +
          '. Check the label on the module.'
        : '',
      line: '',
      damaged: false,
    };
  }

  // What a write changes in one phase, row by row, as [what, was, now]. Compared as numbers: the
  // board's column reads "every 1 s" where the controls hold 1 (seconds). A box that is empty or
  // half typed is not what is on the board. A board with no phases, no log gaps or a spare slot
  // has that replaced whatever the boxes say: flashing gives it the block as it is now.
  function phaseChanges(settings, draft, n) {
    const s = settings;
    const ph = PHASES[n];
    const out = [];
    const want = { transmit: draftRate(draft, n, 'transmit'), log: draftRate(draft, n, 'log') };
    const sayRate = (column) => rateWords(want[column], zeroOf(column, ph.key));
    if (!s.transmit) {
      return [[ph.label, ph.key === 'air' ? 'every ' + intervalWords(s.txIntervalMs) + ' (' + NO_PHASES + ')' : NO_PHASES,
        sayRate('transmit') + ', log ' + sayRate('log')]];
    }
    (s.olderSlots || []).filter((o) => o.phase === n).forEach((o) => {
      out.push([ph.label + ', ' + o.name + ' slot', rateWords(o.rate), 'gone: the block has one slot a phase']);
    });
    const board = { transmit: s.transmit[n], log: s.log ? s.log[n] : null };
    const columns = ['log', 'transmit'];
    columns.forEach((c) => {
      const was = board[c];
      if (!was) { out.push([ph.label + ', ' + c, 'no log gaps', sayRate(c)]); return; }
      if (!!was.follow !== want[c].follow) {
        out.push([ph.label + ', ' + c + ' by battery level', was.follow ? 'on' : 'off', want[c].follow ? 'on' : 'off']);
      }
    });
    for (let k = 0; k < BANDS - 1; k++) {
      const off = columns.filter((c) => board[c] && Number(board[c].percent[k]) !== want[c].percent[k]);
      const was = off.map((c) => board[c].percent[k]);
      if (off.length === 2 && was[0] === was[1]) off.pop();
      off.forEach((c, i) => {
        out.push([ph.label + ', ' + (off.length === 1 && columns.every((x) => board[x]) && was[0] === board[columns[1 - columns.indexOf(c)]].percent[k] ? '' : c + ' ') + 'threshold ' + (k + 1),
          was[i] + ' %', (Number.isNaN(want[c].percent[k]) ? '—' : want[c].percent[k]) + ' %']);
      });
    }
    const showing = want.log.follow ? want.log.percent : (board.log || board.transmit).percent;
    columns.forEach((c) => {
      const was = board[c];
      if (!was) return;
      const zero = zeroOf(c, ph.key);
      const row = (what, a, b) => { if (Number(a) !== b) out.push([ph.label + ', ' + what + ', ' + c, secsOr(a, zero), secsOr(b, zero)]); };
      row(want[c].follow ? 'gauge unread' : 'always', was.fixedMs, want[c].fixedMs);
      for (let k = 0; k < BANDS; k++) {
        const label = bandLabel(showing, k).toLowerCase();
        // What it gives there, or when neither side uses the table, what the table holds.
        if (want[c].follow || was.follow) row(label, gapIn(was, k), gapIn(want[c], k));
        else row(label + ' (not in use)', was.intervalMs[k], want[c].intervalMs[k]);
      }
    });
    return out;
  }

  // The rows the page draws for a phase, top down: each band and the gauge unread with the switch
  // on, one Always row with it off. For each, what the board holds and whether the box replaces
  // it. With no block read there is nothing known to be replaced.
  function phaseRows(settings, draft, n) {
    const follow = !!draft['p' + n + 'f'];
    const keys = follow ? [0, 1, 2, 3, 4, FIXED] : [FIXED];
    const phase = PHASES[n].key;
    const board = settings && settings.transmit
      ? { transmit: settings.transmit[n], log: settings.log ? settings.log[n] : null } : null;
    return keys.map((k) => {
      const cell = (column) => {
        const was = board && board[column];
        if (!was) return { was: '', changed: !!settings };
        const want = draftRate(draft, n, column);
        if (!follow && was.follow) return { was: 'by battery', changed: true };
        return { was: secsOr(gapIn(was, k), zeroOf(column, phase)), changed: Number(gapIn(was, k)) !== gapIn(want, k) };
      };
      return { key: k, log: cell('log'), transmit: cell('transmit') };
    });
  }

  // Which controls would replace what the board holds. With no block read there is nothing known
  // to be replaced, so nothing differs.
  function buildDiffers(settings, remembered, draft) {
    const s = settings;
    const none = { module: false, csid: false, sf: false, dbm: false, ground: false, air: false };
    if (!s || !draft) return none;
    const out = {
      module: !!remembered && !!s.module && remembered !== s.module,
      csid: typed(draft.csid) !== Number(s.csid),
      sf: typed(draft.sf) !== Number(s.sf),
      dbm: typed(draft.dbm) !== Number(s.dbm),
    };
    PHASES.forEach((ph, n) => { out[ph.key] = phaseChanges(s, draft, n).length > 0; });
    return out;
  }

  // Who would hear a board on this spreading factor. receivers is receiverWords():
  // the receivers being read, each with the factor it last reported.
  function listeningOn(sf, receivers) {
    const names = (receivers || []).filter((r) => r.sf === 'SF' + sf).map((r) => r.name);
    if (!names.length) return { none: true, words: 'no receiver is on SF' + sf };
    const list = names.length === 1 ? names[0] : names.slice(0, -1).join(', ') + ' and ' + names[names.length - 1];
    return { none: false, words: list + (names.length === 1 ? ' is' : ' are') + ' on SF' + sf };
  }

  // What a receiver is called. Every receiver announces an id that is the same on any
  // port (from the ESP32's MAC), and newer firmware a name. port is its entry in
  // /api/ports, heard is what the page itself has read off it, board is its USB adapter's
  // entry, which may carry a name given before ids were used. The key (rx1, rx2) is the
  // order ports were found in, so it is only used when nothing else is known.
  function receiverName(key, port, heard, board) {
    const own = (port && port.rxName) || (heard && heard.info && heard.info.name);
    const id = (port && port.rxId) || (heard && heard.id);
    return own || (port && port.name) || (board && board.name) || (id ? 'T-Beam ' + id : key);
  }

  // Where a receiver's name is kept: on the board when it reports one, on this computer
  // against its id when it has announced one, else against its USB adapter.
  function namedBy(port, heard) {
    const id = (port && port.rxId) || (heard && heard.id) || '';
    const own = (port && port.rxName) || (heard && heard.info && heard.info.name);
    return { by: !id ? 'port' : own ? 'board' : 'computer', id };
  }

  // Dot-separated numbers compared as numbers, so 1.10.0 is newer than 1.2.1.
  function isNewer(a, b) {
    const parts = (v) => (/^\d+(\.\d+)*$/.test(String(v || '')) ? String(v).split('.').map(Number) : null);
    const x = parts(a); const y = parts(b);
    if (!x || !y) return false;
    for (let i = 0; i < Math.max(x.length, y.length); i++) {
      const d = (x[i] || 0) - (y[i] || 0);
      if (d) return d > 0;
    }
    return false;
  }

  // Receivers report their version from this one on. One that has announced its id and
  // gives no version is older.
  const FIRST_VERSIONED = '1.2.1';

  // port is the receiver's entry in /api/ports, inApp the version of the image the app carries.
  function receiverFirmware(port, inApp) {
    const fw = port && port.rxFw;
    const old = !fw && !!(port && port.rxId);
    return {
      onBoard: fw || (old ? 'older than ' + FIRST_VERSIONED : '—'),
      inApp: inApp || '—',
      newer: !!inApp && (old || isNewer(inApp, fw)),
    };
  }

  // The receivers being read, for the bar on every screen. heard is the live fleet's
  // receivers: key -> {id, info: {sf}, lastT}.
  function receiverWords(ports, boards, heard, now) {
    return (ports || []).map((p) => {
      const b = (boards || []).find((x) => x.port === p.port);
      const r = heard.get(p.key);
      const name = receiverName(p.key, p, r, b);
      const sf = r && r.info && r.info.sf ? 'SF' + r.info.sf : '';
      if (p.status !== 'open') return { key: p.key, name, sf, word: p.status, cls: 'bad' };
      if (!r) return { key: p.key, name, sf, word: 'nothing heard', cls: 'warn' };
      const quiet = now - r.lastT;
      if (quiet < 7000) return { key: p.key, name, sf, word: 'reading', cls: 'good' };
      const min = Math.floor(quiet / 60000);
      return { key: p.key, name, sf, word: 'quiet for ' + (min ? min + ' min' : Math.floor(quiet / 1000) + ' s'), cls: 'off' };
    });
  }

  // The app refuses to write an image that was read off another board unless told to.
  // Any other refusal has no override, so the two have to be told apart.
  function isOtherBoardRefusal(message) {
    return /different board|another board|other board|force=1/i.test(String(message || ''));
  }

  return {
    MODULE, uidWords, sameBoard, chipName, firmwareWords, stillOnProbe, probesChanged, probeToRead, probeToAutoRead, swapSeen, readButton, readToast, dropChoice, chipList, stepList, powerOptions, sfOptions, dbmWords,
    flashWay, settingsTable, buildDiffers, phaseChanges, phaseRows, draftRate, draftStop, INERT_PERCENT, bandLabel, rateWords, logWords, logZero, LAYOUT, PHASES, BANDS, FIXED, listeningOn, receiverName, namedBy, isNewer, receiverFirmware, receiverWords, isOtherBoardRefusal,
  };
});
