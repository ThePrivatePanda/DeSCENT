// The Data screen: one table of everything the app has written to its folder, filtered by
// kind, board and day, and for a pull its report, boot by boot. Nothing here deletes a file.
// The filter functions are pure and sit first, so the tests can load them in node; the
// screen below needs the page and the app, and is left out without them.
(function () {
  'use strict';
  const D = typeof module === 'object' && module.exports ? require('./datamodel.js') : DG.datamodel;
  const fmt = new Intl.NumberFormat('en-US');
  const two = (n) => String(n).padStart(2, '0');

  const KINDS = { all: null, logs: 'session', pulls: 'pull', images: 'image' };
  const WHENS = ['today', '7d', 'all'];
  const DEFAULT_FILTERS = { kind: 'all', from: 'any', when: 'all', search: '' };
  const STORE_KEY = 'dg-data-filters';

  const startOfDay = (d, back) => new Date(d.getFullYear(), d.getMonth(), d.getDate() - (back || 0)).getTime() / 1000;

  // f = {kind, from, when, search}. "7d" is today and the six calendar days before it, by
  // this computer's clock, like the "Written" column. A file with no date is only in "all".
  function filterRows(rows, f, now) {
    const at = now || new Date();
    const want = KINDS[f.kind] || null;
    const since = f.when === 'today' ? startOfDay(at) : f.when === '7d' ? startOfDay(at, 6) : null;
    const q = String(f.search || '').trim().toLowerCase();
    return rows.filter((r) => {
      if (want && r.kind !== want) return false;
      if (f.from && f.from !== 'any' && r.from !== f.from) return false;
      if (since !== null && !(r.when >= since)) return false;
      return !q || [r.name, r.from, r.what].join('\n').toLowerCase().includes(q);
    });
  }

  // Every board a file came from, in the order they first show up, whatever the filters
  // say: a pill that leaves the row when it is pressed cannot be pressed again. A board
  // that is selected but has no files left (the folder was cleared) stays, to be unpicked.
  function boardsOf(rows, f) {
    const out = [];
    for (const r of rows) if (r.from && !out.includes(r.from)) out.push(r.from);
    if (f && f.from && f.from !== 'any' && !out.includes(f.from)) out.push(f.from);
    return out;
  }

  // What each pill would show if pressed, given the other two filters and the search.
  function filterCounts(rows, f, now) {
    const n = (over) => filterRows(rows, Object.assign({}, f, over), now).length;
    const boards = new Map();
    for (const b of boardsOf(rows, f)) boards.set(b, n({ from: b }));
    const kind = {};
    for (const k of Object.keys(KINDS)) kind[k] = n({ kind: k });
    const when = {};
    for (const w of WHENS) when[w] = n({ when: w });
    return { kind, from: { any: n({ from: 'any' }), boards }, when };
  }

  // The pills are kept for the browser session; the search box is a query, not a choice,
  // and starts empty. Storage can be missing or throw (private window), and the page runs without.
  function loadFilters(store) {
    const out = Object.assign({}, DEFAULT_FILTERS);
    let saved = null;
    try { saved = JSON.parse(store.getItem(STORE_KEY)); } catch (e) { saved = null; }
    if (!saved || typeof saved !== 'object') return out;
    if (Object.prototype.hasOwnProperty.call(KINDS, saved.kind)) out.kind = saved.kind;
    if (typeof saved.from === 'string' && saved.from) out.from = saved.from;
    if (WHENS.includes(saved.when)) out.when = saved.when;
    return out;
  }

  function saveFilters(store, f) {
    try { store.setItem(STORE_KEY, JSON.stringify({ kind: f.kind, from: f.from, when: f.when })); } catch (e) { /* kept for this page only */ }
  }

  // "Today 14:02", "Yesterday 16:40", "14 Oct 09:12".
  function written(sec, now) {
    const d = new Date(sec * 1000);
    if (!sec || Number.isNaN(d.getTime())) return '—';
    const at = now || new Date();
    const hm = two(d.getHours()) + ':' + two(d.getMinutes());
    const back = Math.round((startOfDay(at) - startOfDay(d)) / 86400);
    return (back === 0 ? 'Today' : back === 1 ? 'Yesterday' : D.day(d, at)) + ' ' + hm;
  }

  const pure = { filterRows, filterCounts, boardsOf, loadFilters, saveFilters, written };
  if (typeof module === 'object' && module.exports) module.exports = pure;
  if (typeof window === 'undefined') return;
  window.DGData = pure;

  const A = window.DGApp;
  const { $, esc, logUrl, flatName } = A.ui;
  const { paint, mark } = A.shell;
  const hub = A.hub;
  if (!hub.native) return;

  const store = () => { try { return window.sessionStorage; } catch (e) { return null; } };
  const state = {
    rows: [], bytes: new Map(), sel: null, loaded: false, failed: '',
    filters: loadFilters(store()),
    reports: new Map(), missing: new Set(), pending: new Map(), transferOpen: false,
  };

  // A pull's report.json, asked for once. The row notes and the panel both want it; one that
  // failed is asked for again the next time the folder is read (a pull still running has none yet).
  function reportOf(row) {
    const name = row.report;
    if (state.reports.has(name)) return Promise.resolve(state.reports.get(name));
    if (!state.pending.has(name)) {
      state.pending.set(name, hub.report(name).then((r) => {
        state.reports.set(name, r);
        state.missing.delete(name);
        return r;
      }, () => {
        state.missing.add(name);
        return null;
      }).then((r) => { state.pending.delete(name); return r; }));
    }
    return state.pending.get(name);
  }

  async function load() {
    let logs = [];
    let names = {};
    state.failed = '';
    try {
      logs = await hub.logs();
    } catch (e) {
      state.failed = 'Could not read the folder: ' + (e.message || e);
    }
    try {
      for (const c of await hub.chips()) if (c.name) names[c.uid] = c.name;
    } catch (e) { names = {}; }   // an older build remembers no boards; the file names still say
    state.rows = D.rows(logs, { current: hub.log, names });
    state.bytes = new Map(logs.map((l) => [l.name, Number(l.bytes) || 0]));
    // The newest pull is what this screen is most often opened for, so its report is up
    // without a click the first time.
    if (!state.loaded && !state.sel) {
      const pull = state.rows.find((r) => r.kind === 'pull');
      if (pull) pick(pull.name);
    }
    state.loaded = true;
    if (state.sel && !state.rows.some((r) => r.name === state.sel)) state.sel = null;
    render();
    // The notes on the pull rows come from their reports, which are not in the list.
    const asked = state.rows.filter((r) => r.report && !state.reports.has(r.report));
    if (asked.length) {
      await Promise.all(asked.map(reportOf));
      render();
    }
  }

  async function pick(name) {
    state.sel = name;
    state.transferOpen = false;
    const row = state.rows.find((r) => r.name === name);
    render();
    if (!row || !row.report) return;
    await reportOf(row);
    if (state.sel === name) render();
  }

  // The word on a pull: whether every record the board counted as good was read.
  function completeNote(rep) {
    if (!rep || typeof rep.complete !== 'boolean') return null;
    if (rep.complete) return { cls: 'good', word: 'complete' };
    const short = rep.missing > 0 ? ' · ' + fmt.format(rep.missing) + ' missing'
      : rep.failedCheck > 0 ? ' · ' + fmt.format(rep.failedCheck) + ' failed the check' : '';
    return { cls: 'warn', word: 'incomplete' + short };
  }

  const WHAT = { session: 'Session log', pull: 'Flight log pull', image: 'Saved image' };

  const link = (name, label) => '<a class="button" download="' + esc(flatName(name)) + '" href="' + logUrl(name) + '">' + label + '</a>';
  const ACTION = {
    open: (r) => '<button data-open="' + esc(r.name) + '">Open</button>',
    log: (r) => link(r.name, r.kind === 'session' ? 'Save' : 'Save log'),
    csv: (r) => '<button data-csv="' + esc(r.name) + '">Save CSV</button>',
    raw: (r) => link(r.raw, 'Save raw'),
    save: (r) => link(r.name, 'Save'),
    write: (r) => '<button data-write="' + esc(r.name) + '">Write to a board…</button>',
  };

  function noteHtml(r) {
    if (r.current) return mark('good', 'being written now');
    const n = r.report && completeNote(state.reports.get(r.report));
    return n ? mark(n.cls, n.word) : '';
  }

  function tableHtml(shown, now) {
    return '<thead><tr><th>What</th><th>From</th><th>Written</th><th class="r">Size</th><th></th></tr></thead><tbody>' +
      shown.map((r) => {
        const sel = r.name === state.sel;
        return '<tr data-row="' + esc(r.name) + '"' + (sel ? ' class="sel"' : '') + ' title="' + esc(r.name) + '">' +
          '<td><b>' + WHAT[r.kind] + '</b> ' + noteHtml(r) +
            // the file's name is a tooltip on the rest, and said in words for the one picked
            (sel ? ' <span class="note path">' + esc(r.name) + '</span>' : '') + '</td>' +
          '<td>' + esc(r.from) + '</td>' +
          '<td>' + esc(written(r.when, now)) + '</td>' +
          '<td class="r size">' + r.size + '</td>' +
          '<td><div class="acts">' + r.actions.map((a) => ACTION[a](r)).join('') + '</div></td>' +
        '</tr>';
      }).join('') + '</tbody>';
  }

  const pill = (attr, value, label, count, on) =>
    '<button class="fpill' + (on ? ' on' : '') + '" data-' + attr + '="' + esc(value) + '" aria-pressed="' + on + '">' +
      esc(label) + ' · ' + fmt.format(count) + '</button>';

  function filtersHtml(now) {
    const f = state.filters;
    const c = filterCounts(state.rows, f, now);
    const group = (name, pills) => '<div class="fgroup"><span class="kicker">' + name + '</span>' + pills + '</div>';
    return group('Kind', [['all', 'All'], ['logs', 'Session logs'], ['pulls', 'Pulls'], ['images', 'Images']]
        .map(([k, l]) => pill('kind', k, l, c.kind[k], f.kind === k)).join('')) +
      group('From', pill('from', 'any', 'Any board', c.from.any, f.from === 'any') +
        [...c.from.boards].map(([b, n]) => pill('from', b, b, n, f.from === b)).join('')) +
      group('When', [['today', 'Today'], ['7d', '7 days'], ['all', 'All time']]
        .map(([k, l]) => pill('when', k, l, c.when[k], f.when === k)).join(''));
  }

  function whereHtml(shown) {
    const kept = D.keptIn(hub.log);
    const bytes = shown.reduce((sum, r) => sum + (state.bytes.get(r.name) || 0), 0);
    return (kept ? '<span class="path">' + esc(kept) + '</span> · ' : '') +
      fmt.format(shown.length) + (shown.length === 1 ? ' file' : ' files') + ' · ' + D.bytes(bytes);
  }

  function bootsHtml(rep, boots) {
    const ev = D.events(rep);
    const head = ['Boot', 'Records', 'Ran for', 'Started', 'GPS anchors', 'Clock'].concat(ev ? ['Reset cause', 'Dropped'] : []);
    return '<div class="bootwrap"><table class="boots"><thead><tr>' + head.map((h) => '<th>' + h + '</th>').join('') + '</tr></thead><tbody>' +
      boots.map((b) => '<tr' + (b.dated ? '' : ' class="undated"') + '><td><b>' + esc(b.boot) + '</b></td><td>' + b.records + '</td><td>' + b.ranFor + '</td>' +
        '<td>' + esc(b.started) + '</td><td>' + b.anchors + '</td>' +
        '<td title="How fast the board\'s clock ran against GPS">' + b.clock + '</td>' +
        (ev ? '<td title="The reset flags the board found at this boot">' + esc(b.reset) + '</td>' +
          '<td title="Records lost when the RAM ring overran before the flash caught up">' + b.dropped + '</td>' : '') +
        '</tr>').join('') + '</tbody></table></div>';
  }

  function summary(row, rep, boots, now) {
    const parts = [written(row.when, now).replace(/^(Today|Yesterday)/, (m) => m.toLowerCase())];
    if (Number.isFinite(rep.read)) {
      parts.push(fmt.format(rep.read) + (Number.isFinite(rep.found) ? ' of ' + fmt.format(rep.found) : '') + ' records');
    }
    if (Array.isArray(rep.boots)) {
      parts.push(fmt.format(boots.length) + (boots.length === 1 ? ' boot' : ' boots'));
      parts.push(fmt.format(rep.boots.filter((b) => b && b.timeSource === 'anchor').length) + ' with GPS time');
    }
    if (D.boardIdWords(rep.csids)) parts.push(D.boardIdWords(rep.csids));
    return parts.join(' · ');
  }

  function transferHtml(rep) {
    const open = state.transferOpen;
    return '<button class="expander" data-transfer="1" aria-expanded="' + open + '">How the transfer went</button>' +
      (open ? '<div class="xfer">' + D.transferRows(rep).map((t) =>
        '<div><span>' + t.label + '</span><b>' + esc(t.value) + '</b>' + (t.short ? mark('warn', t.short) : '') + '</div>').join('') + '</div>' : '');
  }

  function detailHtml(row, now) {
    const none = (words) => '<p class="collapsed">' + words + '</p>';
    if (!row || row.kind !== 'pull' || !row.report) return none('Pick a pull to see its report');
    const rep = state.reports.get(row.report);
    if (!rep) return none(state.missing.has(row.report) ? 'No report with this pull.' : 'Reading the report…');
    const boots = D.bootRows(rep);
    const note = completeNote(rep);
    const fw = rep.firmware || {};
    const made = [
      fw.dump && fw.dump.words ? 'Read with the ' + fw.dump.words + '.' : '',
      fw.onBoard && fw.onBoard.built && fw.onBoard.built.words ? 'On the board was the ' + fw.onBoard.built.words + '.' : '',
    ].filter(Boolean).join(' ');
    return '<div class="report"><div class="rmain">' +
        '<div class="rhead"><h3>' + WHAT.pull + ' · ' + esc(row.from) + '</h3>' +
          '<span class="note">' + esc(summary(row, rep, boots, now)) + '</span>' + (note ? mark(note.cls, note.word) : '') + '</div>' +
        (boots.length ? bootsHtml(rep, boots) : '<p class="note">The report lists no boots.</p>') +
        transferHtml(rep) +
      '</div><div class="rside">' +
        '<p class="note">' + (made ? esc(made) + ' ' : '') +
          (row.folder ? 'In <span class="path">' + esc(row.folder) + '/</span>.' : '') + '</p>' +
        '<div class="acts">' +
          '<button class="primary" data-open="' + esc(row.name) + '">Open in Replay</button>' +
          '<button data-csv="' + esc(row.name) + '">Save CSV</button>' +
          (row.raw ? link(row.raw, 'Save raw') : '') +
        '</div>' +
      '</div></div>';
  }

  function render() {
    const now = new Date();
    const shown = filterRows(state.rows, state.filters, now);
    paint($('data-where'), whereHtml(shown));
    paint($('data-filters'), filtersHtml(now));
    paint($('data-table'), tableHtml(shown, now));
    const empty = state.failed || (!state.rows.length ? 'No files yet.'
      : !shown.length ? 'No file matches. Loosen the filters or clear the search.' : '');
    paint($('data-empty'), esc(empty));
    $('data-empty').hidden = !empty;
    $('data-detail').hidden = false;
    paint($('data-detail'), detailHtml(state.rows.find((r) => r.name === state.sel), now));
  }

  $('data-look').addEventListener('click', async () => {
    await load();
    const n = state.rows.length;
    A.ui.toast(state.failed || n + (n === 1 ? ' file' : ' files'));
  });

  $('data-search').addEventListener('input', (e) => {
    state.filters.search = e.target.value || '';
    render();
  });

  // A keyboard press on a pill repaints the pill under the focus; put the focus back.
  function refocus(root, attr, value) {
    if (!root.querySelector) return;
    const again = [...root.querySelectorAll('[' + attr + ']')].find((b) => b.getAttribute(attr) === value);
    if (again) again.focus();
  }

  $('screen-data').addEventListener('click', (e) => {
    const at = (name) => e.target.getAttribute(name);
    const row = (name) => state.rows.find((r) => r.name === name);
    for (const key of ['kind', 'from', 'when']) {
      const v = at('data-' + key);
      if (v === null) continue;
      state.filters[key] = v;
      saveFilters(store(), state.filters);
      render();
      if (e.detail === 0) refocus($('data-filters'), 'data-' + key, v);
      return;
    }
    if (at('data-transfer')) { state.transferOpen = !state.transferOpen; render(); return; }
    if (at('data-open')) {
      const r = row(at('data-open'));
      const kept = D.keptIn(hub.log);
      const sep = kept.includes('\\') ? '\\' : '/';
      A.openServerLog(r.name, { raw: r.raw, dir: kept && r.folder ? kept + sep + r.folder : null });
      return;
    }
    if (at('data-csv')) { A.shell.saveCsv(at('data-csv')); return; }
    if (at('data-write')) {
      const r = row(at('data-write'));
      A.shell.writeImage({ name: r.name, when: r.when, uid: r.uid, from: r.from });
      return;
    }
    // A link saves the file and is not a pick; anything else on a row picks it.
    if (e.target.closest('a')) return;
    const tr = e.target.closest('tr[data-row]');
    if (tr) pick(tr.dataset.row);
  });

  A.shell.register('data', { show: load });
})();
