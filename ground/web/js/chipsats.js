// The ChipSats screen: the boards the app remembers and the one on the debug probe.
// A list on the left; the picked board's sheet on the right, with its radio module, its
// firmware and its flight log. Attaching can reset a board, so it is done when a button
// is pressed, and once when a probe is plugged in while this screen is up.
(function () {
  'use strict';
  const A = window.DGApp;
  const { $, esc, toast, logUrl, flatName } = A.ui;
  const { paint, mark, steps, track, nameWidth, jobs } = A.shell;
  const B = DG.boardmodel;
  const D = DG.datamodel;
  const hub = A.hub;
  if (!hub.native) return;

  const CANNOT = 'This build of the app cannot do that yet.';
  const ANY = 'any';   // the one row an older build gets: whatever is on the probe

  const state = {
    missing: false,     // the app has no /api/chips: it can pull, and nothing else here
    chips: [], probes: [], logs: [], health: null, radio: null,
    fsw: null,          // /api/fsw, for the write confirm. null on older apps
    read: null,         // what the last Read the board said, while its probe stays plugged in
    readAt: 0,          // when, by this computer's clock
    stale: '',          // the job that has changed the board since it was read
    autoRead: [],       // probes already read, or tried, since they were plugged in
    peek: null,         // the question "which board is under the clip" while it is being asked
    tried: {},          // probe -> the board the last read nobody asked for was started on
    looks: 0,
    reading: false, readError: '',
    sel: null,
    loaded: false,      // the app has answered once, so an unknown selection really is gone
    keepFsw: false,     // nothing is saved unless asked
    // off, a fresh image is written anyway
    backup: false,
    pick: null,         // the image about to be written: {kind, label, name, bytes, saved}
    pickFor: null,      // the board it was chosen on, when it was chosen on one
    build: null,        // settings to compile into a fresh image: {uid, csid, sf, dbm, interval}
    refused: '', canForce: false,
    open: {},           // 'radio:<uid>' a folded card on a bench board, 'pulls:<uid>' every pull, shown
    endedAt: {},        // when the pull and the chip job last ended, by kind, for the card under the head
    sent: null,         // the draft the last flash was asked with, remembered for its board once it ends well
    misses: 0,          // id checks in a row that nothing answered, while a board was shown as on the probe
    homeNote: '',       // why the probe's own view is up: what the last id checks saw
    timer: null,
  };

  // Which board a job belongs to. The app says for a flash; for a pull it is remembered
  // here, in the browser, so a reloaded page puts the pull back on the right sheet.
  const store = {
    get(key) { try { return localStorage.getItem(key); } catch (e) { return null; } },
    set(key, v) { try { localStorage.setItem(key, v); } catch (e) { /* storage blocked */ } },
  };
  const pullOwner = () => (jobs.pull && jobs.pull.uid) || store.get('dg-pull-board') || null;
  const chipOwner = () => (jobs.chip && jobs.chip.uid) || null;
  const busy = () => !!((jobs.pull && jobs.pull.running) || (jobs.chip && jobs.chip.running) || state.reading);

  const onProbe = () => (B.stillOnProbe(state.read, state.probes) ? state.read : null);
  const rows = () => (state.missing
    ? [{ id: ANY, uid: null, name: '', module: null, firmware: null, fw: null, lastSeen: 0, onProbe: false, probe: null, unread: false }]
    : B.chipList(state.chips, state.probes, onProbe()));
  const nameOf = (row) => (row.id === ANY ? 'the board on the debug probe' : row.unread ? 'the board on the ST-Link' : B.chipName(row));
  const titleOf = (row) => (row.id === ANY ? 'ChipSat on the debug probe' : row.unread ? 'The board on the ST-Link' : B.chipName(row));
  const when = (epochS) => D.day(new Date(epochS * 1000), new Date());
  const imagesOf = (uid) => state.logs.filter((l) => l.kind === 'image' && B.sameBoard(l.uid, uid));
  const pullsOf = (uid) => state.logs.filter((l) => l.kind === 'pull' && B.sameBoard(l.uid, uid));
  // calendar days, so 23:50 is yesterday at 00:10. Rounded for DST days
  function daysAgo(d, now) {
    const midnight = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
    const n = Math.round((midnight(now) - midnight(d)) / 86400000);
    return n <= 0 ? 'today' : n === 1 ? 'yesterday' : n + ' days ago';
  }

  // ---------- asking the app ----------
  async function load() {
    const got = await Promise.all([
      hub.chips().catch((e) => e),
      hub.chipProbes().catch((e) => e),
      hub.logs().catch(() => []),
      hub.radio().catch(() => null),
      hub.health().catch(() => null),
      hub.fsw ? hub.fsw().catch(() => null) : null,
    ]);
    state.missing = got[0] instanceof Error && !!got[0].missing;
    state.chips = Array.isArray(got[0]) ? got[0] : [];
    seeProbes(Array.isArray(got[1]) ? got[1] : []);
    state.logs = got[2];
    state.radio = got[3] && got[3].layoutVersion ? got[3] : null;
    state.health = got[4];
    state.fsw = got[5] && got[5].ok ? got[5] : null;
    state.loaded = true;
    render();
    readNewProbe();
  }

  // A board can be swapped under a probe without the app seeing it, so nothing is called
  // "on the ST-Link" unless it was read since the probes last changed.
  function forgetRead() {
    state.read = null;
    state.readAt = 0;
    state.stale = '';
    state.misses = 0;
  }
  // The probe's own view: about whatever is under the clip now, not a remembered board. The
  // sheet goes there when the board shown stops answering, when another board answers, or when
  // a read fails, so nothing said about the probe lands under a board that is not on it
  // (2026-10-31, the user: a swapped board kept the old board's sheet and its errors).
  function goHome(serial, note) {
    forgetRead();
    state.homeNote = note || '';
    state.sel = 'probe:' + serial;
  }
  function seeProbes(probes) {
    if (B.probesChanged(state.probes, probes)) forgetRead();
    state.probes = probes;
    // A probe that has gone is new when it comes back, whatever was up when it went.
    state.autoRead = B.probeToAutoRead(probes, state.autoRead).handled;
    for (const serial of Object.keys(state.tried)) {
      if (!probes.some((p) => p.serial === serial)) delete state.tried[serial];
    }
  }

  // Asks which probes are plugged in, and whether a job was started from another window.
  // It attaches to nothing unless a probe has appeared since it last asked.
  async function lookForProbes() {
    if (!busy()) A.shell.pollJobs();
    if (state.missing || busy()) return;
    try { seeProbes(await hub.chipProbes()); } catch (e) { return; }
    render();
    readNewProbe();
    if (state.looks++ % 2) peek();
  }

  // Nothing touches a board unasked unless this screen is up and nothing else is going
  // on: not during a job, and not while something is waiting to be confirmed, so the
  // sheet does not change under a decision.
  const quiet = () => !state.missing && !busy() && !jobs.tbeam && !state.pick && A.shell.current() === 'chipsats';

  // A probe that has just been plugged in is read once without the button. A read that
  // fails is not tried again.
  function readNewProbe() {
    if (!quiet()) return;
    const auto = B.probeToAutoRead(state.probes, state.autoRead);
    if (auto.serial) read(auto.serial, true);
  }

  // A board swapped under a probe that stays plugged in changes nothing the app can see
  // without asking the board. So every other look asks it for its id, in the way that
  // cannot reset it (stm32::peek), and a different id from the one on the sheet is read.
  // With two probes plugged in nothing is asked: which one is the operator's to say.
  async function peek() {
    if (!quiet() || state.peek) return;
    const to = B.probeToRead(state.probes, null).serial;
    // Not before the probe's own first read: that one is readNewProbe's.
    if (!to || !state.autoRead.includes(to)) return;
    state.peek = hub.chipPeek(to).catch(() => null);
    const uid = await state.peek;
    state.peek = null;
    if (!quiet()) return;
    const shown = state.read && state.read.probe && state.read.probe.serial === to ? state.read.uid : null;
    if (!uid) {
      // Nothing answered: lifted off the clip, or mid-reset. One check says nothing; the
      // third in a row (checks are 2 s apart) says the board shown is gone.
      if (shown && ++state.misses >= 3) { goHome(to, 'Nothing has answered on the ST-Link since ' + clock(Date.now() - 4000) + '.'); render(); }
      return;
    }
    state.misses = 0;
    if (!B.swapSeen(uid, shown, state.tried[to])) return;
    state.tried[to] = uid;
    // another board is under the clip: the sheet shown says nothing true about it
    if (shown) goHome(to, '');
    read(to, true);
  }

  // unasked is a read nobody pressed a button for.
  async function read(serial, unasked) {
    if (busy()) return;
    if (!state.autoRead.includes(serial)) state.autoRead.push(serial);
    if (!unasked) delete state.tried[serial];
    state.reading = true; state.readError = '';
    render();
    // One thing on the probe at a time, and the app refuses a second.
    if (state.peek) await state.peek;
    const before = state.read ? state.read.uid : null;
    const known = state.chips;
    try {
      const j = await hub.chipRead(serial);
      // A different board from the one shown before starts clean: nothing chosen or
      // refused for that one is this one's.
      if (B.dropChoice(state.pickFor, before, j.uid)) {
        state.pick = null; state.pickFor = null; state.refused = ''; state.canForce = false;
      }
      state.read = j;
      state.readAt = Date.now();
      state.stale = '';
      state.misses = 0;
      state.homeNote = '';
      state.sel = j.uid;
      state.chips = await hub.chips().catch(() => state.chips);
      toast(B.readToast(j.uid, known));
    } catch (e) {
      // No board answered, so no board is known to be there, and what went wrong is said on
      // the probe's own view, never under a board's name.
      goHome(serial, '');
      state.readError = e.missing ? CANNOT : (e.message || String(e));
      toast(state.readError);
    }
    state.reading = false;
    render();
  }

  // A board still on id 255 answers to the same id as every other board nobody has set
  // up. Known from the block while it is on the probe, else from what the app remembers.
  function unconfigured(r) {
    if (r.onProbe && state.read && state.read.settings) return Number(state.read.settings.csid) === 255;
    const kept = state.chips.find((c) => c.uid === r.uid);
    return !!kept && Number(kept.csid) === 255;
  }

  // ---------- the list ----------
  // The board on the probe gets a card of its own above the boards the app remembers.
  // Board id: as read off the board, else as the app remembers it.
  function csidOf(r) {
    if (r.onProbe && state.read && state.read.settings) return Number(state.read.settings.csid);
    const kept = state.chips.find((c) => c.uid === r.uid);
    return kept && kept.csid != null ? Number(kept.csid) : null;
  }
  const two = (n) => String(n).padStart(2, '0');
  const clock = (ms) => { const d = new Date(ms); return two(d.getHours()) + ':' + two(d.getMinutes()); };

  function rowHtml(r, inCard) {
    const pills = [];
    let sub = [];
    if (r.id === ANY) {
      sub = ['this build cannot tell boards apart'];
    } else if (r.unread) {
      sub = [mark('off', 'not read'), esc(r.probe.kind + ' ' + r.probe.serial)];
    } else {
      pills.push(r.module ? mark('good', B.MODULE[r.module]) : mark('warn', 'module never set'));
      if (unconfigured(r)) pills.push(mark('off', 'unconfigured'));
      const id = csidOf(r);
      if (id !== null && id !== 255) sub.push('Board ' + id);
      if (r.firmware === 'dump') sub.push(mark('warn', 'dump firmware'));
      if (r.firmware === 'bench') sub.push(mark('warn', 'sensor bench'));
      if (r.onProbe) {
        const fw = B.firmwareWords(r.fw, r.module);
        if (fw.kind === 'flight' || fw.kind === 'unknown') sub.push(esc(fw.value));
        if (state.readAt) sub.push('read ' + clock(state.readAt));
      } else {
        sub.push(r.lastSeen ? 'last on the probe ' + esc(when(r.lastSeen)) : 'not read yet');
      }
    }
    const job = (jobs.pull && jobs.pull.running && pullOwner() === r.uid && r.uid) ? mark('warn', 'pulling')
      : (jobs.chip && jobs.chip.running && chipOwner() === r.uid && r.uid) ? mark('warn', 'flashing') : '';
    if (job) sub.push(job);
    return '<button class="item' + (!inCard && r.id === state.sel ? ' on' : '') + '" data-pick="' + esc(r.id) + '">' +
      '<span class="nm"><b>' + esc(titleOf(r)) + '</b>' + pills.join('') + '</span>' +
      '<small>' + sub.join(' · ') + '</small></button>';
  }

  function listHtml() {
    const all = rows();
    const onClip = all.filter((r) => r.id === ANY || r.unread || r.onProbe);
    const kept = all.filter((r) => !onClip.includes(r));
    const probe = state.probes.length === 1 ? esc(state.probes[0].kind + ' ' + state.probes[0].serial)
      : state.probes.length > 1 ? state.probes.length + ' probes' : '';
    // the ring is the board whose sheet is up
    const cards = onClip.length
      ? onClip.map((r) => '<div class="card probe' + (r.id === state.sel ? ' ring' : '') + '">' + rowHtml(r, true) +
          (r.unread ? '<button data-read="' + esc(r.probe.serial) + '" class="primary"' + (busy() ? ' disabled' : '') + '>' +
            (state.reading ? 'Reading the board' : 'Read the board') + '</button>' : '') + '</div>').join('')
      : '<div class="card probe"><p class="none">Nothing on the probe.</p></div>';
    const list = kept.length
      ? '<div class="kick"><span class="kicker">Remembered · ' + kept.length + (kept.length === 1 ? ' board' : ' boards') + '</span></div>' +
        kept.map((r) => rowHtml(r, false)).join('')
      : !all.length ? '<p class="empty">No board yet.</p>' : '';
    return '<div class="kick"><span class="kicker">On the ST-Link</span><span class="note">' + probe + '</span></div>' + cards + list +
      '<p class="note cs-foot">' + READ_WARNING + '</p>';
  }

  // ---------- the sheet ----------
  const READ_WARNING = 'Reading can reset the board. A board is read without asking when it is first seen under the clip.';
  const cap = (t) => t.charAt(0).toUpperCase() + t.slice(1);

  function headPane(r) {
    if (r.id === ANY) {
      return '<div class="pane sheet-head"><div class="top"><span class="id">ChipSat on the debug probe</span></div>' +
        '<span class="note line">This build can pull a flight log. It cannot read, name or flash a ChipSat.</span></div>';
    }
    if (r.unread) {
      return '<div class="pane sheet-head"><div class="top"><span class="id">The board on the ST-Link</span>' + mark('off', 'not read') +
        '<span class="note">' + esc(r.probe.kind + ' ' + r.probe.serial) + '</span>' +
        (r.probe.access === false ? mark('bad', 'no permission to open the probe') : '') + '</div>' +
        '<div class="line"><button data-read="' + esc(r.probe.serial) + '" class="primary"' + (busy() ? ' disabled' : '') + '>' +
          (state.reading ? 'Reading the board' : 'Read the board') + '</button>' +
          '<span class="note">' + READ_WARNING + '</span></div>' +
        (state.homeNote ? '<span class="note line">' + esc(state.homeNote) + '</span>' : '') +
        (state.readError ? '<p class="outcome bad line">Could not read it: ' + esc(state.readError) + '</p>' : '') +
        (state.pick ? '<p class="note line">Chosen to write: ' + esc(state.pick.label) + '. Read the board first.</p>' : '') +
        '</div>';
    }
    const probe = r.onProbe ? r.probe : state.probes[0];
    const readBtn = probe
      ? '<button data-read="' + esc(probe.serial) + '"' + (busy() ? ' disabled' : '') + ' title="' + esc(READ_WARNING) + '">' +
        (state.reading ? 'Reading the board' : r.onProbe ? 'Read again' : 'Read the board') + '</button>'
      : '';
    const where = r.onProbe ? mark('warn', 'on the ST-Link') + '<span class="note">' + esc((r.probe.kind || '') + ' ' + (r.probe.serial || '')) + '</span>'
      : mark('off', r.lastSeen ? 'last on the probe ' + when(r.lastSeen) : 'not read yet');

    // What the board is running is only what the last read found. It is a value while
    // the board is on the probe, and a note about the past once it is not.
    const fw = B.firmwareWords(r.fw, r.module);
    const now = r.onProbe && !state.stale;
    const running = !r.onProbe ? 'Not on the ST-Link.' + (fw.value ? ' ' + cap(fw.value) + ' when last read.' : '')
      : state.stale ? 'Changed by the ' + state.stale + '. Read again.'
        : fw.value || 'Not read';
    const ident = 'UID <span class="hex">' + esc(B.uidWords(r.uid)) + '</span>' +
      (r.onProbe && state.read.flashBytes ? ' · ' + D.bytes(state.read.flashBytes) + ' of flash' : '') +
      ' · <span' + (now && !fw.bad ? ' class="val"' : '') + '>' + esc(running) + '</span>';
    const notes = now ? [fw.note, fw.built && fw.value.toLowerCase() !== fw.built.toLowerCase() ? cap(fw.built) : '']
      .filter(Boolean).map((t) => '<span class="note line">' + esc(t) + '</span>').join('') : '';
    const wrongModule = fw.mismatch && !(r.onProbe && state.stale) ? '<p class="outcome bad line">' + esc(fw.mismatch) + '</p>' : '';
    const alarms = [
      fw.head ? mark('warn', fw.head) +
        (now ? '' : '<span class="note">when it was last read</span>') : '',
      r.module ? '' : mark('warn', 'module never set'),
    ].filter(Boolean).join('');
    // Forgetting drops the app's record of the board and nothing else.
    const forgetting = state.pick && state.pick.kind === 'forget' && state.pickFor === r.uid
      ? '<div class="confirm"><span class="grow">Forget <b>' + esc(nameOf(r)) + '</b>? Its name and module go from this computer. ' +
          'The board is not touched, and the images and logs saved off it stay. Read again, it comes back as a new board.</span>' +
        '<button data-forgetgo="1" class="primary"' + (busy() ? ' disabled' : '') + '>Forget it</button>' +
        '<button data-forgetcancel="1">Cancel</button></div>'
      : '';
    return '<div class="pane sheet-head"><div class="top">' +
      '<input type="text" class="nameit" data-keep="chip:' + esc(r.uid) + '" data-name="' + esc(r.uid) + '" value="' + esc(r.name) +
        '" placeholder="' + esc(B.chipName(r)) + '" style="' + nameWidth(r.name || B.chipName(r)) + '" maxlength="40" aria-label="Name of this board" title="Type a name and press Enter">' +
      '<span class="where">' + where + '</span>' +
      '<span class="cs-ident">' + ident + '</span>' +
      '<span class="acts">' + readBtn + '<button data-forget="1"' + (busy() ? ' disabled' : '') + '>Forget this board</button></span></div>' +
      (alarms ? '<div class="line">' + alarms + '</div>' : '') + notes + wrongModule +
      // a bench board says so in its own card
      (isBench(r) ? '' : consoleDisagrees(r)) +
      (probe && !r.onProbe ? '<span class="note line">' + READ_WARNING + '</span>' : '') +
      forgetting +
      '</div>';
  }

  // The receivers being read now, each with the spreading factor it last reported.
  const receiversNow = () => B.receiverWords(hub.ports, hub.boards, A.live.receivers, Date.now());

// The powers the chosen module has rows for. radio.json is the firmware's own table, so a power
  // that is not here would fail the build rather than produce a quiet image.
  function powersFor(module) {
    const table = state.radio && state.radio[module] && state.radio[module].dbm;
    return table ? table.map((row) => Number(row.dbm)) : (module === 'hp' ? [14, 17, 20, 22] : [10, 14, 15]);
  }

  // One 55-byte packet on air at 125 kHz, CR 4/7, from the firmware's own timeOnAirUs. Shown beside
  // the spreading factor because it is what the choice costs: SF12 holds the radio for over 3 s.
  const AIR_MS = { 5: 47, 6: 80, 7: 143, 8: 257, 9: 456, 10: 854, 11: 1823, 12: 3187 };

  const seconds = (ms) => String(Number((ms / 1000).toFixed(3)));

  // The draft as the boxes hold it: id, SF, power, and per phase the switch, the thresholds and
  // the gaps of both columns. From a block, or blank when there is none. No defaults anywhere
  // (2026-10-31, the user's call): a blank box stays blank until someone types in it.
  function draftFrom(uid, was) {
    const d = {
      uid,
      csid: was && was.csid != null ? String(was.csid) : '',
      sf: was && was.sf != null ? Number(was.sf) : '',
      dbm: was && was.dbm != null ? Number(was.dbm) : '',
    };
    B.PHASES.forEach((ph, n) => {
      const tx = was && was.transmit ? was.transmit[n] : null;
      const log = was && was.log ? was.log[n] : null;
      d['p' + n + 's'] = tx ? seconds(tx.fixedMs) : '';
      d['p' + n + 'f'] = !!(tx && tx.follow);
      for (let k = 0; k < B.BANDS - 1; k++) d['p' + n + 'soc' + k] = tx ? String(tx.percent[k]) : '';
      for (let k = 0; k < B.BANDS; k++) d['p' + n + 'ms' + k] = tx ? seconds(tx.intervalMs[k]) : '';
      d['p' + n + 'l'] = log ? seconds(log.fixedMs) : '';
      for (let k = 0; k < B.BANDS; k++) d['p' + n + 'lms' + k] = log ? seconds(log.intervalMs[k]) : '';
    });
    return d;
  }

  // The last settings known for a board, kept in this browser: the block last read off it, and
  // the draft last written to it. A board that comes back on the dump firmware after a pull has
  // no block, and this is what its boxes start from.
  const draftKey = (uid) => 'dg-draft-' + uid;
  function remembered(uid) {
    try { const d = JSON.parse(store.get(draftKey(uid)) || 'null'); return d && typeof d === 'object' ? d : null; } catch (e) { return null; }
  }
  function remember(uid, d) {
    if (!uid || !d) return;
    const keep = Object.assign({}, d);
    delete keep.uid;
    store.set(draftKey(uid), JSON.stringify(keep));
  }

  // The boxes start from the block read off the board, else from what this browser remembers
  // of it, else empty. n is the B.PHASES index (0 ground, 1 air). p<n>f is the phase's battery
  // switch and p<n>soc<k> its thresholds in %, one set for both columns. p<n>s and p<n>ms<k> are
  // the transmit gap and bands in seconds, p<n>l and p<n>lms<k> the log ones.
  function buildDraft(r) {
    const was = r.onProbe && !state.stale && state.read ? state.read.settings : null;
    if (!state.build || state.build.uid !== r.uid) {
      if (was) {
        state.build = draftFrom(r.uid, was);
        remember(r.uid, state.build);
      } else {
        const kept = remembered(r.uid);
        state.build = kept ? Object.assign(draftFrom(r.uid, null), kept, { uid: r.uid }) : draftFrom(r.uid, null);
      }
    }
    return state.build;
  }

  // Slots in the block's order: ground transmit, air transmit, ground log, air log. The switch
  // and the thresholds go to both of a phase's slots; the firmware treats them independently.
  // With the switch off the band table is not consulted by the firmware, and its boxes are not
  // on the sheet, yet the app wants one for every slot: then every band is the one gap the
  // operator typed, and thresholds nobody typed are the firmware's own table. Nothing the
  // sheet shows is ever filled in from a default (draftStop refuses while a shown box is blank).
  function ratesAsked(d) {
    const out = { layout: B.LAYOUT };
    for (let i = 0; i < 4; i++) {
      const r = B.draftRate(d, i % 2, i < 2 ? 'transmit' : 'log');
      out['fixed' + i] = String(r.fixedMs);
      out['follow' + i] = r.follow ? '1' : '0';
      const percent = r.follow || r.percent.every((p) => !Number.isNaN(p)) ? r.percent : B.INERT_PERCENT;
      out['soc' + i] = percent.join(',');
      out['socms' + i] = (r.follow ? r.intervalMs : r.intervalMs.map((ms) => (Number.isNaN(ms) ? r.fixedMs : ms))).join(',');
    }
    return out;
  }

  // Settings go into the image's block, whether the image is built here or carried, and the app
  // checks them against the firmware's own table before anything runs: a power the module has no
  // row for, or a battery table the firmware would refuse, does not flash and says why. It goes on
  // the first press (2026-10-31, the user's call): those checks are what stand between the press
  // and the board, not a second click.
async function buildAndFlash() {
    const r = rows().find((x) => x.id === state.sel);
    if (!r) return;
    // Never nothing: a press that cannot go through says which reason it is.
    const { d, how, stop } = flightWrite(r);
    if (stop) { state.buildError = stop; render(); return; }
    state.buildError = '';
    if (state.peek) await state.peek;
    try {
      const settings = Object.assign({ csid: d.csid, sf: d.sf, dbm: d.dbm }, ratesAsked(d));
      await hub.chipFlash(Object.assign({ uid: r.uid, backup: state.backup }, settings,
        how.way === 'build' ? { embedded: 'built', module: r.module } : { embedded: 'flight' }));
      // kept once the flash has ended well, for the next time this board has no block
      state.sent = { uid: r.uid, draft: Object.assign({}, d) };
      toast(how.way === 'build' ? 'Building the flight software for ' + nameOf(r) : 'Flashing ' + nameOf(r));
    } catch (e) {
      state.buildError = e.missing ? CANNOT : (e.message || String(e));
    }
    await A.shell.pollJobs();
    render();
  }

  // The flight write with the draft's settings: why it cannot go yet, and one line naming what
  // is not in the Phases tables. Asked from the Write step, or from the Sensor bench card.
  function flightWrite(r) {
    const d = buildDraft(r);
    const how = B.flashWay(state.health, r.module);
    const stop = !r.module ? 'Choose the radio module in Identity and radio first.'
      : how.stop ? how.stop
        : !r.onProbe ? 'Put the board on the ST-Link and press Read the board.'
          : busy() ? 'Something else is running on this board.' : B.draftStop(d);
    const buildOff = stop ? ' disabled' : '';
    const say = (v) => (String(v).trim() === '' ? '—' : String(v));
    const summary = 'board ' + say(d.csid) + ', ' + B.MODULE[r.module || 'le'] + ', SF' + say(d.sf) + ', ' + say(d.dbm) + ' dBm';
    return { d, how, stop, buildOff, summary };
  }

  // The flight write in three steps: what goes in the board's block, how often each phase
  // logs and transmits, then the write and what it changes on the board, row by row.
  function stepsHtml(r) {
    const read = r.onProbe && !state.stale ? state.read : null;
    const settings = read ? read.settings || null : null;
    const t = B.settingsTable(settings, r.module, read ? read.settingsError : '', read && read.firmware ? read.firmware.kind : '');
    const seg = '<span class="seg" role="group" aria-label="Radio module">' + ['le', 'hp'].map((m) =>
      '<button data-module="' + m + '" aria-pressed="' + (r.module === m) + '"' + (r.module === m ? ' class="on"' : '') + '>' + B.MODULE[m] + '</button>').join('') + '</span>';
    const { d, how, stop, buildOff, summary } = flightWrite(r);
    const powers = powersFor(r.module || 'le');
    const box = (field, value, hint) => '<input type="text" inputmode="numeric" data-keep="build:' + field + '"' +
      ' data-build="' + field + '" value="' + esc(String(value)) + '" placeholder="' + hint + '"' +
      ' aria-label="' + hint + '" size="8">';
    const pick = (field, values, value, show) => '<select data-build="' + field + '">' +
      (String(value).trim() === '' ? '<option value="" selected disabled hidden>—</option>' : '') +
      values.map((v) => '<option value="' + v + '"' + (Number(v) === Number(value) ? ' selected' : '') + '>' +
        show(v) + '</option>').join('') + '</select>';
    const differs = B.buildDiffers(settings, r.module, d);
    const wrapChanged = (html, replaces) => replaces ? '<span class="changed">' + html + '</span>' : html;
    // The board's value, struck through when the control above it would replace it. Only a
    // read gives one; a board off the probe, or changed since it was read, has none to show.
    const onBoard = (words, replaced) => !words || words === '—' ? ''
      : '<p class="hint">On the board: <span class="val' + (replaced ? ' was' : '') + '">' + esc(words) + '</span>' +
        (replaced ? ' <b class="chg">changed</b>' : '') + '</p>';
    const field = (label, control, hint, board) => '<span class="l">' + label + '</span><div>' + control +
      '<p class="hint">' + hint + '</p>' + board + '</div>';
    const head = (n, title, note) => '<h3><span class="stepno">' + n + '</span>' + title + (note ? '<span class="note">' + esc(note) + '</span>' : '') + '</h3>';

    // Only a read can say a board has no block. One that is off the probe, or changed since
    // it was read, gets no reason.
    const noBlock = read && t.line ? '<p class="' + (t.damaged ? 'outcome bad' : 'plain') + '">' + esc(t.line) + '</p>' : '';
    const idCard = '<div class="pane step">' + head(1, 'Identity and radio', t.setBy) +
      (t.disagree ? '<p class="outcome bad">' + esc(t.disagree) + '</p>' : '') + noBlock +
      '<div class="form">' +
        field('Board id', wrapChanged(box('csid', d.csid, '1 to 255'), differs.csid),
          '1 to 255, goes in every packet. 254 is the sensor bench\'s.', onBoard(t.rows[0].onBoard, differs.csid)) +
        field('Radio module', wrapChanged(seg, differs.module),
          'The image for the other module transmits almost nothing.', onBoard(t.module, differs.module)) +
        // The firmware takes 5 to 12. The T-Beam's SX1276 has no SF5, and its SF6 is the first
        // generation format, which the ChipSat's SX1262 cannot make: tested 2026-10-17, headerless
        // on both sides, every packet heard and every CRC failed. So neither is offered
        field('Spreading factor', wrapChanged(pick('sf', [7, 8, 9, 10, 11, 12], d.sf, (v) => 'SF' + v), differs.sf),
          (AIR_MS[Number(d.sf)] ? AIR_MS[Number(d.sf)] + ' ' : '') + 'ms on air per packet', onBoard(t.rows[1].onBoard, differs.sf)) +
        field('Transmit power', wrapChanged(pick('dbm', powers, d.dbm, (v) => v + ' dBm'), differs.dbm),
          'The rows the ' + (r.module || 'le').toUpperCase() + ' module has.', onBoard(t.rows[2].onBoard, differs.dbm)) +
      '</div></div>';

    // Ground and air, one table each: how often it logs to the chip and how often it transmits,
    // a row per battery band. The switch and the thresholds are the phase's, for both columns. The
    // board works out the phase from its altitude.
    const numBox = (field, value, label, unit, size) => '<input type="text" inputmode="numeric" data-keep="build:' + field + '" data-build="' + field + '"' +
      ' value="' + esc(String(value)) + '" placeholder="' + unit + '" aria-label="' + label + '" size="' + size + '">';
    const WHAT = ['Switched on, on the bench or the drone, and again once landed.', 'From the climb through the release and the fall.'];
    const LOG_ZERO = ['Log: the gap between records, 0 writes nothing, not even the time anchors.', 'Log: the gap between records, 0 is every reading.'];
    const subs = B.PHASES.map((ph, n) => {
      const follow = !!d['p' + n + 'f'];
      const percent = [0, 1, 2, 3].map((k) => d['p' + n + 'soc' + k]);
      const label = (k) => k === B.FIXED ? (follow ? 'Gauge unread' : 'Always')
        : k === B.BANDS - 1 ? 'Below ' + esc(String(percent[k - 1])) + ' %'
          : (k === 0 ? 'Above ' : 'To ') + numBox('p' + n + 'soc' + k, percent[k], ph.label + ' battery threshold', '%', 3) + ' %';
      const cell = (key, was, zero, what) => {
        const value = d[key];
        return '<td>' + wrapChanged(numBox(key, value, what, 's', 5), was.changed) + ' <span class="u">s</span>' +
          (String(value).trim() !== '' && Number(value) === 0 ? '<span class="zero">' + zero + '</span>' : '') + '</td>';
      };
      const board = (word, c) => '<div><span class="u">' + word + '</span> <span class="val' + (c.changed && c.was ? ' was' : '') + '">' + esc(c.was || '—') + '</span></div>';
      const rows = B.phaseRows(settings, d, n).map((row) => {
        const fixed = row.key === B.FIXED;
        return '<tr><th scope="row">' + label(row.key) + '</th>' +
          cell(fixed ? 'p' + n + 'l' : 'p' + n + 'lms' + row.key, row.log, B.logZero(ph.key), ph.label + ' log gap') +
          cell(fixed ? 'p' + n + 's' : 'p' + n + 'ms' + row.key, row.transmit, 'back to back', ph.label + ' transmit gap') +
          (settings ? '<td class="board">' + board('log', row.log) + board('transmit', row.transmit) + '</td>' : '') + '</tr>';
      }).join('');
      const said = t.rows[3 + n];
      return '<div class="phase' + (differs[ph.key] ? ' changed' : '') + '">' +
        '<div class="phase-name"><span class="dot' + (n ? ' on' : '') + '"></span>' + ph.label + '</div>' +
        '<div class="phase-what">' + WHAT[n] + '</div>' +
        '<label class="check"><input type="checkbox" data-build="p' + n + 'f"' + (follow ? ' checked' : '') + '> Set by battery level (changes both columns)</label>' +
        '<div class="ratewrap"><table class="rates"><thead><tr><th scope="col">Battery</th><th scope="col">Log every</th><th scope="col">Transmit every</th>' +
          (settings ? '<th scope="col">On the board</th>' : '') + '</tr></thead><tbody>' + rows + '</tbody></table></div>' +
        '<p class="hint">' + LOG_ZERO[n] + ' Transmit: the gap after a packet, 0 is back to back.</p>' +
        (said.apart ? '<p class="hint">' + esc(said.apart) + '</p>' : '') +
        (said.older.length ? onBoard(said.older.join('; '), true) : '') + '</div>';
    });
    const phaseCard = '<div class="pane step">' + head(2, 'Phases', 'by altitude') +
      '<div class="phases">' + subs.join('') + '</div>' +
      '<p class="plain">The board picks the phase from altitude: air after a 15 m climb (30 m on GPS if the barometer is dead), ground again after 30 s at rest below half the climb, or after 3 min with the IMU still at any height. Release is not detected, so air covers the climb and the fall. After a reset it logs as air for up to 5 min while transmitting at the ground rate.</p>' +
      '<p class="plain">On the ground the readings mostly stay in the board\'s RAM, and only one per ground log gap reaches flash. So the air\'s battery bands apply only while the board logs as air: the rest of the time the RAM fills at the air\'s Gauge unread gap, and a take-off still writes the 30 s before it.</p></div>';

    // What the write changes, compared with the block read off the board
    const changes = !settings ? [] : [
      ['Radio module', t.module, B.MODULE[r.module || 'le'], differs.module],
      ['Board id', t.rows[0].onBoard, String(d.csid), differs.csid],
      ['Spreading factor', t.rows[1].onBoard, 'SF' + d.sf, differs.sf],
      ['Transmit power', t.rows[2].onBoard, B.dbmWords(Number(d.dbm)), differs.dbm],
    ].filter((c) => c[3]).concat(...B.PHASES.map((ph, n) => B.phaseChanges(settings, d, n)));
    const n = changes.length;
    const diff = !settings ? '' : '<p class="kicker">' + (n ? n + (n === 1 ? ' change' : ' changes') : 'No changes') + ' from the board</p>' +
      changes.map((c) => '<div class="chrow"><span>' + c[0] + '</span><span><span class="was">' + esc(c[1]) + '</span> → <b>' + esc(c[2]) + '</b></span></div>').join('');
    // One press writes. While a job runs the button is off, and the card under the head says
    // what is happening to the board and, after, how it ended.
    const go = '<button data-buildembed="1" class="primary block"' + buildOff + '>' + how.verb + ' ' + esc(nameOf(r)) + '</button>' +
      (stop ? '<p class="note">' + esc(stop) + '</p>' : '');
    const writeCard = '<div class="pane step write">' + head(3, 'Write') + diff +
      '<p class="plain">' + how.column + ' ' + esc(summary) + '. The rates are the Phases tables.</p>' +
      '<p class="plain">' + esc([how.says, how.way === 'build' ? nextImage(r.module) : ''].filter(Boolean).join(' ')) + '</p>' +
      '<label class="check"><input type="checkbox" data-backup="1"' + (state.backup ? ' checked' : '') + '> Save what is on the board first</label>' +
      go + (state.buildError ? '<p class="outcome bad">' + esc(state.buildError) + '</p>' : '') + '</div>';
    return '<div class="cs-steps">' + idCard + phaseCard + writeCard + '</div>';
  }

  function nextImage(module) {
    const f = state.fsw;
    if (!f || !f.tree || !module) return '';
    const p = (f.parts || []).find((x) => x.part === 'flight-' + module);
    if (!p) return '';
    if (p.built) return 'The image is the ' + p.built.words + '.';
    if (p.failed) return 'It did not build from these sources last time; it is tried again.';
    return 'It is built from ' + (f.source && f.source.words ? f.source.words.replace(/^from /, '') : 'the tree') + ' first, about 13 seconds.';
  }

  function jobBlock(job, title) {
    return '<div class="job"><div class="job-head"><b>' + esc(title) + '</b><span class="note">' + D.duration((job.seconds || 0) * 1000) + '</span></div>' +
      steps(job) + track(job.pct) + '</div>';
  }

  // Other images: putting back the saved flight software, a file, the sensor bench. Beside the
  // flight logs and always open (2026-10-31: a Show that opened a pane elsewhere was no use).
  function imagesCard(r) {
    const can = r.onProbe && !busy();
    const dis = can ? '' : ' disabled';
    const job = jobs.chip && jobs.chip.what === 'flash' && chipOwner() === r.uid ? jobs.chip : null;
    const images = imagesOf(r.uid);
    const pick = state.pick && (state.pick.kind === 'saved' || state.pick.kind === 'file') ? state.pick : null;
    const benchPick = state.pick && state.pick.kind === 'bench' && state.pickFor === r.uid ? state.pick : null;
    const top = '<div class="images-top"><h3>Other images</h3><span class="note">Put back, flash a file, sensor bench</span></div>';

    // the app only keeps the newest one
    const newest = images[0];
    const saved = newest
      ? '<div><span class="l">Last saved flight software</span><span class="grow val">' + esc(D.dayTime(new Date(newest.when * 1000), new Date())) + ', ' + D.bytes(newest.bytes) + '</span>' +
          '<button data-putback="' + esc(newest.name) + '"' + dis + '>Put back last saved flight software</button></div>'
      : '<div><span class="l">Last saved flight software</span><span class="note">none saved off this board</span></div>';

    // The sensor bench, its own row: it is not a flight write, and it takes the board's module.
    const benchStop = !r.module ? 'Choose the radio module in Identity and radio first.'
      : !r.onProbe ? 'Put the board on the ST-Link and press Read the board.'
        : busy() ? 'Something else is running on this board.' : '';
    const benchRow = '<div><span class="l">Sensor bench</span><button data-benchwrite="1"' + (benchStop ? ' disabled' : '') + '>Write the sensor bench</button>' +
      '<span class="note">' + esc(benchStop || 'Tests every part and prints it on the console. Not flight software.') + '</span></div>';
    const benchConfirm = !benchPick || job && job.running ? '' : '<div class="confirm"><span class="grow">' +
      (benchStop ? esc(benchStop)
        : 'Writes the <b>' + B.MODULE[r.module] + ' sensor bench</b> over whatever is on <b>' + esc(nameOf(r)) + '</b>, flight software included. ' +
          'It sends as board 254 and has no settings block. ' + (state.backup ? 'The image on it now is saved first.' : 'Nothing is saved first.') +
          ' Write flight software again before it flies.') + '</span>' +
      '<button data-benchgo="1" class="primary"' + (benchStop ? ' disabled' : '') + '>Write the sensor bench</button>' +
      '<button data-benchcancel="1">Cancel</button></div>';

    const confirm = !pick || job && job.running ? '' : '<div class="confirm">' +
      '<span class="grow">Write <b>' + esc(pick.label) + '</b> to <b>' + esc(nameOf(r)) + '</b>? ' +
        (state.backup ? 'The image on it now is saved first.' : 'The image on it now is not saved.') +
        (r.onProbe ? '' : ' Put the board on the ST-Link and press Read the board first.') + '</span>' +
      (state.canForce ? '' : '<button data-flashgo="1" class="primary"' + dis + '>Overwrite the board</button>') +
      '<button data-flashcancel="1">Cancel</button></div>';
    const refused = !state.refused ? '' : '<p class="outcome bad inpane">Not written: ' + esc(state.refused) + '</p>' +
      (state.canForce && pick ? '<div class="confirm"><span class="grow">That image came off a different board.</span>' +
        '<button data-flashforce="1"' + dis + '>Write it to this board anyway</button></div>' : '');

    return '<div class="pane images">' + top + '<div class="rows kvrows">' +
      saved +
      '<div><span class="l">From a file</span><label class="button' + (can ? '' : ' disabled') + '">Flash a file…<input type="file" accept=".bin" data-file="1" hidden' + dis + '></label>' +
        '<span class="note">A file\'s radio settings are whatever it was built with.</span></div>' +
      benchRow +
      '<div><label class="check"><input type="checkbox" data-backup="1"' + (state.backup ? ' checked' : '') + '> Save what is on the board first</label></div>' +
      '</div>' + confirm + benchConfirm + refused + '</div>';
  }

  // ---------- what is happening to the board ----------
  // What a finished pull left behind, in parts the box and the card both draw: a state mark,
  // label and value rows, what the operator must act on, and what only the card shows (which
  // dump firmware read it, what was on the board, the buttons). An older build of the app sends
  // only the log's name, so each row is there when its value is.
  // boardCsid is the id the board's own settings gave when it was read, if they gave one.
  const COUNT = new Intl.NumberFormat('en-US');
  function pullParts(s, boardCsid) {
    const rep = s.report || {};
    const n = (v) => COUNT.format(Number(v));
    const rows = [];
    if (Number.isFinite(rep.read)) rows.push(['Records', n(rep.read) + (Number.isFinite(rep.found) ? ' of ' + n(rep.found) : '')]);
    else if (s.log) rows.push(['Log', esc(s.log)]);
    if (Array.isArray(rep.boots)) {
      rows.push(['Boots', n(rep.boots.length)]);
      const gps = rep.boots.filter((b) => b && b.timeSource === 'anchor').length;
      rows.push(['GPS time', gps ? n(gps) + (gps === 1 ? ' boot' : ' boots') : 'none']);
    }
    const idw = D.boardIdWords(rep.csids);
    if (idw) rows.push([idw.startsWith('board ids') ? 'Board ids' : 'Board id', esc(idw.replace(/^board ids? /, ''))]);
    if (s.dir) rows.push(['Folder', '<span class="path">' + esc(s.dir) + '</span>']);
    if (s.restored) rows.push(['Flight software', 'put back']);
    // Said in words as well as by the coloured edge: a log with holes in it must not
    // read as a whole one to someone who cannot tell amber from grey. The app calls a
    // pull complete when every record the board counted as good was read, so a record
    // the board itself wrote off fails its check here without making the pull incomplete.
    const short = [];
    if (rep.failedCheck > 0) short.push(n(rep.failedCheck) + ' failed the check');
    if (rep.missing > 0) short.push(n(rep.missing) + ' missing');
    if (rep.complete === false && !short.length) {
      short.push(Number.isFinite(rep.found) ? 'fewer records were read than the board counted as good'
        : 'the board did not say how many records it holds');
    }
    const status = rep.complete === true && !short.length ? mark('good', 'complete') : rep.complete === false ? mark('warn', 'incomplete') : '';
    const alerts = [];
    const wrong = D.idMismatch(rep.csids, boardCsid);
    if (wrong) alerts.push(['bad', esc(wrong)]);
    if (short.length) alerts.push(['warn', (rep.complete === false ? 'Incomplete' : 'Warning') + ': ' + esc(short.join(', ')) + '.']);
    // A pull that was told not to put the flight software back ends well and still
    // leaves a board that will not fly.
    if (!s.restored) alerts.push(['bad', 'The board is still running the dump firmware, not flight software.']);
    const fwr = rep.firmware || {};
    const cardRows = [];
    if (fwr.dump && fwr.dump.words) cardRows.push(['Read with', esc('the ' + fwr.dump.words)]);
    if (fwr.onBoard && fwr.onBoard.built && fwr.onBoard.built.words) cardRows.push(['On the board was', esc('the ' + fwr.onBoard.built.words)]);
    const acts = '<div class="acts">' +
      '<button data-openlog="' + esc(s.log) + '" class="primary">Open</button>' +
      '<a class="button" download="' + esc(flatName(s.log)) + '" href="' + logUrl(s.log) + '">Save log</a>' +
      '<button data-csv="' + esc(s.log) + '">Save CSV</button>' +
      (s.raw ? '<a class="button" download="' + esc(flatName(s.raw)) + '" href="' + logUrl(s.raw) + '">Save raw</a>' : '') +
      '</div>';
    return { status, rows, alerts, cardRows, acts };
  }
  const kvHtml = (rows) => (rows.length ? '<dl class="kv">' + rows.map((x) => '<dt>' + x[0] + '</dt><dd>' + x[1] + '</dd>').join('') + '</dl>' : '');
  const alertHtml = (alerts) => alerts.map((x) => '<p class="outcome' + (x[0] ? ' ' + x[0] : '') + '">' + x[1] + '</p>').join('');

  // The job on this board: the pull, clear or flash running now, else the last one to end.
  // A pull nobody can put a board to goes on whichever sheet is up rather than on none.
  function jobOn(r) {
    const pull = jobs.pull && (pullOwner() === r.uid || !pullOwner() || r.id === ANY) ? jobs.pull : null;
    const chip = jobs.chip && jobs.chip.what === 'flash' && chipOwner() === r.uid ? jobs.chip : null;
    if (pull && pull.running) return { kind: 'pull', j: pull };
    if (chip && chip.running) return { kind: 'chip', j: chip };
    if (pull && chip) return store.get('dg-last-job') === 'chip' ? { kind: 'chip', j: chip } : { kind: 'pull', j: pull };
    return pull ? { kind: 'pull', j: pull } : chip ? { kind: 'chip', j: chip } : null;
  }
  // What tells one finished job from the next, so a closed result stays closed across
  // repaints and reloads and the next job opens the card again.
  const jobKey = (kind, j) => [kind, j.log, j.saved, j.error, j.seconds, j.pct, j.wipe ? 'wipe' : ''].map((v) => String(v == null ? '' : v)).join('|');

  // How a job ended: its title, a state mark, label and value rows, what must be acted on,
  // and (card only) more rows, the steps and the buttons. null when it has nothing to say yet
  // (the app's first answer, before anything was pulled). What happened last stays on screen:
  // a pull that failed after reflashing leaves a board running the wrong firmware, and that
  // must not vanish with a toast.
  function ended(kind, j, r) {
    const out = { title: '', status: '', rows: [], alerts: [], cardRows: [], more: '', acts: '' };
    const saved = (p) => (p ? [['Saved copy', '<span class="path">' + esc(p) + '</span>']] : []);
    if (kind === 'chip') {
      if (j.error) return Object.assign(out, { title: 'Flash failed', rows: saved(j.saved), alerts: [['bad', esc(j.error)]] });
      if ((j.steps || []).length) return Object.assign(out, { title: 'Flashed and checked', status: mark('good', 'checked'), rows: saved(j.saved) });
      return null;
    }
    const csid = r && r.onProbe && state.read && state.read.settings ? state.read.settings.csid : null;
    const what = j.wipe ? 'Clear' : 'Pull';
    const left = (p) => (p ? '; the image that was on it is saved as <span class="path">' + esc(p) + '</span>' : '');
    if (j.error && j.error.startsWith('stopped by the operator')) {
      // not a failure: what arrived before the stop is kept
      const dump = !j.restored && (j.steps || []).some((x) => x.startsWith('writing the dump'));
      const parts = j.log ? pullParts(j, csid) : null;
      return Object.assign(out, parts || {}, { title: what + ' stopped', status: '', more: steps(j),
        alerts: [['', esc((j.wipe ? 'The chip was not erased: ' : '') + j.error)]]
          .concat(dump ? [['bad', 'The board is left on the dump firmware' + left(j.savedFsw) + '.']] : [], parts ? parts.alerts : []) });
    }
    if (j.error) {
      return Object.assign(out, { title: what + ' failed', more: steps(j), alerts: [['bad', esc(j.error)]]
        .concat(j.keepFsw && !j.restored ? [['bad', 'The flight software was not put back. The board is still running the dump firmware' + left(j.savedFsw) + '.']] : []) });
    }
    if (j.wipe && j.pct === 100) {
      return Object.assign(out, { title: 'Flash cleared', status: j.restored ? mark('good', 'flight software back') : '',
        rows: [['Flight software', j.restored ? 'back on the board, settings as they were' : 'not on the board']],
        alerts: j.restored ? [] : [['bad', !j.keepFsw ? 'The board is on the dump firmware: write a flight image next.'
          : 'The board was already on the dump firmware and still is: write the flight image and its settings before it flies.']] });
    }
    if (j.log) return Object.assign(out, pullParts(j, csid), { title: 'Flight log pulled' });
    return null;
  }

  // One card under the head for the job on this board (2026-10-31: the user asked for one
  // place to read what is happening, not a checklist on each pane). While it runs: its
  // steps, the bar, where it saves and Stop. After: how it ended, with Close.
  function nowCard(r) {
    const on = jobOn(r);
    if (!on) return '';
    const { kind, j } = on;
    if (j.running) {
      const title = kind === 'pull'
        ? (j.wipe ? 'Clearing the flash on ' : 'Pulling the flight log from ') + nameOf(r) + (j.board ? ', console on ' + j.board : '')
        : 'Flashing ' + nameOf(r);
      // a clear saves nothing
      const to = kind === 'pull' && j.dir && !j.wipe ? (j.raw ? j.dir + '/dump.txt' : j.dir) : '';
      const stop = kind !== 'pull' ? '' : '<div class="acts"><button data-pullstop="1"' + (j.stopping ? ' disabled>Stopping…' : '>Stop') + '</button>' +
        '<span class="note">' + (j.stopping ? 'Stopping after the step in hand.' : 'Keeps what has arrived.') +
          (j.keepFsw ? ' The flight software goes back on.' : ' The board stays on the dump firmware: its flight software is not being kept.') + '</span></div>';
      return '<div class="pane now">' + jobBlock(j, title) +
        (to ? '<p class="note">Saving to <span class="path">' + esc(to) + '</span></p>' : '') + stop +
        '<p class="note">Leave the board plugged in.</p></div>';
    }
    const end = ended(kind, j, r);
    if (!end) return '';
    const key = jobKey(kind, j);
    if (store.get('dg-closed-job') === key) return '';
    const at = state.endedAt[kind];
    return '<div class="pane now"><h3>' + esc(end.title) + end.status + (at ? '<span class="note">' + clock(at) + '</span>' : '') +
        '<button class="ghost close" data-closejob="' + esc(key) + '" title="Take this result off the sheet">Close</button></h3>' +
      kvHtml(end.rows.concat(end.cardRows)) + end.more + alertHtml(end.alerts) + end.acts + '</div>';
  }

  // The console port for this board, as last chosen for it, while that port is still here.
  // The drone's flight controller shows up as a serial port too (ArduPilot, USB 1209:5741), and
  // three pulls were pointed at it on one day. It is never a console: not offered, not kept.
  const isFlightController = (b) => b.usb === '1209:5741';

  // Else the port that last printed the log format (benchpane.js hears every port), else any free one.
  function consoleFor(r) {
    const usable = (hub.boards || []).filter((b) => !isFlightController(b));
    const ports = usable.map((b) => b.port);
    const kept = store.get('dg-console-' + (r.uid || ANY));
    if (kept && ports.includes(kept)) return kept;
    const talking = A.benchPane ? A.benchPane.talking(usable) : '';
    if (talking) return talking;
    const free = usable.find((b) => b.state !== 'receiver');
    return free ? free.port : '';
  }

  // One chooser per sheet: in Sensor bench on a bench board, in Flight log otherwise.
  function consoleChooser(r, port) {
    const boards = hub.boards || [];
    if (!boards.length) return '<span class="note">No serial port. Plug in the board\'s console.</span>';
    return '<select data-console="' + esc(r.uid || ANY) + '" aria-label="Serial port of this board\'s console">' +
      boards.map((b) => '<option value="' + esc(b.port) + '"' + (b.port === port ? ' selected' : '') + (isFlightController(b) ? ' disabled' : '') + '>' +
        esc((b.name || b.port) + (isFlightController(b) ? ', the drone\'s flight controller' : b.state === 'receiver' ? ', being read as a receiver' : '')) + '</option>').join('') + '</select>';
  }

  // What the app last found on the board decides the layout. The console only agrees or not.
  const isBench = (r) => r.firmware === 'bench';
  function consoleDisagrees(r) {
    if (!A.benchPane || !r.uid) return '';
    const s = A.benchPane.sessionOn(consoleFor(r));
    if (!s || s.bench === null) return '';
    const bench = isBench(r);
    if (s.bench === bench) return '';
    const fw = B.firmwareWords(r.fw, r.module).value || r.firmware || 'not known';
    const probe = r.onProbe ? r.probe : state.probes[0];
    return '<p class="outcome bad inpane">Last read: ' + esc(bench ? 'sensor bench' : fw.charAt(0).toLowerCase() + fw.slice(1)) + '. ' +
      (bench ? 'The board did not say test=bench when it started on the console.' : 'The console says test=bench.') +
      (probe ? ' <button data-read="' + esc(probe.serial) + '"' + (busy() ? ' disabled' : '') + '>Read the board</button>' : '') + '</p>';
  }

  // The sensor bench card. Its parts, starts and console are benchpane.js's, painted into
  // #cs-bench on their own timer so the rest of the sheet is not redrawn twice a second.
  function benchCard(r) {
    const port = consoleFor(r);
    const b = (hub.boards || []).find((x) => x.port === port);
    const reading = !!b && b.state === 'receiver';
    const s = A.benchPane ? A.benchPane.sessionOn(port) : null;
    const source = r.onProbe && !state.stale ? 'running it, read off the board now'
      : s && s.bench ? 'the console says test=bench' : 'on it when last read';
    const { stop, buildOff, summary } = flightWrite(r);
    return '<div class="pane bench"><h3>Sensor bench<span class="note">' + esc(source) + '</span></h3>' + consoleDisagrees(r) +
      '<div class="rows kvrows">' +
        '<div><span class="l">Console port</span>' + consoleChooser(r, port) +
          (port && !reading ? '<button data-listen="' + esc(port) + '">Read this port</button>' : '') + (reading ? mark('good', 'being read') : '') + '</div>' +
        '<div><span class="l">On the console</span><button data-radio="' + esc(port) + '"' + (reading ? '' : ' disabled') + '>Radio on/off</button>' +
          '<button data-clear="1">Clear this record</button>' + (reading ? '' : '<span class="note">Radio on/off needs the port read.</span>') + '</div>' +
        '<div><span class="l">Flight software</span><button data-buildembed="1"' + buildOff + '>Write flight software</button>' +
          '<span class="note">' + esc(stop || summary + '. Change it in Identity and radio below.') + '</span></div>' +
      '</div>' +
      (state.buildError ? '<p class="outcome bad inpane">' + esc(state.buildError) + '</p>' : '') +
      '<div id="cs-bench" class="benchlive"></div></div>';
  }

  // Panes a bench board has no use for until flight software goes back on. Open or shut is
  // kept here, since a redraw would shut them.
  function fold(key, r, summary, inner) {
    return '<details class="fold" data-fold="' + key + '"' + (state.open[key + ':' + r.uid] ? ' open' : '') + '><summary>' + summary + '</summary>' + inner + '</details>';
  }

  // The flight logs card, top down: what has been pulled, the console the pull listens on,
  // whether the flight software goes back on, then the pull and the clear with what each does
  // beside it. What a pull or clear is doing, or left behind, is the card under the head.
  function logsCard(r) {
    const boards = hub.boards || [];
    const port = consoleFor(r);
    const ready = (r.onProbe || r.id === ANY) && !busy() && !!port;
    const pulls = r.uid ? pullsOf(r.uid) : [];
    const all = pulls.length > 0 && !!state.open['pulls:' + r.uid];
    const at = (l) => esc(D.dayTime(new Date(l.when * 1000), new Date()));
    const summary = !r.uid ? ''
      : pulls.length ? '<b>' + pulls.length + (pulls.length === 1 ? ' pull' : ' pulls') + '</b> · latest <b>' + at(pulls[0]) + '</b>, ' + D.bytes(pulls[0].bytes)
        : mark('warn', 'never pulled');
    const acts = !pulls.length ? '' : '<span class="acts"><button data-openlog="' + esc(pulls[0].name) + '">Open latest</button>' +
      '<button class="expander" data-allpulls="1" aria-expanded="' + all + '">All ' + pulls.length + '</button></span>';
    const list = !all ? '' : '<div class="rows pulls"><div><span class="l">Log pulls</span><span class="grow note">' + pulls.length + (pulls.length === 1 ? ' pull' : ' pulls') + '</span>' +
        '<button data-todata="1">Show in Data</button></div>' +
      pulls.map((l) => '<div><span class="grow val">' + at(l) + ', ' + D.bytes(l.bytes) + '</span>' +
        '<button data-openlog="' + esc(l.name) + '">Open</button>' +
        '<a class="button" download="' + esc(flatName(l.name)) + '" href="' + logUrl(l.name) + '">Save log</a>' +
        '<button data-csv="' + esc(l.name) + '">Save CSV</button></div>').join('') + '</div>';

    const bench = isBench(r);
    const chooser = bench ? '<span class="val">' + esc(port || 'none') + '</span><span class="note">chosen in Sensor bench</span>'
      : consoleChooser(r, port) + (boards.length ? '<span class="note">The board\'s serial console.</span>' : '');
    const offProbe = !(r.onProbe || r.id === ANY);
    const pullNote = offProbe ? 'Put the board on the ST-Link and press Read the board first.'
      : state.keepFsw ? 'Flashes the board twice. Takes minutes.' : 'Flashes the dump firmware. Takes minutes.';
    const clearNote = state.keepFsw ? 'Erases the log on the chip and nothing else. The flight software is read off and put back, settings untouched.'
      : 'Erases the log on the chip and leaves the board on the dump firmware.';
    // A pull whose console check failed can go anyway: a board that prints nothing can still be pulled
    const s = jobs.pull;
    const mine = s && (pullOwner() === r.uid || !pullOwner() || r.id === ANY);
    const unproven = mine && !s.running && s.error && s.error.includes("is not this board's console");
    const anyway = !unproven ? '' : '<button data-anyconsole="1"' + (ready ? '' : ' disabled') + '>' + (s.wipe ? 'Clear anyway' : 'Pull anyway') + '</button>' +
      '<span class="note">Skips the check, on ' + esc(port) + '.</span>';
    // a clear keeps no copy, so say when it was last pulled
    const wiping = state.pick && state.pick.kind === 'wipe' && state.pickFor === r.uid && !busy()
      ? '<div class="confirm"><span class="grow">' + (pulls.length
        ? 'Erases the flight log on <b>' + esc(nameOf(r)) + '</b> for good. Last pulled ' +
          at(pulls[0]) + ' (' + daysAgo(new Date(pulls[0].when * 1000), new Date()) +
          '); anything it logged since is lost. Nothing is saved first.'
        : mark('warn', 'never pulled') + ' <b>' + esc(nameOf(r)) + '</b> has never been pulled on this computer. ' +
          'Clearing erases its flight log for good, and nothing is saved first.') + '</span>' +
        '<button data-wipego="1" class="primary">Clear the flash</button><button data-wipecancel="1">Cancel</button></div>'
      : '';

    return '<div class="pane logs"><div class="logs-top"><h3>Flight logs</h3><span class="sum">' + summary + '</span>' + acts + '</div>' + list +
      '<div class="rows kvrows">' +
        '<div><span class="l">Console port</span>' + chooser + '</div>' +
        '<div><span class="l">Flight software</span><label class="check"><input type="checkbox" data-keepfsw="1"' + (state.keepFsw ? ' checked' : '') +
            '> Save the flight software that is on it and put it back</label>' +
          (state.keepFsw ? '' : mark('warn', 'the board will be left on the dump firmware')) +
          // the pull saves whatever is on it, here the sensor bench, not flight software
          (bench && state.keepFsw ? '<span class="note">On this board that is the sensor bench.</span>' : '') + '</div>' +
        '<div><span class="l">Pull</span><button data-pull="1" class="primary"' + (ready ? '' : ' disabled') + '>Pull the flight log</button>' +
          '<span class="note">' + pullNote + '</span>' + anyway + '</div>' +
        '<div><span class="l">Clear</span><button data-wipe="1"' + (ready && r.uid ? '' : ' disabled') + '>Clear flash</button>' +
          '<span class="note">' + clearNote + '</span></div>' +
      '</div>' + wiping + '</div>';
  }

  // Clear the chip: the same job as a pull, told to erase instead of read.
  async function wipe(anyConsole) {
    const r = rows().find((x) => x.id === state.sel);
    state.pick = null;
    if (!r || !r.uid || busy()) return;
    const port = consoleFor(r);
    if (!port) return;
    if (state.peek) { await state.peek; if (busy()) return; }
    try {
      const more = { uid: r.uid, wipe: 1 };
      if (anyConsole) more.anyConsole = 1;
      if (r.probe && r.probe.serial) more.probe = r.probe.serial;
      await hub.pull(port, state.keepFsw, more);
      store.set('dg-pull-board', r.uid);
      toast('Clearing the flash on ' + nameOf(r) + (state.keepFsw ? '' : ', not keeping the flight software'));
    } catch (e) {
      toast('Could not start the clear: ' + (e.message || e));
    }
    await A.shell.pollJobs();
    render();
  }

  function sheetHtml() {
    const r = rows().find((x) => x.id === state.sel);
    if (!r) {
      // An image chosen on the Data screen waits here for a board, and says so.
      const chosen = state.pick && state.pick.kind === 'saved'
        ? '<div class="confirm"><span class="grow">Chosen to write: ' + esc(state.pick.label) + '. Read the board first.</span>' +
          '<button data-flashcancel="1">Cancel</button></div>'
        : '';
      return '<div class="pane"><p class="empty">Clip a ChipSat onto the ST-Link and plug the ST-Link in.</p>' + chosen + '</div>';
    }
    if (r.unread) return headPane(r);
    if (r.id === ANY) return headPane(r) + nowCard(r) + '<div class="cs-lower">' + logsCard(r) + '</div>';
    if (isBench(r)) {
      return headPane(r) + nowCard(r) + benchCard(r) + imagesCard(r) +
        fold('radio', r, '<b>Identity and radio, phases</b><span class="note">Not on the board: the sensor bench has no settings block. ' +
          'These are what the next flight write uses.</span>', stepsHtml(r)) +
        fold('log', r, '<b>Flight logs</b><span class="note">Pull or clear the chip.</span>', logsCard(r));
    }
    return headPane(r) + nowCard(r) + stepsHtml(r) + '<div class="cs-lower">' + logsCard(r) + imagesCard(r) + '</div>';
  }

  function render() {
    const all = rows();
    if (state.loaded && !all.some((r) => r.id === state.sel)) {
      const first = all.find((r) => r.onProbe) || all.find((r) => r.unread) || all[0];
      state.sel = first ? first.id : null;
    }
    const btn = B.readButton(state.probes, busy());
    $('cs-look').textContent = btn.label;
    $('cs-look').title = btn.title;
    $('cs-look').disabled = btn.disabled;
    paint($('cs-list'), listHtml());
    paint($('cs-sheet'), sheetHtml());
    const r = all.find((x) => x.id === state.sel);
    if (A.benchPane) A.benchPane.show(r && r.uid && !r.unread && isBench(r) ? r : null, r ? consoleFor(r) : '');
  }

  // ---------- doing things ----------
  async function flash(force) {
    const r = rows().find((x) => x.id === state.sel);
    const p = state.pick;
    if (!r || !r.onProbe || !p || busy()) return;
    state.refused = '';
    if (state.peek) await state.peek;
    try {
      await hub.chipFlash({
        uid: r.uid, backup: state.backup, force,
        saved: p.kind === 'saved' ? p.saved : null,
        name: p.name, bytes: p.bytes,
      });
      state.pick = null; state.canForce = false;
      toast('Flashing ' + nameOf(r));
    } catch (e) {
      state.refused = e.missing ? CANNOT : (e.message || String(e));
      state.canForce = !force && !e.missing && B.isOtherBoardRefusal(state.refused);
      if (!state.canForce) state.pick = null;
    }
    await A.shell.pollJobs();
    render();
  }

  // The sensor bench goes by the board's module. A refusal stays in the Firmware pane.
  async function writeBench() {
    const r = rows().find((x) => x.id === state.sel);
    state.pick = null;
    if (!r || !r.uid || !r.module || !r.onProbe || busy()) { render(); return; }
    state.refused = ''; state.canForce = false;
    if (state.peek) await state.peek;
    try {
      await hub.chipFlash({ uid: r.uid, embedded: 'bench', module: r.module, backup: state.backup });
      toast('Writing the sensor bench on ' + nameOf(r));
    } catch (e) {
      state.refused = e.missing ? CANNOT : (e.message || String(e));
    }
    await A.shell.pollJobs();
    render();
  }

  async function forget() {
    const r = rows().find((x) => x.id === state.sel);
    if (!r || !r.uid || busy()) return;
    state.pick = null;
    try {
      await hub.chipForget(r.uid);
      // Still under the clip, it is a board that has not been read. It is not read again
      // unasked: back in the list two seconds after being forgotten is not forgotten.
      if (r.onProbe && r.probe) {
        const serial = r.probe.serial;
        forgetRead();
        state.tried[serial] = r.uid;
      }
      state.chips = await hub.chips().catch(() => state.chips);
      toast('Forgot ' + nameOf(r));
    } catch (e) {
      toast('Could not forget it: ' + (e.missing ? CANNOT : (e.message || e)));
    }
    render();
  }

async function pull(anyConsole) {
    const r = rows().find((x) => x.id === state.sel);
    if (!r || busy()) return;
    const port = consoleFor(r);
    if (!port) return;
    if (state.peek) { await state.peek; if (busy()) return; }
    try {
      const more = anyConsole ? { anyConsole: 1 } : {};
      if (r.uid) more.uid = r.uid;
      if (r.probe && r.probe.serial) more.probe = r.probe.serial;
      await hub.pull(port, state.keepFsw, more);
      store.set('dg-pull-board', r.uid || '');
      toast('Pulling the flight log from ' + nameOf(r) + (state.keepFsw ? '' : ', not keeping the flight software'));
    } catch (e) {
      toast('Could not start the pull: ' + (e.message || e));
    }
    await A.shell.pollJobs();
    render();
  }

  // A log as a table, written by the app from the file it wrote.
  async function saveCsv(name) {
    try {
      const r = await fetch(logUrl(name).replace(/^api\/log\//, 'api/csv/'));
      if (!r.ok) throw new Error('HTTP ' + r.status);
      const csv = await r.text();
      const count = csv.split('\n').length - 2;   // less the header and the last newline
      if (count < 1) { toast('Nothing to save: no packet in this log passes its CRC.'); return; }
      const out = flatName(name).replace(/\.[^.]*$/, '') + '.csv';
      A.ui.download(new Blob([csv], { type: 'text/csv' }), out);
      toast('Saved ' + COUNT.format(count) + ' rows to ' + out);
    } catch (e) {
      toast('Could not get ' + name + ': ' + (e.message || e));
    }
  }
  A.shell.saveCsv = saveCsv;

  const root = $('screen-chipsats');
  // The probe of the row that is picked, when that row has one.
  function pickedProbe() {
    const r = rows().find((x) => x.id === state.sel);
    return r && r.probe ? r.probe.serial : null;
  }

  // The button at the head of the list: plug a board in, press it, see that board. It
  // only looks when it said it would look, since a read can reset the board.
  $('cs-look').addEventListener('click', async () => {
    if (busy()) return;
    const reads = state.probes.length > 0;
    const picked = pickedProbe();
    await load();
    if (state.missing) { toast(CANNOT); return; }
    const to = B.probeToRead(state.probes, picked);
    if (!reads) {
      const p = state.probes[0];
      toast(p ? 'Found ' + p.kind + ' ' + p.serial : to.why);
      return;
    }
    if (!to.serial) { toast(to.why); return; }
    read(to.serial);
  });

  root.addEventListener('click', async (e) => {
    const item = e.target.closest('[data-pick]');
    if (item) {
      state.sel = item.getAttribute('data-pick'); state.readError = '';
      if (state.pick && state.pick.kind === 'forget') state.pick = null;
      render();
      return;
    }
    const at = (name) => e.target.getAttribute(name);
    const r = rows().find((x) => x.id === state.sel);

    if (at('data-read')) { read(at('data-read')); return; }
    // All n opens in place
    if (at('data-allpulls')) {
      const key = 'pulls:' + (r ? r.uid : '');
      state.open[key] = !state.open[key];
      render();
      return;
    }
    if (at('data-module') && r && r.uid) {
      const m = at('data-module');
      try {
        await hub.chipSetModule(r.uid, m);
        state.chips = await hub.chips().catch(() => state.chips);
        toast(nameOf(r) + ' is ' + B.MODULE[m]);
      } catch (err) {
        toast('Could not save the module: ' + (err.missing ? CANNOT : (err.message || err)));
      }
      render();
      return;
    }
    if (at('data-forget') && r && r.uid) {
      state.pickFor = r.uid;
      state.pick = { kind: 'forget', label: nameOf(r) };
    } else if (at('data-forgetgo')) {
      forget();
      return;
    } else if (at('data-forgetcancel')) {
      state.pick = null;
    } else if (at('data-buildembed')) {
      buildAndFlash();
      return;
    } else if (at('data-closejob')) {
      store.set('dg-closed-job', at('data-closejob'));
    } else if (at('data-benchwrite') && r && r.uid) {
      state.pickFor = r.uid;
      state.pick = { kind: 'bench', label: 'the sensor bench' };
    } else if (at('data-benchgo')) {
      writeBench();
      return;
    } else if (at('data-benchcancel')) {
      state.pick = null;
    } else if (at('data-putback')) {
      const l = state.logs.find((x) => x.name === at('data-putback'));
      state.pickFor = r.uid;
      state.pick = { kind: 'saved', saved: l.name, label: 'the image saved off it on ' + D.dayTime(new Date(l.when * 1000), new Date()) };
    } else if (at('data-flashcancel')) {
      state.pick = null;
    } else if (at('data-flashgo')) { flash(false); return; }
    else if (at('data-flashforce')) { flash(true); return; }
    else if (at('data-pull')) { pull(); return; }
    else if (at('data-anyconsole')) {
      const s = jobs.pull;
      if (s && s.wipe) wipe(true); else pull(true);
      return;
    }
    else if (at('data-pullstop')) {
      try {
        await hub.pullStop();
      } catch (err) {
        toast('Could not stop: ' + (err.message || err));
      }
      await A.shell.pollJobs();
      render();
      return;
    }
    else if (at('data-wipe') && r && r.uid) {
      state.pickFor = r.uid;
      state.pick = { kind: 'wipe', label: 'the flash' };
    } else if (at('data-wipego')) { wipe(); return; }
    else if (at('data-wipecancel')) { state.pick = null; }
    else if (at('data-openlog')) {
      const s = jobs.pull;
      A.openServerLog(at('data-openlog'), s && s.log === at('data-openlog') ? { raw: s.raw || null, dir: s.dir || null } : null);
      return;
    } else if (at('data-csv')) { saveCsv(at('data-csv')); return; }
    else if (at('data-todata')) { A.shell.show('data'); return; }
    else return;
    state.refused = ''; state.canForce = false;
    render();
  });

  // toggle does not bubble, so it is caught on the way down
  root.addEventListener('toggle', (e) => {
    const key = e.target.getAttribute && e.target.getAttribute('data-fold');
    const r = rows().find((x) => x.id === state.sel);
    if (!key || !r || !!state.open[key + ':' + r.uid] === e.target.open) return;
    state.open[key + ':' + r.uid] = e.target.open;
    render();
  }, true);

  root.addEventListener('input', (e) => {
    const built = e.target.getAttribute && e.target.getAttribute('data-build');
    if (built && e.target.tagName === 'INPUT' && state.build) {
      state.build[built] = e.target.value;
      state.buildError = '';
      render();
      return;
    }
  });

  root.addEventListener('change', async (e) => {
    const t = e.target;
    if (t.getAttribute('data-build') && t.tagName === 'SELECT' && state.build) {
      state.build[t.getAttribute('data-build')] = t.value === '' ? '' : Number(t.value);
      state.buildError = '';
      t.blur();
      render();
      return;
    }
    if (t.getAttribute('data-build') && t.type === 'checkbox' && state.build) {
      const key = t.getAttribute('data-build');
      state.build[key] = !!t.checked;
      // Switching the battery on must not change logging until a box is edited, so the log
      // bands start at the gap in use. Transmit keeps its table: those numbers are in view
      const phase = t.checked && key.match(/^p(\d)f$/);
      if (phase) {
        for (let k = 0; k < B.BANDS; k++) state.build['p' + phase[1] + 'lms' + k] = state.build['p' + phase[1] + 'l'];
      }
      state.buildError = '';
      render();
      return;
    }
    if (t.getAttribute('data-name')) {
      try {
        const clean = await hub.chipSetName(t.getAttribute('data-name'), t.value);
        state.chips = await hub.chips().catch(() => state.chips);
        toast(clean ? 'Called it ' + clean : 'Name cleared');
      } catch (err) {
        toast('Could not name it: ' + (err.missing ? CANNOT : (err.message || err)));
      }
      t.blur();
    } else if (t.getAttribute('data-console')) {
      store.set('dg-console-' + t.getAttribute('data-console'), t.value);
      toast('Console port ' + t.options[t.selectedIndex].text.split(',')[0]);
      t.blur();
    } else if (t.getAttribute('data-keepfsw')) {
      state.keepFsw = t.checked;
    } else if (t.getAttribute('data-backup')) {
      state.backup = t.checked;
    } else if (t.getAttribute('data-file')) {
      const f = t.files && t.files[0];
      if (!f) return;
      const on = rows().find((x) => x.id === state.sel);
      state.pickFor = on ? on.uid : null;
      state.pick = { kind: 'file', label: f.name, name: f.name, bytes: await f.arrayBuffer() };
      state.refused = ''; state.canForce = false;
    } else {
      return;
    }
    render();
  });

  // ---------- jobs ----------
  // A toast is gone before anyone who stepped away is back, so how a flash, a pull or a clear
  // ended waits in a box to be dismissed, whichever screen is up.
  function jobDone(kind, j) {
    const r = rows().find((x) => x.uid && x.uid === j.uid) || null;
    const end = ended(kind, j, r);
    if (!end) return;
    // The board by its name; a board the list does not know, by the id in the pull, else its uid
    const ids = j.report && Array.isArray(j.report.csids) ? j.report.csids : [];
    const who = r ? nameOf(r) : ids.length === 1 ? 'Board ' + ids[0] : j.uid ? B.chipName({ uid: j.uid }) : 'The board';
    const when = new Date().toTimeString().slice(0, 8);
    paint($('flash-done-body'), '<h2 id="flash-done-title"><span>' + esc(who) + ' · ' + esc(end.title) + '</span>' + end.status +
      '<span class="note">' + when + '</span></h2>' + kvHtml(end.rows) + alertHtml(end.alerts));
    $('flash-done').hidden = false;
    $('flash-done-ok').focus();
  }
  $('flash-done-ok').addEventListener('click', () => { $('flash-done').hidden = true; });

  A.shell.onJobs((over) => {
    for (const kind of over) {
      const j = jobs[kind];
      if (kind !== 'pull' && j.what !== 'flash') continue;
      // the card under the head shows the one that ended last, also after a reload
      state.endedAt[kind] = Date.now();
      store.set('dg-last-job', kind);
      jobDone(kind, j);
      if (kind === 'pull') {
        state.stale = 'pull';
      } else {
        if (!j.error && state.sent && state.sent.uid === j.uid) remember(j.uid, state.sent.draft);
        state.sent = null;
        // The job reads the board again once it has written it, and that read is the
        // sheet's, also when its settings came back different. Without it nobody knows
        // what is on the board, so the sheet says to read. A block read gives the draft
        // its numbers; a bench or dump image has none, so the draft keeps the last ones.
        if (j.read && j.read.uid) {
          state.read = j.read;
          state.readAt = Date.now();
          state.stale = '';
          if (j.read.settings) state.build = null;
        } else {
          state.stale = 'flash';
        }
      }
    }
    if (over.length) load();
    else if (A.shell.current() === 'chipsats') render();
  });

  // The board the bar's chip is about: the job running now, or the one that finished
  // while another screen was up.
  const jobUid = () => {
    if (jobs.pull && jobs.pull.running) return pullOwner();
    if (jobs.chip && jobs.chip.running) return chipOwner();
    if (jobs.unseen) return jobs.unseen.kind === 'pull' ? pullOwner() : chipOwner();
    return null;
  };
  A.shell.boardName = () => { const r = rows().find((x) => x.uid && x.uid === jobUid()); return r ? nameOf(r) : ''; };
  A.shell.showJobBoard = () => { const uid = jobUid(); if (uid) { state.sel = uid; render(); } };

  // Data hands an image over to be written: it is chosen here, on its own board's sheet
  // when the app remembers that board, and still has to be confirmed.
  A.shell.writeImage = (entry) => {
    state.pickFor = null;
    state.pick = { kind: 'saved', saved: entry.name, label: 'the image saved on ' + D.dayTime(new Date(entry.when * 1000), new Date()) +
      (entry.from ? ' off ' + entry.from : '') };
    state.refused = ''; state.canForce = false;
    const own = state.chips.find((c) => B.sameBoard(c.uid, entry.uid));
    if (own) state.sel = own.uid;
    A.shell.show('chipsats');
    render();
  };

  A.shell.register('chipsats', {
    show() {
      A.shell.seen();
      A.shell.pollJobs();
      load();
      clearInterval(state.timer);
      state.timer = setInterval(lookForProbes, 1000);
    },
    hide() { clearInterval(state.timer); state.timer = null; },
  });

  // Asked once up front, so the bar can name the board a job is on before this screen
  // has been opened. These are lists the app keeps; none of them touches a board.
  load();
})();
