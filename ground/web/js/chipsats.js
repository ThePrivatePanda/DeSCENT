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
    read: null,         // what the last Read the board said, while its probe stays plugged in
    stale: '',          // the job that has changed the board since it was read
    autoRead: [],       // probes already read, or tried, since they were plugged in
    peek: null,         // the question "which board is under the clip" while it is being asked
    tried: {},          // probe -> the board the last read nobody asked for was started on
    looks: 0,
    reading: false, readError: '',
    sel: null,
    loaded: false,      // the app has answered once, so an unknown selection really is gone
    keepFsw: true,
    backup: true,
    pick: null,         // the image about to be written: {kind, label, name, bytes, saved}
    pickFor: null,      // the board it was chosen on, when it was chosen on one
    build: null,        // settings to compile into a fresh image: {uid, csid, sf, dbm, interval}
    refused: '', canForce: false,
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

  // ---------- asking the app ----------
  async function load() {
    const got = await Promise.all([
      hub.chips().catch((e) => e),
      hub.chipProbes().catch((e) => e),
      hub.logs().catch(() => []),
      hub.radio().catch(() => null),
      hub.health().catch(() => null),
    ]);
    state.missing = got[0] instanceof Error && !!got[0].missing;
    state.chips = Array.isArray(got[0]) ? got[0] : [];
    seeProbes(Array.isArray(got[1]) ? got[1] : []);
    state.logs = got[2];
    state.radio = got[3] && got[3].layoutVersion ? got[3] : null;
    state.health = got[4];
    state.loaded = true;
    render();
    readNewProbe();
  }

  // A board can be swapped under a probe without the app seeing it, so nothing is called
  // "on the ST-Link" unless it was read since the probes last changed.
  function forgetRead() {
    state.read = null;
    state.stale = '';
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
    if (!B.swapSeen(uid, shown, state.tried[to])) return;
    state.tried[to] = uid;
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
      state.stale = '';
      state.sel = j.uid;
      state.chips = await hub.chips().catch(() => state.chips);
      toast(B.readToast(j.uid, known));
    } catch (e) {
      // No board answered, so no board is known to be there.
      forgetRead();
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
  function listHtml() {
    const all = rows();
    const none = !all.length
      ? '<p class="empty">No board yet.</p>' : '';
    return none + all.map((r) => {
      let lines;
      if (r.id === ANY) {
        lines = '<small>this build cannot tell boards apart</small>';
      } else if (r.unread) {
        lines = '<small>' + mark('off', 'not read') + '</small><small>' + esc(r.probe.kind + ' ' + r.probe.serial) + '</small>';
      } else {
        lines = '<small>' + (r.module ? '<span class="val">' + B.MODULE[r.module] + '</span>' : mark('bad', 'module never set')) +
            (r.firmware === 'dump' ? ' ' + mark('bad', 'dump firmware') : '') + '</small>' +
          '<small>' + (r.onProbe ? mark('good', 'on the ST-Link')
            : (r.lastSeen ? 'last on the probe ' + esc(when(r.lastSeen)) : 'not read yet')) + '</small>';
      }
      const job = (jobs.pull && jobs.pull.running && pullOwner() === r.uid && r.uid) ? mark('warn', 'pulling')
        : (jobs.chip && jobs.chip.running && chipOwner() === r.uid && r.uid) ? mark('warn', 'flashing') : '';
      return '<button class="item' + (r.id === state.sel ? ' on' : '') + '" data-pick="' + esc(r.id) + '">' +
        '<b>' + esc(titleOf(r)) + '</b>' + (unconfigured(r) ? '<small>' + mark('warn', 'unconfigured') + '</small>' : '') + lines + (job ? '<small>' + job + '</small>' : '') + '</button>';
    }).join('');
  }

  // ---------- the sheet ----------
  const READ_WARNING = 'Reading can reset the board. A board is read without asking when it is first seen under the clip.';

  function headPane(r) {
    if (r.id === ANY) {
      return '<div class="pane sheet-head"><span class="id">ChipSat on the debug probe</span>' +
        '<span class="note line">This build can pull a flight log. It cannot read, name or flash a ChipSat.</span></div>';
    }
    if (r.unread) {
      return '<div class="pane sheet-head"><span class="id">The board on the ST-Link</span>' + mark('off', 'not read') +
        '<span class="note">' + esc(r.probe.kind + ' ' + r.probe.serial) + '</span>' +
        (r.probe.access === false ? mark('bad', 'no permission to open the probe') : '') +
        '<div class="line"><button data-read="' + esc(r.probe.serial) + '" class="primary"' + (busy() ? ' disabled' : '') + '>' +
          (state.reading ? 'Reading the board' : 'Read the board') + '</button>' +
          '<span class="note">' + READ_WARNING + '</span></div>' +
        (state.readError ? '<p class="outcome bad line">Could not read it: ' + esc(state.readError) + '</p>' : '') +
        (state.pick ? '<p class="note line">Chosen to write: ' + esc(state.pick.label) + '. Read the board first.</p>' : '') +
        '</div>';
    }
    const probe = r.onProbe ? r.probe : state.probes[0];
    const readBtn = probe
      ? '<button data-read="' + esc(probe.serial) + '"' + (busy() ? ' disabled' : '') + ' title="' + esc(READ_WARNING) + '">' +
        (state.reading ? 'Reading the board' : r.onProbe ? 'Read again' : 'Read the board') + '</button>'
      : '';
    const where = r.onProbe ? mark('good', 'on the ST-Link') + '<span class="note">' + esc((r.probe.kind || '') + ' ' + (r.probe.serial || '')) + '</span>'
      : mark('off', r.lastSeen ? 'last on the probe ' + when(r.lastSeen) : 'not read yet');
    const running = B.firmwareWords(r.fw, r.module);
    const alarms = [
      running.head ? mark('bad', running.head) +
        (r.onProbe && !state.stale ? '' : '<span class="note">when it was last read</span>') : '',
      r.module ? '' : mark('bad', 'module never set'),
    ].filter(Boolean).join('');
    // Forgetting drops the app's record of the board and nothing else.
    const forgetting = state.pick && state.pick.kind === 'forget' && state.pickFor === r.uid
      ? '<div class="confirm"><span class="grow">Forget <b>' + esc(nameOf(r)) + '</b>? Its name and module go from this computer. ' +
          'The board is not touched, and the images and logs saved off it stay. Read again, it comes back as a new board.</span>' +
        '<button data-forgetgo="1" class="primary"' + (busy() ? ' disabled' : '') + '>Forget it</button>' +
        '<button data-forgetcancel="1">Cancel</button></div>'
      : '';
    return '<div class="pane sheet-head">' +
      '<input type="text" class="nameit" data-keep="chip:' + esc(r.uid) + '" data-name="' + esc(r.uid) + '" value="' + esc(r.name) +
        '" placeholder="' + esc(B.chipName(r)) + '" style="' + nameWidth(r.name || B.chipName(r)) + '" maxlength="40" aria-label="Name of this board" title="Type a name and press Enter">' +
      where + readBtn +
      '<span class="spacer"></span><span class="note">' + (r.onProbe && state.read.flashBytes ? D.bytes(state.read.flashBytes) + ' of flash, ' : '') +
        'UID <span class="hex">' + esc(B.uidWords(r.uid)) + '</span></span>' +
      '<button data-forget="1"' + (busy() ? ' disabled' : '') + '>Forget this board</button>' +
      (alarms ? '<div class="line">' + alarms + '</div>' : '') +
      (probe && !r.onProbe ? '<span class="note line">' + READ_WARNING + '</span>' : '') +
      (state.readError ? '<p class="outcome bad line">Could not read it: ' + esc(state.readError) + '</p>' : '') +
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

  function buildDraft(r) {
    const was = r.onProbe && !state.stale && state.read ? state.read.settings : null;
    if (!state.build || state.build.uid !== r.uid) {
      state.build = {
        uid: r.uid,
        csid: was && was.csid != null ? String(was.csid) : (r.csid != null ? String(r.csid) : '255'),
        sf: was && was.sf != null ? Number(was.sf) : 7,
        dbm: was && was.dbm != null ? Number(was.dbm) : (r.module === 'hp' ? 14 : 14),
        interval: was && was.txIntervalMs != null ? String(was.txIntervalMs) : '1000',
      };
    }
    return state.build;
  }

  // Settings go into the image by building it, which is also what checks them: a power the module
  // has no row for does not flash, it fails to compile and says which powers that module takes.
  // Laid out as the other panes are - a row per setting, then a confirm strip saying what the
  // button will do.
async function buildAndFlash() {
    const r = rows().find((x) => x.id === state.sel);
    if (!r) return;
    // Never nothing: a press that cannot go through says which of the three reasons it is.
    if (!r.module) { state.buildError = 'Choose the radio module first.'; render(); return; }
    if (!r.onProbe) { state.buildError = 'That board is not on the ST-Link. Press Read the board.'; render(); return; }
    if (busy()) { state.buildError = 'Something else is running on this board.'; render(); return; }
    const how = B.flashWay(state.health, r.module);
    if (how.stop) { state.buildError = how.stop; render(); return; }
    const d = buildDraft(r);
    state.buildError = '';
    if (state.peek) await state.peek;
    try {
      const settings = { csid: d.csid, sf: d.sf, dbm: d.dbm, interval: d.interval };
      await hub.chipFlash(Object.assign({ uid: r.uid, backup: state.backup }, settings,
        how.way === 'build' ? { embedded: 'built', module: r.module } : { embedded: 'flight' }));
      toast(how.way === 'build' ? 'Building the flight software for ' + nameOf(r) : 'Flashing ' + nameOf(r));
    } catch (e) {
      state.buildError = e.missing ? CANNOT : (e.message || String(e));
    }
    await A.shell.pollJobs();
    render();
  }

  function radioPane(r) {
    const read = r.onProbe && !state.stale ? state.read : null;
    const settings = read ? read.settings || null : null;
    const t = B.settingsTable(settings, r.module, read ? read.settingsError : '', read && read.firmware ? read.firmware.kind : '');
    const d = buildDraft(r);
    const seg = '<span class="seg" role="group" aria-label="Radio module">' + ['le', 'hp'].map((m) =>
      '<button data-module="' + m + '" aria-pressed="' + (r.module === m) + '"' + (r.module === m ? ' class="on"' : '') + '>' + B.MODULE[m] + '</button>').join('') + '</span>';
    const job = jobs.chip && jobs.chip.what === 'flash' && chipOwner() === r.uid ? jobs.chip : null;
    const how = B.flashWay(state.health, r.module);
    const now = job && job.running ? jobBlock(job, (how.way === 'build' ? 'Building and flashing ' : 'Flashing ') + nameOf(r)) : '';
    const powers = powersFor(r.module || 'le');
    const box = (field, value, hint) => '<input type="text" inputmode="numeric" data-keep="build:' + field + '"' +
      ' data-build="' + field + '" value="' + esc(String(value)) + '" placeholder="' + hint + '"' +
      ' aria-label="' + hint + '" size="8">';
    const pick = (field, values, value, show) => '<select data-build="' + field + '">' +
      values.map((v) => '<option value="' + v + '"' + (Number(v) === Number(value) ? ' selected' : '') + '>' +
        show(v) + '</option>').join('') + '</select>';

    const stop = !r.module ? 'Choose the radio module above first.'
      : how.stop ? how.stop
        : !r.onProbe ? 'Put the board on the ST-Link and press Read the board.'
          : busy() ? 'Something else is running on this board.' : '';
    const buildOff = stop ? ' disabled' : '';

    const head = '<div class="pane"><h3>Radio and identity' + (t.setBy ? '<span class="note">' + esc(t.setBy) + '</span>' : '') + '</h3>' + now +
      (t.disagree ? '<p class="outcome bad inpane">' + esc(t.disagree) + '</p>' : '');

    // How the last write ended stays until the next job, as a flash's does.
    const flashJob = jobs.chip && jobs.chip.what === 'flash' && chipOwner() === r.uid ? jobs.chip : null;
    const last = !flashJob || flashJob.running || state.pick ? ''
      : flashJob.error ? '<p class="outcome bad inpane">Flash failed: ' + esc(flashJob.error) + '</p>'
        : (flashJob.steps || []).length ? '<p class="outcome inpane">Flashed and checked.' +
          (flashJob.saved ? ' What was on the board is saved as <span class="path">' + esc(flashJob.saved) + '</span>.' : '') + '</p>' : '';

    // Only a read can say a board has no block. One that is off the probe, or changed since
    // it was read, gets the dashes and no reason for them.
    const noBlock = read && t.line ? '<p class="' + (t.damaged ? 'outcome bad' : 'plain') + ' inpane">' + esc(t.line) + '</p>' : '';

    const pick_state = state.pick && state.pick.kind === 'build' ? state.pick : null;
    const intervalPhrase = d.interval === '0' ? 'back to back' : 'every ' + d.interval + ' ms';
    const summary = 'board ' + d.csid + ', ' + B.MODULE[r.module || 'le'] + ', SF' + d.sf + ', ' + d.dbm + ' dBm, ' + intervalPhrase;
    // The module is said in words either way: the image for the other module transmits
    // almost nothing and still reports success.
    const module = '<b>' + B.MODULE[r.module || 'le'] + ' module</b>';
    const will = how.way === 'build'
      ? 'Builds the flight software for the ' + module + ' with these settings and writes it to <b>' + esc(nameOf(r)) + '</b>. About 13 seconds.'
      : 'Writes the flight image this app carries for the ' + module + ' to <b>' + esc(nameOf(r)) + '</b>, with these settings in it.';
    const confirm = !pick_state || job && job.running ? '' : '<div class="confirm">' +
      '<span class="grow">' + (stop ? esc(stop) : will) + '</span>' +
      '<button data-buildgo="1" class="primary"' + buildOff + '>' + how.verb + '</button>' +
      '<button data-buildcancel="1">Cancel</button></div>';
    // Which of the two ways this screen is going to use, and why it is that one.
    const way = '<p class="plain inpane">' + esc(how.says) + '</p>';

    const buildError = state.buildError ? '<p class="outcome bad inpane">' + esc(state.buildError) + '</p>' : '';

    const differs = B.buildDiffers(settings, r.module, d);
    // The board's value, struck through when the control beside it would replace it.
    const onBoard = (words, replaced) => '<div><span class="val' + (replaced ? ' was' : '') + '">' + esc(words || '—') + '</span></div>';

    const wrapChanged = (html, replaces) => replaces ? '<span class="changed">' + html + '</span>' : html;

    const table = '<div class="kv">' +
      '<div class="h">Setting</div><div class="h">On the board</div><div class="h">' + how.column + '</div>' +
      '<div class="l">Radio module</div>' + onBoard(t.module, differs.module) + '<div>' + wrapChanged(seg, differs.module) + '</div>' +
      '<div class="l">Board id</div>' + onBoard(t.rows[0].onBoard, differs.csid) + '<div>' +
        wrapChanged(box('csid', d.csid, '1 to 255'), differs.csid) + '<span class="note">1 to 255, goes in every packet</span></div>' +
      '<div class="l">Spreading factor</div>' + onBoard(t.rows[1].onBoard, differs.sf) + '<div>' +
        wrapChanged(pick('sf', [5, 6, 7, 8, 9, 10, 11, 12], d.sf, (v) => 'SF' + v), differs.sf) +
        (AIR_MS[Number(d.sf)] ? '<span class="note">' + AIR_MS[Number(d.sf)] + ' ms on air per packet</span>' : '<span class="note">ms on air per packet</span>') + '</div>' +
      '<div class="l">Transmit power</div>' + onBoard(t.rows[2].onBoard, differs.dbm) + '<div>' +
        wrapChanged(pick('dbm', powers, d.dbm, (v) => v + ' dBm'), differs.dbm) + '<span class="note">the rows the ' + (r.module || 'le').toUpperCase() + ' module has</span></div>' +
      '<div class="l">Transmit interval</div>' + onBoard(t.rows[3].onBoard, differs.interval) + '<div>' +
        wrapChanged(box('interval', d.interval, '1000'), differs.interval) + '<span class="note" style="margin-left: 4px;">ms</span>' +
        '<span class="note">the gap after transmission' + (d.interval !== '0' && AIR_MS[Number(d.sf)] && Number(d.interval) < AIR_MS[Number(d.sf)] ? '. At SF' + d.sf + ' a packet takes ' + AIR_MS[Number(d.sf)] + ' ms, so this sends back to back' : '. 0 sends back to back') + '</span></div>' +
      '<div class="l">' + how.verb + '</div><div><span class="val">' + esc(summary) + '</span></div><div style="grid-column: span 2"><button data-buildembed="1"' + buildOff + '>' + how.verb + '</button></div>' +
      '</div>';
    return head + noBlock + way + table + confirm + last + buildError + '</div>';
  }

  function jobBlock(job, title) {
    return '<div class="job"><div class="job-head"><b>' + esc(title) + '</b><span class="note">' + D.duration((job.seconds || 0) * 1000) + '</span></div>' +
      steps(job) + track(job.pct) + '</div>';
  }

  function firmwarePane(r) {
    const can = r.onProbe && !busy();
    const dis = can ? '' : ' disabled';
    const job = jobs.chip && jobs.chip.what === 'flash' && chipOwner() === r.uid ? jobs.chip : null;
    const images = imagesOf(r.uid);

    // What the board is running is only what the last read found. It is a value while
    // the board is on the probe, and a note about the past once it is not.
    const fw = B.firmwareWords(r.fw, r.module);
    const was = fw.value ? ' ' + fw.value.charAt(0).toUpperCase() + fw.value.slice(1) + ' when last read.' : '';
    const on = !r.onProbe ? '<span class="note">Not on the ST-Link.' + esc(was) + '</span>'
      : state.stale ? '<span class="note">Changed by the ' + esc(state.stale) + '. Read again.</span>'
        : (fw.bad ? mark('bad', fw.value) : '<span class="val">' + esc(fw.value || 'Not read') + '</span>') +
          (fw.note ? '<br><span class="note">' + esc(fw.note) + '</span>' : '');
    const wrongModule = fw.mismatch && !(r.onProbe && state.stale) ? '<p class="outcome bad inpane">' + esc(fw.mismatch) + '</p>' : '';

    const saved = images.length
      ? images.map((l) => '<div><span class="l">Saved off this board</span><span class="grow val">' + esc(D.dayTime(new Date(l.when * 1000), new Date())) + ', ' + D.bytes(l.bytes) + '</span>' +
          '<button data-putback="' + esc(l.name) + '"' + dis + '>Put back</button></div>').join('')
      : '<div><span class="l">Saved off this board</span><span class="note">nothing yet</span></div>';

    const pick = state.pick && (state.pick.kind === 'saved' || state.pick.kind === 'file') ? state.pick : null;
    const confirm = !pick || job && job.running ? '' : '<div class="confirm">' +
      '<span class="grow">Write <b>' + esc(pick.label) + '</b> to <b>' + esc(nameOf(r)) + '</b>? ' +
        (state.backup ? 'The image on it now is saved first.' : 'The image on it now is not saved.') +
        (r.onProbe ? '' : ' Put the board on the ST-Link and press Read the board first.') + '</span>' +
      (state.canForce ? '' : '<button data-flashgo="1" class="primary"' + dis + '>Overwrite the board</button>') +
      '<button data-flashcancel="1">Cancel</button></div>';
    const refused = !state.refused ? '' : '<p class="outcome bad inpane">Not written: ' + esc(state.refused) + '</p>' +
      (state.canForce && pick ? '<div class="confirm"><span class="grow">That image came off a different board.</span>' +
        '<button data-flashforce="1"' + dis + '>Write it to this board anyway</button></div>' : '');

    // A job that is running goes first, above the buttons it has switched off. How the
    // last one ended goes last, and gives way to whatever is being confirmed now.
    const now = job && job.running ? jobBlock(job, 'Flashing ' + nameOf(r)) : '';
    let last = '';
    if (now || pick || state.refused) last = '';
    else if (job && job.error) last = '<p class="outcome bad inpane">Flash failed: ' + esc(job.error) + '</p>' +
      (job.saved ? '<p class="step inpane">What was on the board is saved as <span class="path">' + esc(job.saved) + '</span>.</p>' : '');
    else if (job && (job.steps || []).length) last = '<p class="outcome inpane">Flashed and checked.' +
      (job.saved ? ' What was on the board is saved as <span class="path">' + esc(job.saved) + '</span>.' : '') + '</p>';

    return '<div class="pane"><h3>Firmware</h3>' + now + wrongModule + '<div class="rows kvrows">' +
      '<div><span class="l">On the board</span><span class="grow">' + on + '</span></div>' +
      saved +
      '<div><span class="l">From a file</span><label class="button' + (can ? '' : ' disabled') + '">Flash a file…<input type="file" accept=".bin" data-file="1" hidden' + dis + '></label>' +
        '<span class="note">A file\'s radio settings are whatever it was built with.</span></div>' +
      '<div><label class="check"><input type="checkbox" data-backup="1"' + (state.backup ? ' checked' : '') + '> Save what is on the board first</label></div>' +
      '</div>' + confirm + refused + last + '</div>';
  }

  // What a finished pull left behind: how much of the log came off the chip, where it is
  // on this computer, and the ways to take it somewhere else. An older build of the app
  // sends only the log's name, so each part is drawn when it is there and left out when
  // it is not.
  // boardCsid is the id the board's own settings gave when it was read, if they gave one.
  function pullResult(s, boardCsid) {
    const rep = s.report || {};
    const n = (v) => Number(v).toLocaleString();
    const got = [];
    if (Number.isFinite(rep.read)) {
      got.push(n(rep.read) + (Number.isFinite(rep.found) ? ' of ' + n(rep.found) : '') + ' records');
    }
    if (Array.isArray(rep.boots)) {
      got.push(n(rep.boots.length) + (rep.boots.length === 1 ? ' boot' : ' boots'));
      got.push(n(rep.boots.filter((b) => b && b.timeSource === 'anchor').length) + ' with GPS time');
    }
    if (D.boardIdWords(rep.csids)) got.push(D.boardIdWords(rep.csids));
    const wrong = D.idMismatch(rep.csids, boardCsid);
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
    const word = rep.complete === false ? 'Incomplete' : 'Warning';
    const where = [
      s.dir ? 'In <span class="path">' + esc(s.dir) + '</span>.' : '',
      s.restored ? 'The flight software was put back.' : '',
    ].filter(Boolean).join(' ');
    return '<p class="outcome inpane">' +
        esc(got.length ? 'Last pull: ' + got.join(', ') + '.' : 'Last pull wrote ' + s.log + '.') + '</p>' +
      (wrong ? '<p class="outcome bad inpane">' + esc(wrong) + '</p>' : '') +
      (short.length ? '<p class="outcome warn inpane">' + word + ': ' + esc(short.join(', ')) + '.</p>' : '') +
      (where ? '<p class="step inpane">' + where + '</p>' : '') +
      // A pull that was told not to put the flight software back ends well and still
      // leaves a board that will not fly.
      (s.restored ? '' : '<p class="outcome bad inpane">The board is still running the dump firmware, not flight software.</p>') +
      '<div class="acts inpane">' +
        '<button data-openlog="' + esc(s.log) + '" class="primary">Open</button>' +
        '<a class="button" download="' + esc(flatName(s.log)) + '" href="' + logUrl(s.log) + '">Save log</a>' +
        '<button data-csv="' + esc(s.log) + '">Save CSV</button>' +
        (s.raw ? '<a class="button" download="' + esc(flatName(s.raw)) + '" href="' + logUrl(s.raw) + '">Save raw</a>' : '') +
      '</div>';
  }

  // The console port for this board, as last chosen for it, while that port is still here.
  function consoleFor(r) {
    const ports = (hub.boards || []).map((b) => b.port);
    const kept = store.get('dg-console-' + (r.uid || ANY));
    if (kept && ports.includes(kept)) return kept;
    const free = (hub.boards || []).find((b) => b.state !== 'receiver');
    return free ? free.port : '';
  }

  function logPane(r) {
    const s = jobs.pull;
    // A pull nobody can put a board to is shown on whichever sheet is up, with the port
    // it is reading, rather than on none.
    const mine = s && (pullOwner() === r.uid || !pullOwner() || r.id === ANY);
    const boards = hub.boards || [];
    const port = consoleFor(r);
    const ready = (r.onProbe || r.id === ANY) && !busy() && !!port;

    if (mine && s.running) {
      return '<div class="pane"><h3>Flight log</h3>' +
        jobBlock(s, 'Pulling the flight log' + (s.board ? ', console on ' + s.board : '')) +
        '<p class="note inpane">Leave the board plugged in.</p></div>';
    }

    // What happened last time stays on screen: a pull that failed after reflashing leaves a
    // board running the wrong firmware, and that must not vanish with a toast.
    let last = '';
    if (mine && s.error) {
      last = '<p class="outcome bad inpane">Pull failed: ' + esc(s.error) + '</p>' +
        '<div class="inpane">' + steps(s) + '</div>' +
        (s.keepFsw && !s.restored
          ? '<p class="outcome bad inpane">The flight software was not put back. The board is still running the dump firmware' +
            (s.savedFsw ? '; the image that was on it is saved as <span class="path">' + esc(s.savedFsw) + '</span>' : '') + '.</p>'
          : '');
    } else if (mine && s.log) {
      // The id read off the board before the pull is still its id: a pull puts the same image back.
      const read = r.onProbe && state.read.settings ? state.read.settings.csid : null;
      last = pullResult(s, read);
    } else {
      const earlier = pullsOf(r.uid)[0];
      if (earlier) {
        last = '<div class="rows"><div><span class="l">Last pull</span><span class="grow val">' + esc(D.dayTime(new Date(earlier.when * 1000), new Date())) + ', ' + D.bytes(earlier.bytes) + '</span>' +
          '<button data-openlog="' + esc(earlier.name) + '">Open</button><button data-todata="1">Show in Data</button></div></div>';
      }
    }

    const chooser = boards.length
      ? '<select data-console="' + esc(r.uid || ANY) + '" aria-label="Serial port of this board\'s console">' +
          boards.map((b) => '<option value="' + esc(b.port) + '"' + (b.port === port ? ' selected' : '') + '>' +
            esc((b.name || b.port) + (b.state === 'receiver' ? ', being read as a receiver' : '')) + '</option>').join('') + '</select>' +
        '<span class="note">The board\'s serial console.</span>'
      : '<span class="note">No serial port. Plug in the board\'s console.</span>';

    return '<div class="pane"><h3>Flight log</h3>' + last +
      '<div class="rows kvrows">' +
        '<div><span class="l">Console port</span>' + chooser + '</div>' +
        '<div><label class="check"><input type="checkbox" data-keepfsw="1"' + (state.keepFsw ? ' checked' : '') +
          '> Save the flight software that is on it and put it back</label>' +
          (state.keepFsw ? '' : mark('warn', 'the board will be left on the dump firmware')) + '</div>' +
        '<div><button data-pull="1" class="primary"' + (ready ? '' : ' disabled') + '>Pull the flight log</button>' +
          '<span class="note">' + (r.onProbe || r.id === ANY
            ? 'Flashes the board twice. Takes minutes.'
            : 'Put the board on the ST-Link and press Read the board first.') + '</span></div>' +
      '</div></div>';
  }

  function sheetHtml() {
    const r = rows().find((x) => x.id === state.sel);
    if (!r) {
      return '<div class="pane"><p class="empty">Clip a ChipSat onto the ST-Link and plug the ST-Link in.</p></div>';
    }
    if (r.unread) return headPane(r);
    if (r.id === ANY) return headPane(r) + logPane(r);
    return headPane(r) + radioPane(r) + '<div class="two">' + firmwarePane(r) + logPane(r) + '</div>';
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

async function pull() {
    const r = rows().find((x) => x.id === state.sel);
    if (!r || busy()) return;
    const port = consoleFor(r);
    if (!port) return;
    if (state.peek) { await state.peek; if (busy()) return; }
    try {
      const more = {};
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

  // A log as a table, straight from the file the app wrote.
  async function saveCsv(name) {
    try {
      const r = await fetch(logUrl(name));
      if (!r.ok) throw new Error('HTTP ' + r.status);
      const events = DG.recorder.mergeFiles([DG.recorder.parseLogFile(await r.text(), name, 0)]);
      const csv = DG.exportcsv.toCsv(events);
      const count = csv.split('\n').length - 2;   // less the header and the last newline
      if (count < 1) { toast('Nothing to save: no packet in this log passes its CRC.'); return; }
      const out = flatName(name).replace(/\.[^.]*$/, '') + '.csv';
      A.ui.download(new Blob([csv], { type: 'text/csv' }), out);
      toast('Saved ' + count.toLocaleString() + ' rows to ' + out);
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
      state.pickFor = r.uid;
      state.pick = { kind: 'build', label: 'the flight software' };
    } else if (at('data-buildgo')) {
      buildAndFlash();
      return;
    } else if (at('data-buildcancel')) {
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
      state.build[t.getAttribute('data-build')] = Number(t.value);
      state.buildError = '';
      t.blur();
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
  A.shell.onJobs((ended) => {
    for (const kind of ended) {
      const j = jobs[kind];
      if (kind === 'pull') {
        toast(j.error ? 'Pull failed: ' + j.error : 'Flight log pulled. Open it from ChipSats.');
        state.stale = 'pull';
      } else if (j.what === 'flash') {
        toast(j.error ? 'Flash failed: ' + j.error : 'Flashed and checked.');
        // The job reads the board again once it has written it, and that read is the
        // sheet's. Without it nobody knows what is on the board, so the sheet says to read.
        if (!j.error && j.read && j.read.uid) {
          state.read = j.read;
          state.stale = '';
        } else {
          state.stale = 'flash';
        }
      }
    }
    if (ended.length) load();
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
