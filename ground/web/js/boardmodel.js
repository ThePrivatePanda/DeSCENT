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
  // older apps only send fswTree and always built when they had one
  function flashWay(health, module) {
    const local = health && typeof health.fswLocal === 'boolean' ? health.fswLocal : (!health || health.fswTree !== false);
    if (local) {
      return { way: 'build', verb: 'Build and flash', column: 'Build with', stop: '',
        says: 'A flight software tree is here, so this builds the exact image.' };
    }
    const off = !!health && health.fswTree === true;
    const carried = { way: 'carried', verb: 'Patch and flash', column: 'Flash with', stop: '',
      says: off ? 'This computer\'s flight software is switched off, so this patches the image this app carries.'
        : 'No flight software tree, so this patches the image this app carries.' };
    if (!module) return carried;
    const images = Array.isArray(health.flightImages) ? health.flightImages : [];
    if (images.some((i) => i.module === module)) return carried;
    const none = (off ? 'This computer\'s flight software is switched off' : 'No flight software tree') +
      ', and this build carries no flight image for the ' + MODULE[module] + ' module. Flash a file instead.';
    return Object.assign(carried, { says: none, stop: none });
  }

  function intervalWords(ms) {
    if (!Number.isFinite(ms)) return '—';
    return ms === 0 ? 'back to back' : String(Number((ms / 1000).toFixed(2))) + ' s';
  }

  // Slot names from the firmware before 2026-10-22. Ground fills waiting and landed, air fills
  // rising and falling, so older firmware still gets sensible gaps
  const PHASE_NAMES = ['waiting', 'rising', 'falling', 'landed'];
  const PHASES = [
    { key: 'ground', label: 'Ground', slots: [0, 3] },
    { key: 'air', label: 'Air', slots: [1, 2] },
  ];
  // A block from before the phases (layout 1) has one gap, used from switch-on to the end.
  const NO_PHASES = 'no phases in this firmware';

  // 0 means every IMU reading in the air, nothing written on the ground
  function logWords(ms, phase) {
    if (Number(ms) === 0) return phase === 'ground' ? 'nothing written' : 'every reading';
    return 'every ' + intervalWords(ms);
  }

  // One phase's rate as the "On the board" column says it: "every 60 s", or what the phase
  // follows on the battery and what it falls back to.
  function rateWords(r) {
    if (!r) return NO_PHASES;
    if (!r.follow) return r.fixedMs === 0 ? 'back to back' : 'every ' + intervalWords(r.fixedMs);
    const p = r.percent, ms = r.intervalMs;
    const parts = [intervalWords(ms[0]) + ' above ' + p[0] + ' %'];
    for (let i = 1; i < p.length; i++) parts.push(intervalWords(ms[i]) + ' to ' + p[i] + ' %');
    parts.push('then ' + intervalWords(ms[ms.length - 1]));
    return 'by battery: ' + parts.join(', ') + ' (' + intervalWords(r.fixedMs) + ' when the gauge cannot be read)';
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
    // A second slot that differs (set by older firmware) is shown too. Layout 1 has one gap,
    // shown under air since it was the old falling gap. Layout 2 has no log gaps
    PHASES.forEach((ph) => {
      const words = (i) => rateWords(s.rates[i]) + (s.logMs ? ', log ' + logWords(s.logMs[i], ph.key) : '');
      const [first, second] = ph.slots;
      const same = (a, b) => JSON.stringify(s.rates[a]) === JSON.stringify(s.rates[b]) && (!s.logMs || Number(s.logMs[a]) === Number(s.logMs[b]));
      const onBoard = !s ? '—' : s.rates ? words(first) + (same(first, second) ? '' : '; ' + PHASE_NAMES[second] + ' slot ' + words(second))
        : ph.key === 'air' ? 'every ' + intervalWords(s.txIntervalMs) + ' (' + NO_PHASES + ')' : NO_PHASES;
      rows.push({ key: ph.key, label: ph.label, onBoard });
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

  // Which rows of the Build with column would replace what the board holds. Compared as
  // numbers: the board's column reads "+14 dBm" and "every 1 s" where the controls hold 14
  // and 1 (seconds). A box that is empty or half typed is not what is on the board. With no
  // block read there is nothing known to be replaced, so nothing differs. A board with no
  // phases is always replaced in every phase: flashing gives it them.
  function buildDiffers(settings, remembered, draft) {
    const s = settings;
    const none = { module: false, csid: false, sf: false, dbm: false, ground: false, air: false };
    if (!s || !draft) return none;
    const typed = (v) => (/^\s*-?\d+(\.\d+)?\s*$/.test(String(v)) ? Number(v) : NaN);
    const out = {
      module: !!remembered && !!s.module && remembered !== s.module,
      csid: typed(draft.csid) !== Number(s.csid),
      sf: typed(draft.sf) !== Number(s.sf),
      dbm: typed(draft.dbm) !== Number(s.dbm),
    };
    // boxes go into both slots, so either slot differing counts. No log gaps on the board counts too
    PHASES.forEach((ph, n) => {
      const same = ph.slots.every((i) => {
        const r = s.rates ? s.rates[i] : null;
        return !!r && Math.round(typed(draft['p' + n + 's']) * 1000) === Number(r.fixedMs) && !!draft['p' + n + 'f'] === !!r.follow &&
          r.percent.every((p, k) => typed(draft['p' + n + 'soc' + k]) === Number(p)) &&
          r.intervalMs.every((ms, k) => Math.round(typed(draft['p' + n + 'ms' + k]) * 1000) === Number(ms)) &&
          !!s.logMs && Math.round(typed(draft['p' + n + 'l']) * 1000) === Number(s.logMs[i]);
      });
      out[ph.key] = !same;
    });
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
      return { key: p.key, name, sf, word: 'quiet for ' + (min ? min + ' min' : Math.floor(quiet / 1000) + ' s'), cls: 'warn' };
    });
  }

  // The app refuses to write an image that was read off another board unless told to.
  // Any other refusal has no override, so the two have to be told apart.
  function isOtherBoardRefusal(message) {
    return /different board|another board|other board|force=1/i.test(String(message || ''));
  }

  return {
    MODULE, uidWords, sameBoard, chipName, firmwareWords, stillOnProbe, probesChanged, probeToRead, probeToAutoRead, swapSeen, readButton, readToast, dropChoice, chipList, stepList, powerOptions, sfOptions, dbmWords,
    flashWay, settingsTable, buildDiffers, rateWords, logWords, PHASE_NAMES, PHASES, listeningOn, receiverName, namedBy, isNewer, receiverFirmware, receiverWords, isOtherBoardRefusal,
  };
});
