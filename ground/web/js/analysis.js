// The Analysis screen: a pulled flight log, or a session log, as the app reads it (/api/check, the
// same reading as `descent-ground check`), boot by boot. What it shows is the firmware's own codes,
// flags and counts and the values it logged. Nothing here says whether a value looks right, and a
// phase is only ever what a phase event said. The words and markup are pure and sit first, so the
// tests can load them in node; the screen below needs the page and the app.
(function () {
  'use strict';
  const D = typeof module === 'object' && module.exports ? require('./datamodel.js') : DG.datamodel;
  const fmt = new Intl.NumberFormat('en-US');
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const mark = (cls, word) => '<span class="mark ' + cls + '">' + esc(word) + '</span>';
  const plural = (n, one) => fmt.format(n) + ' ' + one + (n === 1 ? '' : 's');
  const LISTED = 20;   // gaps and spells drawn per boot; the rest are counted

  // As check.rs prints a length of time, so the page and the command line read the same
  function secs(ms) {
    if (ms < 1000) return ms + ' ms';
    if (ms < 120000) return (ms / 1000).toFixed(1) + ' s';
    if (ms < 7200000) return (ms / 60000).toFixed(1) + ' min';
    return (ms / 3600000).toFixed(1) + ' h';
  }
  // Uptime: on the boot's own clock, the only clock a boot without GPS time has
  const up = (ms) => 'up ' + secs(ms);
  const span = (a, b) => (a === b ? up(a) : up(a) + ' to ' + secs(b));

  function endWords(b) {
    if (b.end === 'pull') return 'restarted for the pull';
    if (b.end === 'unknown') return 'not known, the next boot is not in the log';
    return b.endReset ? 'reset ' + b.endReset.split('+').join(', ') : 'reset, cause not in the log';
  }

  function times(b, source) {
    return b.systemEvents.filter((s) => !source || s.source === source).reduce((n, s) => n + s.count, 0);
  }

  // A pull's folder is named for the minute it started, <day>/pulls/1813-board15
  function pullWhen(name, when, now) {
    const m = /^(\d{4})-(\d{2})-(\d{2})\/pulls\/(\d{2})(\d{2})/.exec(name);
    if (m) return D.dayTime(new Date(+m[1], m[2] - 1, +m[3], +m[4], +m[5]), now);
    return when ? D.dayTime(new Date(when * 1000), now) : 'time not known';
  }

  // The pull folders and the session logs, newest first. A pull's ground.log from /api/logs is the
  // same pull as its folder and is not listed twice. names: uid -> what the board is called here.
  function itemsOf(pulls, logs, names, now) {
    const nameOf = (uid) => {
      const key = uid && Object.keys(names).find((k) => k.toLowerCase().endsWith(String(uid).toLowerCase().slice(-8)));
      return key ? names[key] : '';
    };
    const items = pulls.map((p) => ({
      kind: 'pull', name: p.name, when: p.when || 0,
      title: nameOf(p.uid) || D.boardPart(p.name + '/dump.txt') || 'A board',
      at: pullWhen(p.name, p.when, now),
      check: p.dumpBytes === null ? null : p.name + '/dump.txt',
      dumpBytes: p.dumpBytes, log: p.log ? p.name + '/ground.log' : null,
      report: p.report ? p.name + '/report.json' : null,
    }));
    for (const l of logs) {
      if (l.kind !== 'session') continue;
      items.push({ kind: 'log', name: l.name, when: l.when || 0, title: 'Session log', at: l.when ? D.dayTime(new Date(l.when * 1000), now) : '',
        check: l.name, log: l.name, file: D.baseName(l.name), bytes: l.bytes });
    }
    return items.sort((a, b) => b.when - a.when);
  }

  function itemHtml(it, sel) {
    const sub = [it.kind === 'pull' ? 'Pull' : esc(it.file), esc(it.at)];
    if (it.kind === 'pull' && it.dumpBytes === null) sub.push(mark('warn', 'no dump.txt'));
    else if (it.kind === 'pull' && it.dumpBytes === 0) sub.push(mark('warn', 'empty'));
    return '<button class="item' + (it.name === sel ? ' on' : '') + '" data-pick="' + esc(it.name) + '">' +
      '<b>' + esc(it.title) + '</b><small>' + sub.join(' · ') + '</small></button>';
  }

  function table(head, rows, cls) {
    return '<div class="an-wrap"><table class="grid plain' + (cls ? ' ' + cls : '') + '"><thead><tr>' +
      head.map((h) => '<th' + (h.r ? ' class="r"' : '') + '>' + esc(h.t || h) + '</th>').join('') + '</tr></thead><tbody>' +
      rows.join('') + '</tbody></table></div>';
  }

  // The phase lane: one stretch per phase event, sized by uptime. imufast and imuslow change the
  // IMU's rate and not the phase, so they are ticks on it.
  function laneHtml(b) {
    if (!b.phases.length) {
      return '<p class="note">No phase events in this boot. Firmware from before 2026-11-06 wrote none.</p>';
    }
    const end = Math.max(1, b.phases[b.phases.length - 1].toMs);
    const at = (ms) => (100 * ms / end).toFixed(2) + '%';
    const name = (s) => s.phase + (s.heldFix ? ', held fix' : '');
    const segs = b.phases.map((s) => '<span class="seg' + (s.heldFix ? ' held' : '') + '" style="left:' + at(s.fromMs) + ';width:' +
      at(s.toMs - s.fromMs) + '" title="' + esc(name(s) + ', ' + span(s.fromMs, s.toMs)) + '"><span>' + esc(name(s)) + '</span></span>');
    const ticks = b.events.filter((e) => e.source === 'phase' && (e.what === 'imufast' || e.what === 'imuslow'))
      .map((e) => '<span class="tick" style="left:' + at(e.firstMs) + '" title="' + esc(e.what + ', ' + span(e.firstMs, e.lastMs)) + '"></span>');
    const said = b.phases.map((s) => {
      const why = s.by ? 'by ' + s.by + (s.reason && s.reason !== 'none' ? ', ' + s.reason : '') + (s.count > 1 ? ', ' + s.count + ' times from here' : '')
        : 'every boot starts in boot-air';
      return '<li><b>' + esc(name(s)) + '</b> ' + esc(span(s.fromMs, s.toMs)) + ' <span class="note">' + esc(why) + '</span></li>';
    });
    return '<div class="lane" role="img" aria-label="Phases from the phase events">' + segs.join('') + ticks.join('') + '</div>' +
      '<ol class="phaselist">' + said.join('') + '</ol>';
  }

  function eventsHtml(b) {
    if (!b.events.length) return '<p class="note">No system events in this boot.</p>';
    const rows = b.events.map((e) => {
      const reason = e.reason === 'none' ? '' : esc(e.reason) + (e.reasonMeaning ? ' <span class="note">' + esc(e.reasonMeaning) + '</span>' : '');
      const detail = e.detailWords ? esc(e.detailWords) + (e.count > 1 ? ' <span class="note">the last time</span>' : '') : '';
      return '<tr><td>' + esc(span(e.firstMs, e.lastMs)) + '</td><td><b>' + esc(e.source + ' ' + e.what) + '</b>' +
        (e.part !== 'none' ? ' on ' + esc(e.part) : '') + '</td><td class="wrap">' + reason + '</td><td class="r">' + fmt.format(e.count) +
        '</td><td>' + detail + '</td><td class="wrap note">' + esc(e.meaning) + '</td></tr>';
    });
    return table(['When', 'Event', 'Reason', { t: 'Times', r: true }, 'Detail', 'What it is'], rows, 'events') +
      (b.eventsLeftOut ? '<p class="note">And ' + plural(b.eventsLeftOut, 'later run') + ' not listed: the app sends the first 200 of a boot.</p>' : '');
  }

  // The firmware's valid bit, record by record, over the boot's records. Dark is clear.
  function flagsHtml(b) {
    const dur = b.lastMs - b.firstMs;
    const at = (ms) => (dur > 0 ? (100 * (ms - b.firstMs) / dur).toFixed(2) : '0') + '%';
    const rows = b.sensors.map((s) => {
      const drawn = s.longest.slice(0, LISTED);
      const bar = s.clear === s.records ? '<span class="off" style="left:0;width:100%"></span>'
        : drawn.map((p) => '<span class="off" style="left:' + at(p.fromMs) + ';width:max(2px,' + (dur > 0 ? (100 * p.ms / dur).toFixed(2) : '100') + '%)"></span>').join('');
      const words = s.clear === 0 ? 'set on all ' + fmt.format(s.records)
        : s.clear === s.records ? 'clear on all ' + fmt.format(s.records)
          : 'clear on ' + fmt.format(s.clear) + ' of ' + fmt.format(s.records) + ', ' + plural(s.spells, 'spell') + ', ' + secs(s.clearMs) + ' in all';
      const left = s.clear < s.records && s.spells > drawn.length ? ', the ' + fmt.format(s.spells - drawn.length) + ' shortest not drawn' : '';
      return '<div class="flag"><b>' + esc(s.name) + '</b><span class="strip">' + bar + '</span><span class="note">' + esc(words + left) + '</span></div>';
    });
    return '<div class="flags">' + rows.join('') + '</div>' +
      '<p class="note">All-fresh flag set on ' + fmt.format(b.fresh) + ' of ' + plural(b.records, 'record') + '.</p>';
  }

  function rangesHtml(b) {
    const cell = (v, ms, unit) => (v === null ? '—' : '<b>' + Math.round(v * 100) / 100 + '</b> ' + esc(unit) + ' <span class="note">' + esc(up(ms)) + '</span>');
    const rows = b.ranges.map((r) => '<tr><td>' + esc(r.words) + '</td><td>' + cell(r.min, r.minMs, r.unit) + '</td><td>' + cell(r.max, r.maxMs, r.unit) +
      '</td><td class="r">' + fmt.format(r.set) + '</td><td class="r">' + fmt.format(r.clear) + '</td><td>' + esc(r.flag) +
      (r.heldFix ? ' <span class="note">' + plural(r.heldFix, 'reading') + ' on a held fix left out</span>' : '') + '</td></tr>');
    return table(['Reading', 'Lowest', 'Highest', { t: 'Flag set', r: true }, { t: 'Flag clear', r: true }, 'Flag'], rows, 'ranges') +
      '<p class="note">Only readings whose valid bit is set are in the lowest and highest. Acceleration and rotation are the length of the three axes.</p>';
  }

  function gapsHtml(b) {
    const gaps = b.gaps.slice().sort((x, y) => x.afterMs - y.afterMs);
    const rows = gaps.slice(0, LISTED).map((g) => '<li><b>' + secs(g.ms) + '</b> after ' + esc(up(g.afterMs)) + ' · ' +
      (g.droppedRose ? 'the dropped count rose in it' : 'cause not in the log') + '</li>');
    const spacing = 'Record spacing, which is not a phase: ' + secs(b.fastMs) + ' close together, ' + secs(b.steadyMs) + ' at a steady slower rate, ' +
      secs(b.gapMs) + ' in gaps' + (b.spacingChanges.length ? ', changing ' + plural(b.spacingChanges.length, 'time') : '') + '.';
    return (rows.length ? '<ul class="gaps">' + rows.join('') + '</ul>' : '<p class="note">No gaps between records.</p>') +
      (gaps.length > LISTED ? '<p class="note">And ' + plural(gaps.length - LISTED, 'more gap') + '.</p>' : '') +
      '<p class="note">' + esc(spacing) + '</p>';
  }

  // What the log cannot say about this boot, so it is not read as having said it
  function holesHtml(b) {
    const said = [];
    if (b.end === 'pull') said.push('This boot ended when the board was restarted for the pull, not in a reset of its own.');
    said.push('Up to about 30 s before ' + (b.end === 'pull' ? 'that restart' : 'its reset') + ' may be missing: what was still in the RAM ring and the system events still pooling.');
    if (!b.records) said.push('No records in this boot' + (b.events.length ? ', only its events.' : '.'));
    if (b.phases.some((s) => s.heldFix)) said.push('While landed asleep the GPS fields are the fix it held, with the valid bit set.');
    said.push('On the ground the current firmware keeps one packet record per ground log gap.');
    return '<ul class="holes">' + said.map((s) => '<li>' + esc(s) + '</li>').join('') + '</ul>';
  }

  function bootHtml(b) {
    const parts = [plural(b.records, 'record')];
    if (b.records) parts.push(span(b.firstMs, b.lastMs));
    parts.push(b.startUtc ? 'first record ' + D.utcStamp(Date.parse(b.startUtc)) : 'no GPS time');
    parts.push('ended: ' + endWords(b));
    const section = (title, body) => '<div class="an-part"><h4>' + title + '</h4>' + body + '</div>';
    return '<div class="pane an-boot" data-bootpane="' + b.n + '"><h3>Boot ' + b.n + '<span class="note">' + esc(parts.join(' · ')) + '</span></h3>' +
      section('Phases', laneHtml(b)) +
      section('System events', eventsHtml(b)) +
      (b.records ? section('Validity flags', flagsHtml(b)) + section('Readings', rangesHtml(b)) + section('Gaps', gapsHtml(b)) : '') +
      section('Not in the log', holesHtml(b)) +
      '</div>';
  }

  function bootTableHtml(boots, open) {
    const rows = boots.map((b) => {
      const opened = open.has(b.n);
      const phase = times(b, 'phase');
      return '<tr data-boot="' + b.n + '"' + (opened ? ' class="sel"' : '') + '>' +
        '<td><button class="expander" data-boot="' + b.n + '" aria-expanded="' + opened + '">Boot ' + b.n + '</button></td>' +
        '<td class="r">' + fmt.format(b.records) + '</td>' +
        '<td>' + (b.records ? esc(span(b.firstMs, b.lastMs)) : '—') + '</td>' +
        '<td>' + esc(endWords(b)) + '</td>' +
        '<td>' + (b.startUtc ? esc(D.utcStamp(Date.parse(b.startUtc))) : '<span class="note">no GPS time</span>') + '</td>' +
        '<td class="r">' + (phase ? fmt.format(phase) : '<span class="note">none</span>') + '</td>' +
        '<td class="r">' + fmt.format(times(b)) + '</td>' +
        '<td class="r">' + fmt.format(b.dropped) + '</td></tr>';
    });
    return table(['Boot', { t: 'Records', r: true }, 'Uptime', 'How it ended', 'GPS time', { t: 'Phase events', r: true },
      { t: 'System events', r: true }, { t: 'Dropped', r: true }], rows, 'boots');
  }

  // o: {item, report, open: Set of boot numbers, now}
  function pullHtml(check, o) {
    const boots = check.boots.slice().sort((a, b) => a.n - b.n);
    const fw = (o.report && o.report.firmware) || {};
    const lines = [];
    if (check.fromLog) {
      lines.push('Read from a log made out of a pull. It keeps no reset causes, no dropped counts and no slot counts: dump.txt has those.');
    } else if (check.format === 'none') {
      lines.push('Nothing in this pull reads as a record: dump.txt is ' + D.bytes(check.bytes) + '.');
    } else {
      lines.push(fmt.format(check.read) + ' slots read' + (Number.isFinite(check.found) ? ' of ' + fmt.format(check.found) + ' the board counted' : ', the board never gave its count') +
        ', ' + fmt.format(check.failedCheck) + ' failed their check, ' + fmt.format(check.missing) + ' missing' +
        (check.boardBad ? ', ' + fmt.format(check.boardBad) + ' the board itself called bad' : '') + '.');
    }
    if (check.wrapped) lines.push('The log went round the chip: the oldest records were written over.');
    if (check.bootsNotInLog.length) lines.push('Boots not in the log: ' + check.bootsNotInLog.join(', ') + '.');
    if (check.lockEvents) lines.push(plural(check.lockEvents, 'lock event') + ' from the older ring, counted and not read.');
    if (check.printings > 1) lines.push(check.printings + ' printings in the file; the last whole one is what was read.');
    if (check.csidDiffers) lines.push(check.csidDiffers.charAt(0).toUpperCase() + check.csidDiffers.slice(1) + '.');
    const fwWords = [
      fw.dump && fw.dump.words ? 'Read with the ' + fw.dump.words + '.' : '',
      fw.onBoard && fw.onBoard.built && fw.onBoard.built.words ? 'On the board: ' + fw.onBoard.built.words + '.' : '',
    ].filter(Boolean);
    if (!o.report) fwWords.push('No report.json in this pull, so the firmware is not known here.');
    const set = check.consoles.slice().reverse().find((s) => s.settings);
    const settings = set
      ? '<p><span class="kicker">Settings at the pull</span> <span class="hex">' + esc(set.settings) + '</span>' + (set.settingsCut ? ' <span class="note">line cut short</span>' : '') +
        (set.refused ? ' ' + mark('bad', 'refused ' + set.refused) : '') + '</p>' +
        '<p class="note">As the board printed them when it restarted for the pull. Flash keeps no settings, so an older boot may have run with others.</p>'
      : '<p class="note">No settings line in this pull.</p>';
    const unread = check.unreadLines
      ? '<p class="note">' + plural(check.unreadLines, 'line') + ' not read, for example: ' + check.unreadSamples.map((s) => '<span class="hex">' + esc(s) + '</span>').join(', ') + '</p>' : '';
    const state = check.format === 'none' ? mark('warn', 'nothing read') : check.fromLog ? '' : check.complete ? mark('good', 'complete') : mark('warn', 'incomplete');
    const head = '<div class="pane sheet-head an-head"><div class="top"><span class="id">' + esc(o.item.title) + '</span>' + state +
      (o.item.log ? '<span class="acts"><button class="primary" data-open="' + esc(o.item.name) + '">Open in Replay</button></span>' : '') + '</div>' +
      '<p class="line">Pull of ' + esc(o.item.at) + ' · ' + esc(check.board) + (check.uid ? ' · chip id <span class="hex">' + esc(check.uid) + '</span>' : '') +
        ' · ' + plural(boots.length, 'boot') + '</p>' +
      lines.map((l) => '<p class="line">' + esc(l) + '</p>').join('') +
      fwWords.map((l) => '<p class="line note">' + esc(l) + '</p>').join('') +
      settings + unread + '</div>';
    if (!boots.length) return head;
    const opened = boots.filter((b) => o.open.has(b.n));
    return head +
      '<div class="pane an-boots"><h3>Boots<span class="note">in chip order. Each one ended with the next one\'s reset cause; the last ended with the restart for the pull.' +
        (o.item.log ? ' In Replay a boot\'s records come in as the receiver flash-boot and its number.' : '') + '</span></h3>' +
        bootTableHtml(boots, o.open) + '</div>' +
      opened.map(bootHtml).join('');
  }

  function logHtml(check, o) {
    const when = check.firstT !== null && check.lastT !== null
      ? (check.dated ? D.dayTime(new Date(check.firstT), o.now) + ' to ' + D.dayTime(new Date(check.lastT), o.now) : secs(check.lastT - check.firstT) + ' of lines')
      : 'no lines in this log';
    const range = (r, unit) => (r ? r[0] + ' to ' + r[1] + ' ' + unit : '—');
    const boards = check.boards.map((b) => {
      const clear = Object.keys(b.clear).filter((k) => b.clear[k] > 0).map((k) => k + ' ' + fmt.format(b.clear[k])).join(', ');
      return '<tr><td><b>' + b.csid + '</b></td><td class="r">' + fmt.format(b.packets) + '</td><td class="r">' + (b.fromChip ? '—' : fmt.format(b.missed)) +
        '</td><td class="r">' + fmt.format(b.restarts) + '</td><td>' + esc(range(b.rssi, 'dBm')) + '</td><td>' + esc(range(b.snr, 'dB')) +
        '</td><td>' + esc(secs(b.longestQuietMs)) + '</td><td class="wrap">' + (clear ? esc(clear) : '<span class="note">none</span>') + '</td></tr>';
    });
    const receivers = check.receivers.map((r) => '<tr><td><b>' + esc(r.key) + '</b>' + (r.id ? ' <span class="hex">' + esc(r.id) + '</span>' : '') +
      '</td><td>' + (r.sf ? 'SF' + esc(r.sf) : '—') + '</td><td class="r">' + fmt.format(r.packets) + '</td><td class="r">' + fmt.format(r.badCrc) +
      '</td><td class="r">' + fmt.format(r.radioErrors) + '</td><td class="r">' + fmt.format(r.boots) + '</td><td>' + esc(secs(r.longestQuietMs)) + '</td></tr>');
    const consoles = check.consoles.map((s) => {
      const watch = Object.keys(s.watch).map((k) => k + ' ×' + s.watch[k]).join(', ');
      const others = Object.keys(s.others).map((k) => k + ' ×' + s.others[k]).join(', ');
      return '<li><b>' + esc(s.source) + '</b> ' + esc(span(s.firstMs, s.lastMs)) + ', ' + plural(s.lines, 'line') +
        (s.reset ? ', reset ' + esc(s.reset) : '') + (s.hardwareWatchdog ? ' ' + mark('bad', 'hardware watchdog') : '') +
        (watch ? '<br><span class="note">recovery and bus lines: ' + esc(watch) + '</span>' : '') +
        (s.phases.length ? '<br><span class="note">phase lines: ' + esc(s.phases.map((p) => up(p.ms) + ' ' + p.text).join(', ')) + '</span>' : '') +
        (others ? '<br><span class="note">other warnings and errors: ' + esc(others) + '</span>' : '') + '</li>';
    });
    return '<div class="pane sheet-head an-head"><div class="top"><span class="id">Session log</span>' +
      (o.item.log ? '<span class="acts"><button class="primary" data-open="' + esc(o.item.name) + '">Open in Replay</button></span>' : '') + '</div>' +
      '<p class="line">' + esc(o.item.file) + ' · ' + esc(when) + ' · ' + plural(check.lines, 'line') + ' · ' + D.bytes(check.bytes) + '</p>' +
      (check.badCrc || check.badLength ? '<p class="line">' + fmt.format(check.badCrc) + ' failed the CRC, ' + fmt.format(check.badLength) + ' of the wrong length.</p>' : '') +
      (check.unreadLines ? '<p class="note">' + plural(check.unreadLines, 'line') + ' not read.</p>' : '') + '</div>' +
      '<div class="pane"><h3>Boards heard<span class="note">missed and restarts as the packet counter says</span></h3>' +
        (boards.length ? table(['CSID', { t: 'Packets', r: true }, { t: 'Missed', r: true }, { t: 'Restarts', r: true }, 'RSSI', 'SNR', 'Longest quiet', 'Flags clear, packets'], boards)
          : '<p class="empty">No board heard in this log.</p>') + '</div>' +
      '<div class="pane"><h3>Receivers</h3>' +
        (receivers.length ? table(['Receiver', 'SF', { t: 'Packets', r: true }, { t: 'Bad CRC', r: true }, { t: 'Radio errors', r: true }, { t: 'Restart lines', r: true }, 'Longest quiet'], receivers)
          : '<p class="empty">No receiver lines in this log.</p>') + '</div>' +
      '<div class="pane"><h3>Consoles caught</h3>' +
        (consoles.length ? '<ul class="consoles">' + consoles.join('') + '</ul>'
          : '<p class="empty">No console in this log, so no sensor recovery, bus or phase lines.</p>') + '</div>';
  }

  const pure = { itemsOf, itemHtml, pullHtml, logHtml };
  if (typeof module === 'object' && module.exports) module.exports = pure;
  if (typeof window === 'undefined') return;

  const A = window.DGApp;
  const { $ } = A.ui;
  const { paint } = A.shell;
  const hub = A.hub;
  if (!hub.native) return;

  // checks: name -> {check} | {error}, asked once a pick. open: name -> Set of opened boot numbers
  const state = { items: [], sel: null, failed: '', checks: new Map(), reports: new Map(), open: new Map(), loaded: false };

  async function load() {
    let pulls = [];
    let logs = [];
    let names = {};
    state.failed = '';
    try {
      [pulls, logs] = await Promise.all([hub.pulls(), hub.logs()]);
    } catch (e) {
      state.failed = 'Could not read the folder: ' + (e.message || e);
    }
    try {
      for (const c of await hub.chips()) if (c.name) names[c.uid] = c.name;
    } catch (e) { names = {}; }   // the folder names still say which board
    state.items = itemsOf(pulls, logs, names, new Date());
    if (state.sel && !state.items.some((i) => i.name === state.sel)) state.sel = null;
    render();
    if (!state.loaded && !state.sel && state.items.length) await pick(state.items[0].name);
    state.loaded = true;
  }

  async function pick(name) {
    state.sel = name;
    render();
    const it = state.items.find((i) => i.name === name);
    if (!it || !it.check || state.checks.has(name)) return;
    const [check, report] = await Promise.all([
      hub.check(it.check).then((c) => ({ check: c.items[0] || null }), (e) => ({ error: e.message || String(e) })),
      it.report ? hub.report(it.report).catch(() => null) : null,
    ]);
    state.checks.set(name, check);
    state.reports.set(name, report);
    // the newest boot is the one a pull after a drop is usually for
    if (check.check && check.check.kind === 'pull' && check.check.boots.length) {
      state.open.set(name, new Set([Math.max(...check.check.boots.map((b) => b.n))]));
    }
    if (state.sel === name) render();
  }

  function sheetHtml(it) {
    if (!it) return '<div class="pane"><p class="empty">' + (state.items.length ? 'Pick a pull or a log.' : 'No pulls or logs in the folder yet.') + '</p></div>';
    if (!it.check) return '<div class="pane sheet-head an-head"><div class="top"><span class="id">' + esc(it.title) + '</span>' + mark('warn', 'no dump.txt') + '</div>' +
      '<p class="line">Pull of ' + esc(it.at) + '. This pull folder has no dump.txt, so nothing came off the board.</p></div>';
    const got = state.checks.get(it.name);
    if (!got) return '<div class="pane"><p class="empty">Reading ' + esc(it.check) + '…</p></div>';
    if (got.error) return '<div class="pane"><p class="empty">Could not read ' + esc(it.check) + ': ' + esc(got.error) + '</p></div>';
    if (!got.check) return '<div class="pane"><p class="empty">The app found no pull or log in ' + esc(it.check) + '.</p></div>';
    const o = { item: it, report: state.reports.get(it.name), open: state.open.get(it.name) || new Set(), now: new Date() };
    return got.check.kind === 'pull' ? pullHtml(got.check, o) : logHtml(got.check, o);
  }

  function render() {
    paint($('an-list'), state.failed ? '<p class="empty">' + esc(state.failed) + '</p>'
      : state.items.length ? state.items.map((i) => itemHtml(i, state.sel)).join('') : '<p class="empty">No pulls or logs yet.</p>');
    paint($('an-sheet'), sheetHtml(state.items.find((i) => i.name === state.sel)));
  }

  $('an-look').addEventListener('click', async () => {
    state.checks.clear();
    await load();
    if (state.sel) await pick(state.sel);
    const n = state.items.length;
    A.ui.toast(state.failed || fmt.format(n) + (n === 1 ? ' pull or log' : ' pulls and logs'));
  });

  $('screen-analysis').addEventListener('click', (e) => {
    const at = (name) => e.target.getAttribute(name);
    const button = e.target.closest('[data-pick]');
    const picked = at('data-pick') || (button && button.getAttribute('data-pick'));
    if (picked) { pick(picked); return; }
    if (at('data-open')) {
      const it = state.items.find((i) => i.name === at('data-open'));
      const kept = D.keptIn(hub.log);
      const sep = kept.includes('\\') ? '\\' : '/';
      A.openServerLog(it.log, it.kind === 'pull' ? { raw: it.name + '/dump.txt', dir: kept ? kept + sep + it.name : null } : {});
      return;
    }
    const row = e.target.closest('tr[data-boot]');
    const boot = at('data-boot') || (row && row.getAttribute('data-boot'));
    if (boot !== null && state.sel) {
      const open = state.open.get(state.sel) || new Set();
      const n = Number(boot);
      if (open.has(n)) open.delete(n); else open.add(n);
      state.open.set(state.sel, open);
      render();
    }
  });

  A.shell.register('analysis', { show: load });
})();
