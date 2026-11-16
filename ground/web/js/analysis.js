// The Analysis screen: a pulled flight log, or a session log, as the app reads it (/api/check, the
// same reading as `descent-ground check`), boot by boot. What it shows is the firmware's own codes,
// flags and counts and the values it logged. Nothing here says whether a value looks right, and a
// phase is only ever what a phase event said. The words and markup are pure and sit first, so the
// tests can load them in node; the screen below needs the page and the app.
(function () {
  'use strict';
  const D = typeof module === 'object' && module.exports ? require('./datamodel.js') : DG.datamodel;
  const F = typeof module === 'object' && module.exports ? require('./flight.js') : DG.flight;
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

  // How long a boot ran on its own clock, as far as the log shows: to its last record, or to its
  // last event where one came after
  function bootLength(b) {
    return Math.max(1, b.lastMs, ...b.phases.map((s) => s.toMs), ...b.events.map((e) => e.lastMs));
  }

  const phaseName = (s) => s.phase + (s.heldFix ? ', held fix' : '');

  // One boot's stretch of the merged phase bar: a segment per phase event, sized by uptime. imufast
  // and imuslow change the IMU's rate and not the phase, so they are ticks on it.
  function laneCell(b, at) {
    if (!b.phases.length) return '<span class="nophase" title="No phase events in boot ' + b.n + '"></span>';
    const segs = b.phases.map((s) => '<span class="seg' + (s.heldFix ? ' held' : '') + '" style="left:' + at(s.fromMs) + ';width:' +
      at(s.toMs - s.fromMs) + '" title="' + esc('Boot ' + b.n + ', ' + phaseName(s) + ', ' + span(s.fromMs, s.toMs)) + '"><span>' + esc(phaseName(s)) + '</span></span>');
    const ticks = b.events.filter((e) => e.source === 'phase' && (e.what === 'imufast' || e.what === 'imuslow'))
      .map((e) => '<span class="tick" style="left:' + at(e.firstMs) + '" title="' + esc(e.what + ', ' + span(e.firstMs, e.lastMs)) + '"></span>');
    return segs.join('') + ticks.join('');
  }

  // One validity bit over one boot, on the phase bar's axis. Before the first record and after the
  // last there are no records, which is not the same as the flag being set, so it is drawn apart.
  function stripCell(b, s, at) {
    if (!b.records) return '<span class="norec" style="left:0;width:100%" title="No records in boot ' + b.n + '"></span>';
    const off = (from, ms) => '<span class="off" style="left:' + at(from) + ';width:max(2px,' + at(ms) + ')"></span>';
    const spells = s.clear === s.records ? off(b.firstMs, b.lastMs - b.firstMs) : s.longest.map((p) => off(p.fromMs, p.ms)).join('');
    return '<span class="norec" style="left:0;width:' + at(b.firstMs) + '"></span>' + spells +
      '<span class="norec" style="left:' + at(b.lastMs) + ';width:' + at(bootLength(b) - b.lastMs) + '"></span>';
  }

  function imuFirstWords(b) {
    if (!b.records) return 'no records';
    return b.imuFirstValidMs === null ? 'IMU never valid in this boot' : 'IMU first valid at ' + up(b.imuFirstValidMs);
  }

  const sensorOf = (b, name) => b.sensors.find((s) => s.name === name);

  function gpsFirstWords(b) {
    const ms = sensorOf(b, 'gps').firstValidMs;
    return ms === null ? 'GPS never had a fix in this boot' : 'GPS first fix at ' + up(ms);
  }

  // One value, or the lowest to the highest where the boots differ
  function spread(values) {
    if (!values.length) return null;
    const lo = Math.min(...values);
    const hi = Math.max(...values);
    return lo === hi ? secs(lo) : secs(lo) + ' to ' + secs(hi);
  }

  // The middle step between records, from each boot's spacing, a line per phase only where the
  // phases differ
  function spacingHtml(list) {
    const parts = [['air', 'in the air'], ['ground', 'on the ground'], ['none', 'with no phase events']]
      .map(([k, where]) => [spread(list.map((r) => r[k]).filter((v) => v !== null)), where])
      .filter((p) => p[0]);
    if (!parts.length) return '—';
    if (parts.every((p) => p[0] === parts[0][0])) return esc(parts[0][0]);
    return parts.map((p) => esc(p[0]) + ' <span class="note">' + p[1] + '</span>').join('<br>');
  }

  // 0, 1, 3, 4, 5 as 0, 1, 3–5
  function numbers(ns) {
    const out = [];
    for (const n of ns) {
      const last = out[out.length - 1];
      if (last && n === last[1] + 1) last[1] = n; else out.push([n, n]);
    }
    return out.map(([a, z]) => (a === z ? a : z === a + 1 ? a + ', ' + z : a + '–' + z)).join(', ');
  }

  // Which boots hold a kind of data and over what uptime. Each boot is named while there are few.
  function whereHtml(spans, of) {
    if (!spans.length) return '<span class="note">none</span>';
    const each = spans.map((s) => 'boot ' + s.n + ' ' + span(s.from, s.to));
    if (spans.length <= 3) return each.map(esc).join('<br>');
    return '<span title="' + esc(each.join('; ')) + '">' + spans.length + ' of ' + plural(of, 'boot') + ': ' + numbers(spans.map((s) => s.n)) + '</span>';
  }

  const shown = (v, unit) => String(+v.toFixed(7)) + (unit ? ' ' + unit : '');
  const extremeHtml = (x, unit) => (x ? '<b>' + esc(shown(x.v, unit)) + '</b><br><span class="note">boot ' + x.n + ', ' + esc(up(x.ms)) + '</span>' : '—');
  const tally = (pairs) => {
    const counts = new Map();
    for (const [k, n] of pairs) counts.set(k, (counts.get(k) || 0) + n);
    return [...counts].map(([k, n]) => k + ' ×' + fmt.format(n)).join(', ');
  };

  // The packet's fields as the inventory lists them, by the part that measures them
  const FIELDS = ['accel', 'gyro', 'mag', 'quat', 'pressure', 'temp', 'humidity', 'envAlt', 'lat', 'lon', 'gpsAlt', 'battery', 'counter'];
  const LENGTHS = ['accel', 'gyro', 'mag'];

  function fieldRow(key, on) {
    const withRecords = on.filter((b) => b.records);
    if (key === 'quat') {
      const s = withRecords.map((b) => sensorOf(b, 'quat'));
      return { what: 'Orientation quaternion, i j k real', flag: 'quat', count: fmt.format(s.reduce((n, x) => n + x.records - x.clear, 0)) + ' of ' + fmt.format(s.reduce((n, x) => n + x.records, 0)),
        spacing: spacingHtml(s.map((x) => x.spacing)), spans: s.map((x, i) => ({ n: withRecords[i].n, from: x.firstValidMs, to: x.lastValidMs })).filter((x) => x.from !== null) };
    }
    const rs = withRecords.map((b) => [b, b.ranges.find((r) => r.key === key)]);
    const r0 = rs.length ? rs[0][1] : null;
    let lo = null;
    let hi = null;
    for (const [b, r] of rs) {
      if (r.min !== null && (!lo || r.min < lo.v)) lo = { v: r.min, n: b.n, ms: r.minMs };
      if (r.max !== null && (!hi || r.max > hi.v)) hi = { v: r.max, n: b.n, ms: r.maxMs };
    }
    const set = rs.reduce((n, [, r]) => n + r.set + r.heldFix, 0);
    const all = rs.reduce((n, [, r]) => n + r.set + r.clear + r.heldFix, 0);
    const held = rs.reduce((n, [, r]) => n + r.heldFix, 0);
    const flag = r0 ? r0.flag : null;
    const words = r0 ? r0.words.charAt(0).toUpperCase() + r0.words.slice(1) + (LENGTHS.includes(key) ? ', length of x y z' : '') : key;
    const spans = flag
      ? rs.map(([b]) => ({ n: b.n, from: sensorOf(b, flag).firstValidMs, to: sensorOf(b, flag).lastValidMs })).filter((x) => x.from !== null)
      : rs.map(([b]) => ({ n: b.n, from: b.firstMs, to: b.lastMs }));
    return {
      what: words, flag, unit: r0 ? r0.unit : '', lo, hi, spans,
      count: (flag ? fmt.format(set) + ' of ' + fmt.format(all) : fmt.format(all)) + (held ? '<br><span class="note">' + fmt.format(held) + ' on a held fix, left out of lowest and highest</span>' : ''),
      spacing: spacingHtml(rs.map(([b]) => (flag ? sensorOf(b, flag).spacing : b.recordSpacing))),
    };
  }

  // Every kind of data the chip keeps, over the boots ticked. Flash holds 64 byte records, each a
  // packet, a GPS time anchor or an event (fsw FlashLog.h); the settings come off the console.
  function inventoryHtml(check, on) {
    const middle = (ms) => { const steps = ms.slice(1).map((m, i) => m - ms[i]).sort((a, b) => a - b); return steps.length ? steps[Math.floor(steps.length / 2)] : null; };
    const rows = FIELDS.map((k) => fieldRow(k, on));
    const records = on.reduce((n, b) => n + (b.records ? b.sensors[0].records : 0), 0);
    const csids = [...new Set(on.flatMap((b) => b.csids))].sort((a, b) => a - b);
    rows.push({ what: 'Board id', flag: null, count: fmt.format(records), values: csids.length ? csids.join(', ') : 'none' });
    rows.push({ what: 'All-fresh flag', flag: 'fresh', count: fmt.format(on.reduce((n, b) => n + b.fresh, 0)) + ' of ' + fmt.format(records),
      values: 'set when every part was read afresh for that record' });
    const withAnchors = on.filter((b) => b.anchorMs.length);
    rows.push({
      what: 'GPS time anchors', flag: null, count: fmt.format(on.reduce((n, b) => n + b.anchorMs.length, 0)),
      spacing: esc(spread(withAnchors.map((b) => middle(b.anchorMs)).filter((v) => v !== null)) || '—'),
      spans: withAnchors.map((b) => ({ n: b.n, from: b.anchorMs[0], to: b.anchorMs[b.anchorMs.length - 1] })),
      values: fmt.format(on.reduce((n, b) => n + b.anchors, 0)) + ' used for the clock, ' + fmt.format(on.reduce((n, b) => n + b.anchorsRejected, 0)) + ' rejected',
    });
    const booted = on.filter((b) => b.resetCsr !== null);
    rows.push({ what: 'Boot events, with the reset cause', flag: null, count: fmt.format(booted.length) + ' of ' + plural(on.length, 'boot'),
      where: booted.length ? 'boot' + (booted.length === 1 ? ' ' : 's ') + numbers(booted.map((b) => b.n)) : null,
      values: booted.length ? tally(booted.map((b) => [b.reset, 1])) : 'none' });
    const sums = (phase) => on.map((b) => [b, b.systemEvents.filter((s) => (s.source === 'phase') === phase)]).filter(([, l]) => l.length);
    for (const [phase, what] of [[true, 'Phase events'], [false, 'System events: sensor, bus, radio, flash']]) {
      const got = sums(phase);
      const list = got.flatMap(([, l]) => l);
      rows.push({
        what, flag: null, count: fmt.format(list.reduce((n, s) => n + s.records, 0)) + '<br><span class="note">' + plural(list.reduce((n, s) => n + s.count, 0), 'time') + '</span>',
        spans: got.map(([b, l]) => ({ n: b.n, from: Math.min(...l.map((s) => s.firstMs)), to: Math.max(...l.map((s) => s.lastMs)) })),
        values: list.length ? tally(list.map((s) => [(phase ? '' : s.source + ' ') + s.what, s.count])) : 'none',
      });
    }
    const dropping = on.filter((b) => b.droppedEvents.length);
    rows.push({ what: 'Dropped-record events', flag: null, count: fmt.format(dropping.reduce((n, b) => n + b.droppedEvents.length, 0)),
      spans: dropping.map((b) => ({ n: b.n, from: b.droppedEvents[0].ms, to: b.droppedEvents[b.droppedEvents.length - 1].ms })),
      values: plural(on.reduce((n, b) => n + b.dropped, 0), 'record') + ' dropped, as the boots counted them' });
    const unread = on.flatMap((b) => b.otherEvents.map((e) => ['kind 0x' + e.kind.toString(16).toUpperCase().padStart(2, '0'), 1])
      .concat(b.systemRaw.map((e) => ['system layout ' + e.layout, 1])));
    if (unread.length || check.lockEvents) {
      rows.push({ what: 'Events this app does not read', flag: null, count: fmt.format(unread.length + check.lockEvents),
        values: tally(unread.concat(check.lockEvents ? [['lock events from the older ring, all boots', check.lockEvents]] : [])) });
    }
    const set = check.consoles.slice().reverse().find((s) => s.settings);
    rows.push({ what: 'Settings', flag: null, count: set ? '1 line' : '0', where: 'the restart for the pull',
      values: set ? '<span class="hex">' + esc(set.settings) + '</span>' : 'no settings line in this pull', raw: true });

    const html = rows.map((r) => '<tr><td>' + esc(r.what) + '</td><td>' + (r.flag ? esc(r.flag) : '—') + '</td><td class="r">' + r.count + '</td><td class="wrap">' + (r.spacing || '—') +
      '</td><td class="wrap">' + (r.where ? esc(r.where) : r.spans ? whereHtml(r.spans, on.length) : '—') + '</td>' +
      (r.values !== undefined ? '<td colspan="2" class="wrap">' + (r.raw ? r.values : esc(r.values)) + '</td>' : '<td>' + extremeHtml(r.lo, r.unit) + '</td><td>' + extremeHtml(r.hi, r.unit) + '</td>') + '</tr>');
    const bad = on.reduce((n, b) => n + b.badPacketCrc, 0);
    return '<div class="pane an-inv"><h3>What this pull holds<span class="note">over the boots ticked under Phases and sensors: ' +
      (on.length ? 'boot' + (on.length === 1 ? ' ' : 's ') + numbers(on.map((b) => b.n)) : 'none') + '. Values as logged, with the firmware\'s own flags.</span></h3>' +
      (on.length ? table(['Data', 'Flag', { t: 'Records', r: true }, 'Spacing', 'Boots and uptime', 'Lowest', 'Highest'], html, 'inv') : '<p class="empty">No boot ticked.</p>') +
      '<div class="an-part"><p class="note">Records counts the packet records with the flag set, of those whose packet passed its CRC' + (bad ? ' (' + plural(bad, 'record') + ' failed it and are left out)' : '') +
        '. Spacing is the middle step between those records. Every record carries each part\'s latest reading, so it is how often the chip logged, not how often the part measured. ' +
        'Uptime is from the boot\'s start.</p>' +
        '<p class="note">Not in the record: the satellite count, the fix type, GPS speed and heading, and the state of charge in percent. The GPS flag is set on a 3D fix whose position the receiver did not mark invalid; the battery byte is the cell voltage.</p></div></div>';
  }

  // One chart's points over the boots ticked. cells: [{b, left, width}], each boot's stretch of the
  // merged bar in px; c: {flag, keys, length}. Only records with the flag set are drawn: a record
  // with it clear, a gap between records and the end of a boot each break the line. Where a pixel
  // holds more than two records it keeps each series' lowest and highest, in the order they came,
  // so a spike in a 14,000 record boot still shows.
  function chartPoints(cells, c) {
    const x = [];
    const ys = (c.length ? [0] : c.keys).map(() => []);
    const at = [];
    const push = (px, v, where) => { x.push(px); v.forEach((y, j) => ys[j].push(y)); at.push(where); };
    const cut = (px) => { if (x.length && ys[0][ys[0].length - 1] !== null) push(px, ys.map(() => null), null); };
    for (const { b, left, width } of cells) {
      const t = b.track;
      const len = bootLength(b);
      const gaps = b.gaps.map((g) => g.afterMs).sort((p, q) => p - q);
      let g = 0;
      let run = null;
      let last = left;
      const flush = () => {
        if (!run) return;
        if (run.length <= 2) {
          for (const p of run) push(p.x, p.v, p.at);
        } else {
          const ends = ys.map((_, j) => {
            let lo = 0;
            let hi = 0;
            run.forEach((p, i) => { if (p.v[j] < run[lo].v[j]) lo = i; if (p.v[j] > run[hi].v[j]) hi = i; });
            return lo < hi ? [lo, hi] : [hi, lo];
          });
          for (const k of [0, 1]) push(run[ends[0][k]].x, ends.map((e, j) => run[e[k]].v[j]), run[ends[0][k]].at);
        }
        run = null;
      };
      for (let i = 0; t && i < t.ms.length; i++) {
        const ms = t.ms[i];
        const px = Math.max(last, left + width * ms / len);
        last = px;
        for (; g < gaps.length && gaps[g] < ms; g++) {
          if (i > 0 && gaps[g] >= t.ms[i - 1]) { flush(); cut(px); }
        }
        if (!((t.valid[i] >> c.flag) & 1) || c.keys.some((k) => t[k][i] === null)) { flush(); cut(px); continue; }
        const v = c.length ? [Math.hypot(...c.keys.map((k) => t[k][i]))] : c.keys.map((k) => t[k][i]);
        if (run && Math.floor(run[0].x) !== Math.floor(px)) flush();
        (run = run || []).push({ x: px, v, at: [b.n, ms] });
      }
      flush();
      cut(left + width);
    }
    return { x, ys, at };
  }

  // The boots ticked, laid end to end in chip order, each as wide as it ran, with the sensors'
  // flags under them on the same axis. Boots share no clock without GPS time, so the time between
  // two is not drawn, only the line where one ends and the next begins.
  function mergedHtml(boots, picked) {
    const picks = '<div class="an-picks" role="group" aria-label="Boots in the bar">' +
      boots.map((b) => '<label><input type="checkbox" data-pickboot="' + b.n + '"' + (picked.has(b.n) ? ' checked' : '') + '> Boot ' + b.n + '</label>').join('') +
      '<button data-pickall="1">All</button><button data-pickall="0">None</button></div>';
    const on = boots.filter((b) => picked.has(b.n));
    const head = '<h3>Phases and sensors<span class="note">over the boots ticked, in chip order, each as wide as it ran. Boots share no clock without GPS time, ' +
      'so the time between two boots is not drawn: a line marks where one ended and the next began.</span></h3>';
    if (!on.length) return '<div class="pane an-merged">' + head + picks + '<p class="empty">No boot ticked.</p></div>';

    const row = (cls, label, draw) => '<div class="mrow ' + cls + '"><b>' + label + '</b><span class="mbar">' +
      on.map((b) => '<span class="mcell" data-mboot="' + b.n + '" style="flex-grow:' + bootLength(b) + '">' +
        draw(b, (ms) => (100 * ms / bootLength(b)).toFixed(2) + '%') + '</span>').join('') + '</span></div>';
    const ends = row('mhead', '', (b) => {
      const words = 'Boot ' + b.n + ' · ' + endWords(b);
      return '<span title="' + esc(words) + '">' + esc(words) + '</span>';
    });
    const lane = row('mlane', 'Phase', laneCell);
    const names = (on.find((b) => b.sensors.length) || { sensors: [] }).sensors.map((s) => s.name);
    const strips = names.map((name) => row('mstrip', esc(name), (b, at) => stripCell(b, b.sensors.find((s) => s.name === name), at)));
    const notDrawn = names.map((name) => {
      const n = on.reduce((sum, b) => {
        const s = b.sensors.find((x) => x.name === name);
        return sum + (s && s.clear < s.records ? s.spells - s.longest.length : 0);
      }, 0);
      return n ? name + ' ' + fmt.format(n) : '';
    }).filter(Boolean);

    const firsts = on.map((b) => '<li><b>Boot ' + b.n + '</b> ' + esc(imuFirstWords(b)) + (b.records ? ' · ' + esc(gpsFirstWords(b)) : '') + '</li>');
    const thinned = on.filter((b) => b.track && b.track.every > 1).map((b) => 'boot ' + b.n + ' one record in ' + b.track.every);
    const said = on.flatMap((b) => {
      const tag = '<span class="bootno">Boot ' + b.n + '</span> ';
      if (!b.phases.length) return ['<li>' + tag + '<span class="note">No phase events in this boot. Firmware from before 2026-11-06 wrote none.</span></li>'];
      return b.phases.map((s) => {
        const why = !s.by ? 'Every boot starts in boot-air.'
          : (s.reasonMeaning ? s.reasonMeaning.charAt(0).toUpperCase() + s.reasonMeaning.slice(1) + '.' : '') + (s.count > 1 ? ' ' + s.count + ' times from here.' : '');
        const code = s.by ? ' <code>' + esc(s.by + (s.reason && s.reason !== 'none' ? ', ' + s.reason : '')) + '</code>' : '';
        return '<li>' + tag + '<b>' + esc(phaseName(s)) + '</b> ' + esc(span(s.fromMs, s.toMs)) + ' <span class="note">' + esc(why) + '</span>' + code + '</li>';
      });
    });
    return '<div class="pane an-merged">' + head + picks +
      '<div class="an-part"><div class="mgrid" role="img" aria-label="Phases from the phase events, and each sensor\'s valid flag, filled where it was set">' +
        ends + lane + strips.join('') + '</div>' +
        '<div class="an-charts" aria-label="Each logged quantity over the same boots"></div>' +
        '<p class="note">Sensor strips are filled where the firmware\'s valid flag was set, empty where it was clear, hatched where the boot has no records.' +
          (notDrawn.length ? ' Spells not drawn, the shortest: ' + esc(notDrawn.join(', ')) + '.' : '') + '</p>' +
        '<p class="note">Charts draw only records with the quantity\'s flag set, on the same axis as the bar. A break in a line is the flag clear, a gap between records or the end of a boot. ' +
          'Where a pixel holds several records it shows their lowest and highest.' + (thinned.length ? ' Long boots are drawn from part of their records: ' + esc(thinned.join(', ')) + '.' : '') + '</p>' +
        '<ul class="firsts">' + firsts.join('') + '</ul>' +
        '<p class="note">IMU valid is the first record with all four IMU flags set: accel, gyro, mag and quat. The GPS fix is the first record with the gps flag set, a 3D fix. Uptime is from the boot\'s start.</p></div>' +
      '<div class="an-part"><h4>Phases</h4><ol class="phaselist">' + said.join('') + '</ol></div>' +
      '<div class="an-part"><h4>System events</h4>' + eventsHtml(on) + '</div></div>';
  }

  function eventsHtml(boots) {
    const rows = boots.flatMap((b) => b.events.map((e) => {
      const code = e.source + ' ' + e.what + (e.part !== 'none' ? ' on ' + e.part : '') + (e.reason !== 'none' ? ', ' + e.reason : '');
      const why = e.reasonMeaning ? '<br><span class="note">Why: ' + esc(e.reasonMeaning) + '.</span>' : '';
      const detail = e.detailWords ? esc(e.detailWords) + (e.count > 1 ? ' <span class="note">the last time</span>' : '') : '';
      return '<tr><td class="r">' + b.n + '</td><td>' + esc(span(e.firstMs, e.lastMs)) + '</td><td class="said"><code>' + esc(code) + '</code><br>' +
        esc(e.meaning) + why + '</td><td class="r">' + fmt.format(e.count) + '</td><td>' + detail + '</td></tr>';
    }));
    const left = boots.filter((b) => b.eventsLeftOut)
      .map((b) => '<p class="note">Boot ' + b.n + ': and ' + plural(b.eventsLeftOut, 'later run') + ' not listed, the app sends the first 200 of a boot.</p>');
    if (!rows.length) return '<p class="note">No system events in the boots ticked.</p>';
    return table([{ t: 'Boot', r: true }, 'When', 'What happened', { t: 'Times', r: true }, 'Detail'], rows, 'events') + left.join('');
  }

  // The firmware's valid bit, record by record, over the boot's records. Filled is set, empty is clear.
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

  // What the flight picker offers: each boot with a track, whole, then each of its air spells,
  // newest boot first. The first air spell of the newest boot that has one is the default.
  function flightChoices(check) {
    const boots = (check.boots || []).filter((b) => b.track && b.track.ms.length).sort((a, b) => b.n - a.n);
    const list = boots.flatMap((b) => F.flightsOf(b).map((f) => Object.assign(f, { value: b.n + ':' + (f.spell || 0) })));
    const first = list.find((f) => f.kind === 'air') || list[0] || null;
    return { list, first };
  }

  // o: {item, report, open: Set of boot numbers, picked: Set of the boots in the merged bar (all
  // of them when not given), now}
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
    const picked = o.picked || new Set(boots.map((b) => b.n));
    return head + inventoryHtml(check, boots.filter((b) => picked.has(b.n))) +
      '<div class="pane an-boots"><h3>Boots<span class="note">in chip order. Each one ended with the next one\'s reset cause; the last ended with the restart for the pull.' +
        (o.item.log ? ' In Replay a boot\'s records come in as the receiver flash-boot and its number.' : '') + '</span></h3>' +
        bootTableHtml(boots, o.open) + '</div>' +
      mergedHtml(boots, picked) +
      (flightChoices(check).list.length ? '<div class="an-flightslot"></div>' : '') +
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
      '</td><td>' + esc(r.sf ? 'SF' + r.sf : (r.bySf || []).map((s) => 'SF' + s.sf).join(' ') || '—') + '</td><td class="r">' + fmt.format(r.packets) + '</td><td class="r">' + fmt.format(r.badCrc) +
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

  const pure = { itemsOf, itemHtml, pullHtml, logHtml, flightChoices, chartPoints };
  if (typeof module === 'object' && module.exports) module.exports = pure;
  if (typeof window === 'undefined') return;

  const A = window.DGApp;
  const { $ } = A.ui;
  const { paint } = A.shell;
  const hub = A.hub;
  if (!hub.native) return;

  // checks: name -> {check} | {error}, asked once a pick. open: name -> Set of opened boot numbers.
  // picked: name -> Set of the boots in the merged bar
  const state = { items: [], sel: null, failed: '', checks: new Map(), reports: new Map(), open: new Map(), picked: new Map(), loaded: false };

  // The flight view is one pane kept across repaints, since the sheet is markup and a map is not:
  // after each paint it goes back into the sheet's slot. Its clock is the boot's uptime in ms.
  const fl = { pane: null, view: null, check: null, choices: null, cur: null, boot: null, samples: [], all: [], ms: 0, timer: null, speed: 1 };

  function flightPane() {
    if (fl.pane) return fl.pane;
    const p = document.createElement('section');
    p.className = 'pane an-flight';
    p.innerHTML = '<div class="an-fhead"><h3>Flight</h3><select class="an-fpick" aria-label="Which flight"></select></div>' +
      '<p class="note an-fnote"></p><div class="an-fview"></div>' +
      '<div class="an-time"><button class="primary" data-play>Play</button>' +
      '<span class="seg" role="group" aria-label="Speed"><button data-speed="0.25">0.25×</button><button data-speed="1">1×</button><button data-speed="4">4×</button></span>' +
      '<input type="range" class="an-fpos" min="0" max="0" step="50" value="0" aria-label="Time in the flight"><span class="an-fwhen"></span>' +
      '<button data-jump="release">Release</button><button data-jump="touchdown">Touchdown</button></div>' +
      '<div class="an-freadout"></div>';
    fl.pane = p;
    fl.view = new DG.FlightView(p.querySelector('.an-fview'), { live: false, toast: A.ui.toast, onDraw: () => fl.cur && setTime(fl.ms) });
    p.querySelector('.an-fpick').addEventListener('change', (e) => chooseFlight(e.target.value));
    p.querySelector('.an-fpos').addEventListener('input', (e) => setTime(Number(e.target.value)));
    p.addEventListener('click', (e) => {
      const b = e.target.closest('button');
      if (!b) return;
      if (b.dataset.play !== undefined) play();
      else if (b.dataset.speed) { fl.speed = Number(b.dataset.speed); markSpeed(); }
      else if (b.dataset.jump) {
        const j = fl.jumps[b.dataset.jump];
        if (j) { setTime(j.ms); A.ui.toast((b.dataset.jump === 'release' ? 'Release' : 'Touchdown') + ' at ' + up(j.ms) + ', ' + j.by); }
      }
    });
    markSpeed();
    return p;
  }

  function markSpeed() {
    for (const b of fl.pane.querySelectorAll('[data-speed]')) b.classList.toggle('on', Number(b.dataset.speed) === fl.speed);
  }

  function loadFlights(check) {
    fl.check = check;
    fl.choices = flightChoices(check);
    const pick = fl.pane.querySelector('.an-fpick');
    pick.innerHTML = fl.choices.list.map((f) => '<option value="' + f.value + '">' + esc(f.label) + '</option>').join('');
    if (fl.choices.first) { pick.value = fl.choices.first.value; chooseFlight(fl.choices.first.value); }
  }

  // The firmware's phase events say where release and touchdown are; where a flight has none,
  // the IMU rule from find_landing.py does, and each says which it is.
  function chooseFlight(value) {
    stop();
    const f = fl.choices.list.find((x) => x.value === value);
    const b = fl.check.boots.find((x) => x.n === f.n);
    fl.cur = f;
    fl.boot = b;
    fl.all = F.fromTrack(b.track, b.phases.filter((s) => s.heldFix).map((s) => [s.fromMs, s.toMs]));
    fl.samples = fl.all.filter((s) => s.ms >= f.fromMs && s.ms <= f.toMs);
    const td = F.touchdown(fl.samples);
    const rel = td ? F.release(fl.samples, td) : null;
    fl.jumps = {
      release: f.releaseMs !== null && f.releaseMs !== undefined ? { ms: f.releaseMs, by: 'the firmware\'s falling event' }
        : rel ? { ms: rel.ms, by: 'derived: the first turn at half the peak rotation in the 30 s before the derived touchdown' } : null,
      touchdown: f.landedMs !== null && f.landedMs !== undefined ? { ms: f.landedMs, by: 'the firmware\'s ' + f.landedPhase + ' event' }
        : td ? { ms: td.ms, by: 'derived: the IMU still for ' + F.STILL_FOR_S + ' s' } : null,
    };
    const said = [];
    if (f.kind === 'air') {
      said.push('Phase events: in the air ' + up(f.toAirMs) + (f.releaseMs !== null ? ', falling ' + up(f.releaseMs) : '') +
        (f.landedMs !== null ? ', ' + f.landedPhase + ' ' + up(f.landedMs) : ', no landing in the log') + '.');
    }
    said.push(td ? 'Derived from the IMU: still from ' + up(td.ms) + ' (under ' + F.STILL_ACCEL_MPS2 + ' m/s² and ' + F.STILL_GYRO_DPS + ' deg/s for ' + F.STILL_FOR_S + ' s after moving).'
      : 'Derived from the IMU: no 20 s still after moving in this window.');
    if (b.track.every > 1) said.push('Every ' + b.track.every + 'th record: this boot has more than the track keeps.');
    const fixes = fl.samples.filter(F.isFix).length;
    said.push(fixes ? fixes + ' records with a GPS fix in this window.' : 'No record with a GPS fix in this window, so nothing to place on the map.');
    fl.pane.querySelector('.an-fnote').textContent = said.join(' ');
    for (const k of ['release', 'touchdown']) fl.pane.querySelector('[data-jump="' + k + '"]').disabled = !fl.jumps[k];
    // the clock runs over the records the window holds, so it opens on one: on the ground the
    // firmware logs every few seconds, and the window's edge can fall between two
    fl.from = fl.samples.length ? fl.samples[0].ms : f.fromMs;
    fl.to = fl.samples.length ? fl.samples[fl.samples.length - 1].ms : f.toMs;
    const pos = fl.pane.querySelector('.an-fpos');
    pos.min = fl.from;
    pos.max = fl.to;
    // the boot's own pane, so its flags carry the cursor as well as the merged bar
    const open = state.open.get(state.sel) || new Set();
    if (!open.has(b.n)) { open.add(b.n); state.open.set(state.sel, open); render(); }
    fl.view.show([{ key: 'boot' + b.n + ':' + f.value, name: 'Boot ' + b.n, index: 0, samples: fl.samples, base: fl.all, gaps: F.trackGaps(fl.samples, b.gaps), dim: false, sel: true, card: null }]);
    setTime(f.kind === 'air' ? f.toAirMs - 30000 : fl.from);
  }

  function setTime(ms) {
    fl.ms = Math.max(fl.from, Math.min(fl.to, ms));
    const t = fl.ms / 1000;
    fl.view.setTime(t);
    fl.pane.querySelector('.an-fpos').value = fl.ms;
    const wall = fl.boot.startUtc ? new Date(Date.parse(fl.boot.startUtc) + fl.ms - fl.boot.firstMs) : null;
    const when = up(fl.ms) + (wall ? ', ' + wall.toTimeString().slice(0, 8) + '.' + String(wall.getMilliseconds()).padStart(3, '0').slice(0, 1) : ', no GPS time');
    fl.pane.querySelector('.an-fwhen').textContent = when;
    const r = F.readingsAt(fl.samples, F.heights(fl.all, fl.view.source), t);
    paint(fl.pane.querySelector('.an-freadout'), DG.flightmap.readoutHtml(r && Object.assign(r, { when })));
    placeCursor();
  }

  function stop() {
    clearInterval(fl.timer);
    fl.timer = null;
    if (fl.pane) fl.pane.querySelector('[data-play]').textContent = 'Play';
  }

  // Steps of a tenth of a second on the page's clock, scaled by the speed. The glyph shows the
  // record at or before each step, never one made up between two.
  function play() {
    if (fl.timer) { stop(); return; }
    if (!fl.cur) return;
    if (fl.ms >= fl.to) setTime(fl.from);
    fl.pane.querySelector('[data-play]').textContent = 'Pause';
    let last = performance.now();
    fl.timer = setInterval(() => {
      const now = performance.now();
      const next = fl.ms + (now - last) * fl.speed;
      last = now;
      setTime(next);
      if (next >= fl.to) stop();
    }, 100);
  }

  // A line at the cursor on the boot's stretch of the merged bar and strips, and on each flag's
  // strip in the boot's own pane
  function placeCursor() {
    const b = fl.boot;
    if (!b) return;
    const sheet = $('an-sheet');
    const box = sheet.querySelector('[data-bootpane="' + b.n + '"]');
    const holders = [...sheet.querySelectorAll('.an-merged .mlane [data-mboot="' + b.n + '"], .an-merged .mstrip [data-mboot="' + b.n + '"]')]
      .map((h) => [h, 100 * fl.ms / bootLength(b)])
      .concat(box ? [...box.querySelectorAll('.flag .strip')].map((h) => [h, 100 * (fl.ms - b.firstMs) / Math.max(1, b.lastMs - b.firstMs)]) : []);
    for (const [holder, pct] of holders) {
      let c = holder.querySelector('.an-cursor');
      if (!c) { c = document.createElement('span'); c.className = 'an-cursor'; holder.appendChild(c); }
      c.style.left = Math.max(0, Math.min(100, pct)).toFixed(2) + '%';
    }
    const along = Math.max(0, Math.min(1, fl.ms / bootLength(b)));
    for (const p of ch.plots) {
      const cell = p.cells.find((x) => x.b.n === b.n);
      let c = p.u.over.querySelector('.an-cursor');
      if (!c) { c = document.createElement('span'); c.className = 'an-cursor'; p.u.over.appendChild(c); }
      c.hidden = !cell;
      if (cell) c.style.left = p.u.valToPos(cell.left + cell.width * along, 'x') + 'px';
    }
  }

  // The charts under the strips, one per quantity, drawn in the merged bar's own pixels so each
  // boot lines up from the phase lane down. Built after a paint, like the flight pane, and again
  // when the bar's width or the theme changes.
  const ch = { holder: null, width: 0, plots: [], watch: new ResizeObserver(() => drawCharts()) };
  const FLAGS = ['accel', 'gyro', 'mag', 'quat', 'gps', 'soc', 'env'];
  // the Live tabs' panels by their first series, so a chart here is titled and scaled as there
  const CHARTED = [['ax', 0], ['gx', 1], ['mx', 2], ['qi', 3], ['envAlt', 6], ['pressure', 6], ['temp', 6], ['humidity', 6], ['gpsAlt', 4], ['lat', 4], ['lon', 4], ['battery', 5]];

  function drawCharts() {
    const holder = $('an-sheet').querySelector('.an-charts');
    if (!holder) { ch.holder = null; ch.plots = []; return; }
    const bar = holder.parentNode.querySelector('.mlane .mbar');
    const width = bar.clientWidth;
    if (!width || (holder === ch.holder && width === ch.width)) return;
    for (const p of ch.plots) p.u.destroy();
    holder.textContent = '';
    ch.plots = [];
    if (holder !== ch.holder) { ch.watch.disconnect(); ch.watch.observe(holder); }
    ch.holder = holder;
    ch.width = width;

    const boots = new Map(state.checks.get(state.sel).check.boots.map((b) => [b.n, b]));
    const barBox = bar.getBoundingClientRect();
    const axisW = Math.round(barBox.left - holder.getBoundingClientRect().left);
    const cells = [...bar.children].map((cell) => {
      const r = cell.getBoundingClientRect();
      return { b: boots.get(Number(cell.dataset.mboot)), left: r.left - barBox.left + cell.clientLeft, width: cell.clientWidth };
    });
    const edges = cells.slice(1).map((c) => c.left - 1);   // the middle of the 2 px line between two boots
    const panels = DG.TABS.flatMap((t) => t.panels || []).filter((p) => p.series);
    const { axis, grid, ink } = DG.Charts.look();
    const lineWidth = A.settings().lineWidth;

    for (const [key, flag] of CHARTED) {
      const panel = panels.find((p) => p.series[0].key === key);
      const length = ['ax', 'gx', 'mx'].includes(key);
      const keys = panel.series.map((s) => s.key);
      const pts = chartPoints(cells, { flag, keys, length });
      const box = document.createElement('div');
      box.className = 'chart';
      box.innerHTML = '<h3>' + esc(panel.title + (length ? ', length of x y z' : '')) + (panel.unit ? ' <span>' + esc(panel.unit) + '</span>' : '') +
        ' <span class="note">' + FLAGS[flag] + ' flag</span><span class="readout"></span></h3>';
      holder.appendChild(box);
      if (!pts.ys[0].some((v) => v !== null)) {
        box.insertAdjacentHTML('beforeend', '<div class="none">No record with the ' + FLAGS[flag] + ' flag set in the boots ticked</div>');
        continue;
      }
      const single = pts.ys.length === 1;
      const readout = box.querySelector('.readout');
      const dec = panel.dec === undefined ? 2 : panel.dec;
      const u = new uPlot({
        width: axisW + width, height: 110, padding: [6, 0, 4, 0],
        legend: { show: !single, live: true },
        cursor: { points: { size: 7 }, drag: { x: false, y: false } },
        scales: {
          x: { time: false, range: () => [0, width] },
          y: panel.span ? { range: (u, min, max) => [Math.min(panel.span[0], min), Math.max(panel.span[1], max)] } : {},
        },
        // as many decimals as the step between ticks needs, so 76.384 and 76.385 never both read 76.385
        axes: [{ show: false }, Object.assign({ size: axisW, grid, values: (u, ticks, ai, space, step) => ticks.map((v) => DG.Charts.number(v, Math.max(0, -Math.floor(Math.log10(step))))) }, axis)],
        series: [{}].concat(pts.ys.map((_, j) => Object.assign({
          label: single ? panel.title : panel.series[j].label, spanGaps: false,
          points: { show: pts.x.length < 40, size: 4, width: 1 },
          value: (u, v) => (v == null ? '—' : +v.toFixed(5)),
        }, DG.Charts.line(single, j, lineWidth)))),
        hooks: {
          draw: [(u) => {
            const { top, height } = u.bbox;
            u.ctx.save();
            u.ctx.fillStyle = ink;
            for (const e of edges) u.ctx.fillRect(u.valToPos(e, 'x', true) - devicePixelRatio, top, 2 * devicePixelRatio, height);
            u.ctx.restore();
          }],
          setCursor: [(u) => {
            const at = u.cursor.idx == null ? null : pts.at[u.cursor.idx];
            const v = at && single ? DG.Charts.number(pts.ys[0][u.cursor.idx], dec) + (panel.unit ? ' ' + panel.unit : '') + ' ' : '';
            readout.textContent = at ? v : '';
            if (at) readout.insertAdjacentHTML('beforeend', '<small>boot ' + at[0] + ', ' + esc(up(at[1])) + '</small>');
          }],
        },
      }, [pts.x].concat(pts.ys), box);
      ch.plots.push({ u, cells });
    }
    placeCursor();
  }

  $('btn-theme').addEventListener('click', () => { ch.width = 0; drawCharts(); });

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
      state.picked.set(name, new Set(check.check.boots.map((b) => b.n)));
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
    const o = { item: it, report: state.reports.get(it.name), open: state.open.get(it.name) || new Set(), picked: state.picked.get(it.name), now: new Date() };
    return got.check.kind === 'pull' ? pullHtml(got.check, o) : logHtml(got.check, o);
  }

  function render() {
    paint($('an-list'), state.failed ? '<p class="empty">' + esc(state.failed) + '</p>'
      : state.items.length ? state.items.map((i) => itemHtml(i, state.sel)).join('') : '<p class="empty">No pulls or logs yet.</p>');
    const it = state.items.find((i) => i.name === state.sel);
    const got = it && state.checks.get(it.name);
    // a new pull's flights first: choosing one opens its boot, which renders again
    if (got && got.check && got.check !== fl.check && flightChoices(got.check).list.length) {
      flightPane();
      loadFlights(got.check);
    }
    paint($('an-sheet'), sheetHtml(it));
    drawCharts();
    const slot = $('an-sheet').querySelector('.an-flightslot');
    if (!slot) { stop(); return; }
    if (fl.pane.parentNode !== slot) slot.appendChild(fl.pane);
    placeCursor();
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
    const ticked = at('data-pickboot');
    const all = at('data-pickall');
    if (ticked !== null || all !== null) {
      const picked = state.picked.get(state.sel);
      if (all === '1') for (const b of state.checks.get(state.sel).check.boots) picked.add(b.n);
      else if (all === '0') picked.clear();
      else if (!picked.delete(Number(ticked))) picked.add(Number(ticked));
      render();
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
