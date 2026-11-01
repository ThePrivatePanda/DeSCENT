// What the Data screen says about the files the app has written: one row per file from
// /api/logs, and a pull's report.json as a boot table and a transfer summary.
// Words only, no page, so it can be tested.
(function (root, factory) {
  const mod = factory();
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).datamodel = mod;
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const two = (n) => String(n).padStart(2, '0');
  const count = (n) => Number(n).toLocaleString('en-US');

  function duration(ms) {
    if (!Number.isFinite(ms) || ms < 0) return '—';
    if (ms < 1000) return Math.round(ms) + ' ms';
    const s = Math.floor(ms / 1000);
    if (s < 60) return s + ' s';
    if (s < 3600) return Math.floor(s / 60) + ' min' + (s % 60 ? ' ' + (s % 60) + ' s' : '');
    const min = Math.floor((s % 3600) / 60);
    return Math.floor(s / 3600) + ' h' + (min ? ' ' + min + ' min' : '');
  }

  // How fast the board's clock ran against GPS. Parts per million in the report, said
  // as a percentage because that is how a crystal's error is read off a datasheet.
  function clockWords(ppm) {
    if (ppm === null || ppm === undefined || !Number.isFinite(Number(ppm))) return '—';
    const v = Number(ppm);
    if (v === 0) return 'on time';
    const way = v > 0 ? ' fast' : ' slow';
    const pct = Math.abs(v) / 10000;
    if (pct < 0.01) return Math.round(Math.abs(v)) + ' ppm' + way;
    return String(Number(pct.toFixed(2))) + '%' + way;
  }

  function bytes(n) {
    if (!Number.isFinite(n)) return '—';
    return n >= 1048576 ? (n / 1048576).toFixed(1) + ' MB'
      : n >= 1024 ? Math.round(n / 1024) + ' kB' : n + ' B';
  }

  function day(d, now) {
    if (Number.isNaN(d.getTime())) return '—';
    const year = now && d.getFullYear() === now.getFullYear() ? '' : ' ' + d.getFullYear();
    return d.getDate() + ' ' + MONTHS[d.getMonth()] + year;
  }

  // Local time: these are files on this laptop, dated by its clock.
  function dayTime(d, now) {
    if (Number.isNaN(d.getTime())) return '—';
    return day(d, now) + ', ' + two(d.getHours()) + ':' + two(d.getMinutes());
  }

  function utcStamp(ms) {
    const d = new Date(ms);
    if (Number.isNaN(d.getTime())) return '';
    return d.getUTCDate() + ' ' + MONTHS[d.getUTCMonth()] + ' ' + d.getUTCFullYear() + ' ' +
      two(d.getUTCHours()) + ':' + two(d.getUTCMinutes()) + ':' + two(d.getUTCSeconds()) + ' UTC';
  }

  // When the running app was built, from /api/health. Every build carries the same
  // version number, so this is the only thing that tells one from the next. An older
  // build does not say.
  function builtWords(health, now) {
    const at = health ? Number(health.built) : NaN;
    return at > 0 ? 'Built ' + dayTime(new Date(at * 1000), now) : '';
  }

  // The folder the app writes into, from the path of the log it is writing now.
  function keptIn(logPath) {
    const p = logPath ? String(logPath) : '';
    const cut = Math.max(p.lastIndexOf('/'), p.lastIndexOf('\\'));
    const dir = cut > 0 ? p.slice(0, cut) : '';
    // <data>/<day>/logs
    return dir.replace(/[\\/]\d{4}-\d{2}-\d{2}[\\/]logs$/, '');
  }

  const baseName = (p) => String(p || '').split(/[\\/]/).pop();
  // The app reports the log it is writing as a full path; /api/log/ wants it from the data
  // folder down. A log outside the day folders is just its file name.
  const dataName = (p) => {
    const day = /\d{4}-\d{2}-\d{2}\/logs\/[^/]+$/.exec(String(p || '').replace(/\\/g, '/'));
    return day ? day[0] : baseName(p);
  };

  // Which board a pull or a saved image came from, read off its name. The app has named
  // these with the date first and with the board first, and with a 13-digit number for
  // the date, so the date is taken out wherever it sits and what is left is the board.
  function boardPart(name) {
    const parts = String(name).split('/');
    // <day>/<kind>/1432-board9/..., images 1101-board9-0027003A.bin, 1432-2-... for a second one that minute
    if (/^\d{4}-\d{2}-\d{2}$/.test(parts[0]) && parts.length >= 3) {
      const last = parts[parts.length - 1];
      const image = /\.bin$/i.test(last);
      let own = (image ? last : parts[2]).replace(/\.[a-z]+$/i, '');
      if (!/^\d{4}-/.test(own)) return '';
      own = own.replace(/^\d{4}-(?:\d{1,3}-)?/, '');
      if (image) own = own.replace(/-[0-9a-f]{8}$/i, '');
      return own === 'ground' ? '' : own;
    }
    const own = parts.length > 1 && /\.bin$/i.test(parts[parts.length - 1]) ? parts[parts.length - 1] : parts[0];
    const m = /^descent-(?:ground-)?(pull|fsw|backup|image|chip)-(.*)$/.exec(own.replace(/\.[a-z]+$/i, ''));
    if (!m) return '';
    return m[2]
      .replace(/\d{4}-\d{2}-\d{2}[T_]\d{2}-\d{2}(?:-\d{2})?/, '')
      .replace(/(^|-)\d{13}(?=-|$)/, '')
      .replace(/^-+|-+$/g, '').replace(/-{2,}/g, '-');
  }

  const WHAT = { session: 'Live session', pull: 'Flight log pull', image: 'Firmware image' };
  const ACTIONS = { session: ['open', 'log', 'csv'], pull: ['open', 'log', 'csv', 'raw'], image: ['save', 'write'] };

  // opt.current: path of the log being written now. opt.names: uid -> the board's name.
  function rows(logs, opt) {
    const o = opt || {};
    const current = String(o.current || '').replace(/\\/g, '/');
    // full path vs names from the data folder down, or older apps' flat names
    const isCurrent = (name) => !!current && (current.endsWith('/' + name) || baseName(current) === name);
    const names = o.names || {};
    // A file may name its board by the last eight digits of the id.
    const nameOf = (uid) => {
      const tail = String(uid).toLowerCase();
      const key = Object.keys(names).find((k) => k.toLowerCase().endsWith(tail));
      return key ? names[key] : '';
    };
    return (logs || []).slice().sort((a, b) => (b.when || 0) - (a.when || 0)).map((l) => {
      const kind = WHAT[l.kind] ? l.kind : 'session';
      const slash = l.name.lastIndexOf('/');
      const folder = slash > 0 ? l.name.slice(0, slash) : '';
      const part = boardPart(l.name);
      const from = kind === 'session' ? 'Receivers'
        : (l.uid && nameOf(l.uid)) || (l.uid ? 'board …' + String(l.uid).slice(-4) : part || 'a board');
      return {
        name: l.name, kind, what: WHAT[kind], from, uid: l.uid || null,
        when: l.when, size: bytes(l.bytes),
        current: kind === 'session' && isCurrent(l.name),
        folder,
        raw: kind === 'pull' && folder ? folder + '/dump.txt' : null,
        report: kind === 'pull' && folder ? folder + '/report.json' : null,
        actions: ACTIONS[kind],
      };
    });
  }

  // Every record carries the id of the board that wrote it. A report lists the distinct
  // ones, ascending; an older report has no list.
  const ids = (csids) => (Array.isArray(csids) ? csids.filter((n) => Number.isFinite(Number(n))) : []);

  function boardIdWords(csids) {
    const list = ids(csids);
    if (!list.length) return '';
    if (list.length === 1) return 'board id ' + list[0];
    return 'board ids ' + list.slice(0, -1).join(', ') + ' and ' + list[list.length - 1];
  }

  // The records against the id the board's own settings give. 255 is a board nobody has
  // given an id, and a board with no settings block gives none, so neither is compared.
  function idMismatch(csids, boardCsid) {
    const list = ids(csids);
    const own = Number(boardCsid);
    if (!list.length || boardCsid === null || boardCsid === undefined || !Number.isFinite(own) || own === 255) return '';
    if (list.some((n) => Number(n) === own)) return '';
    return 'The records carry ' + boardIdWords(list) + ', not ' + own + '.';
  }

  // firstMs and lastMs are on the boot's own clock. A boot started at uptime0Ms + firstMs,
  // which is a date only when GPS pinned that clock down.
  function bootRows(report) {
    const boots = report && Array.isArray(report.boots) ? report.boots : [];
    return boots.map((b) => {
      const at = Number(b.uptime0Ms) + Number(b.firstMs);
      const dated = b.timeSource === 'anchor' && b.uptime0Ms !== null && b.uptime0Ms !== undefined &&
        Number.isFinite(at) && at > 0;
      const used = Number(b.anchorsUsed) || 0;
      const out = Number(b.anchorsRejected) || 0;
      const row = {
        boot: String(b.n),
        records: count(b.records),
        ranFor: duration(Number(b.lastMs) - Number(b.firstMs)),
        started: dated ? utcStamp(at) : 'not dated',
        dated,
        anchors: !used && !out ? 'none' : count(used) + ' used' + (out ? ', ' + count(out) + ' left out' : ''),
        clock: clockWords(b.clockFastPpm),
        ids: ids(b.csids).join(', ') || '—',
      };
      // only with event records. reset is the RCC CSR flags the board printed
      if (events(report)) {
        row.reset = typeof b.reset === 'string' ? b.reset.split('+').join(', ') : '—';
        row.dropped = Number.isFinite(b.dropped) ? count(b.dropped) : '—';
      }
      return row;
    });
  }

  function events(report) {
    const boots = report && Array.isArray(report.boots) ? report.boots : [];
    return boots.some((b) => b.reset !== undefined || b.dropped !== undefined || b.otherEvents !== undefined);
  }

  // short is the word that goes in the warning mark beside the value; empty when the
  // row is as it should be.
  function transferRows(report) {
    const r = report || {};
    const known = Number.isFinite(r.found);
    const share = (n) => (known && r.found > 0 && n > 0 ? (100 * n / r.found).toFixed(1) + '%' : n > 0 ? 'some' : '');
    return [
      { key: 'found', label: 'The board said it holds',
        value: known ? count(r.found) + ' records' + (r.boardBad > 0 ? ', ' + count(r.boardBad) + ' of them bad on the chip' : '')
          : 'it did not say', short: '' },
      { key: 'read', label: 'Read with the check passing', value: count(r.read || 0), short: '' },
      { key: 'failed', label: 'Failed their check and left out', value: count(r.failedCheck || 0), short: share(r.failedCheck || 0) },
      { key: 'missing', label: 'Missing', value: count(r.missing || 0), short: share(r.missing || 0) },
      { key: 'gap', label: 'Longest the reader went without reading', value: duration(r.longestReaderGapMs), short: '' },
      { key: 'bytes', label: 'Came off the board', value: bytes(r.bytes), short: '' },
      { key: 'whole', label: 'Every record the board counted as good was read',
        value: r.complete ? 'yes' : 'no', short: r.complete ? '' : 'incomplete' },
    ].concat(r.wrapped ? [{ key: 'wrapped', label: 'The log had gone round the chip, so the oldest records were written over',
      value: 'yes', short: '' }] : []);
  }

  return { duration, clockWords, bytes, day, dayTime, utcStamp, keptIn, builtWords, baseName, dataName, boardPart, rows, boardIdWords, idMismatch, bootRows, events, transferRows };
});
