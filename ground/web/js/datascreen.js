// The Data screen: one table of everything the app has written to its folder, and for a
// pull its report, boot by boot. Nothing here deletes a file.
(function () {
  'use strict';
  const A = window.DGApp;
  const { $, esc, logUrl, flatName } = A.ui;
  const { paint, mark } = A.shell;
  const D = DG.datamodel;
  const hub = A.hub;
  if (!hub.native) return;

  const state = {
    rows: [], sel: null, loaded: false, failed: '',
    report: null, reportFor: null, reportMissing: false,
  };

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
    // The newest pull is what this screen is most often opened for, so its report is up
    // without a click the first time.
    if (!state.loaded && !state.sel) {
      const pull = state.rows.find((r) => r.kind === 'pull');
      if (pull) pick(pull.name);
    }
    state.loaded = true;
    if (state.sel && !state.rows.some((r) => r.name === state.sel)) state.sel = null;
    render();
  }

  async function pick(name) {
    state.sel = name;
    const row = state.rows.find((r) => r.name === name);
    render();
    if (!row || !row.report || state.reportFor === name) return;
    state.report = null; state.reportMissing = false; state.reportFor = name;
    try {
      state.report = await hub.report(row.report);
    } catch (e) {
      state.reportMissing = true;
    }
    if (state.sel === name) render();
  }

  const ACTION = {
    open: (r) => '<button data-open="' + esc(r.name) + '">Open</button>',
    log: (r) => '<a class="button" download="' + esc(flatName(r.name)) + '" href="' + logUrl(r.name) + '">Save log</a>',
    csv: (r) => '<button data-csv="' + esc(r.name) + '">Save CSV</button>',
    raw: (r) => '<a class="button" download="' + esc(flatName(r.raw)) + '" href="' + logUrl(r.raw) + '">Save raw</a>',
    save: (r) => '<a class="button" download="' + esc(flatName(r.name)) + '" href="' + logUrl(r.name) + '">Save</a>',
    write: (r) => '<button data-write="' + esc(r.name) + '">Write to a board…</button>',
  };

  function tableHtml() {
    const now = new Date();
    return state.rows.map((r) =>
      '<tr data-row="' + esc(r.name) + '"' + (r.name === state.sel ? ' class="sel"' : '') + '>' +
        '<td><b class="val">' + r.what + '</b>' + (r.current ? ' ' + mark('good', 'being written now') : '') +
          '<span class="note under">' + esc(r.name) + '</span></td>' +
        '<td>' + esc(r.from) + '</td>' +
        '<td>' + esc(D.dayTime(new Date(r.when * 1000), now)) + '</td>' +
        '<td class="r">' + r.size + '</td>' +
        '<td><div class="acts">' + r.actions.map((a) => ACTION[a](r)).join('') + '</div></td>' +
      '</tr>').join('');
  }

  function detailHtml(row) {
    if (state.reportFor !== row.name || (!state.report && !state.reportMissing)) {
      return '<div class="pane"><p class="empty">Reading the report…</p></div>';
    }
    if (state.reportMissing) {
      return '<div class="pane"><p class="empty">No report with this pull.</p></div>';
    }
    const boots = D.bootRows(state.report);
    const undated = boots.filter((b) => !b.dated).length;
    const ev = D.events(state.report);
    const bootTable = boots.length
      ? '<div class="scroll"><table class="grid plain"><thead><tr><th>Boot</th><th class="r">Records</th><th class="r">Ran for</th><th>Started</th><th>GPS anchors</th><th>Clock</th><th class="r">Board id</th>' +
        (ev ? '<th>Reset cause</th><th class="r">Dropped</th>' : '') + '</tr></thead><tbody>' +
        boots.map((b) => '<tr><td>' + esc(b.boot) + '</td><td class="r">' + b.records + '</td><td class="r">' + b.ranFor + '</td>' +
          '<td' + (b.dated ? '' : ' class="muted"') + '>' + esc(b.started) + '</td>' +
          '<td' + (b.anchors === 'none' ? ' class="muted"' : '') + '>' + b.anchors + '</td>' +
          '<td' + (b.clock === '—' ? ' class="muted"' : '') + ' title="How fast the board\'s clock ran against GPS">' + b.clock + '</td>' +
          '<td class="r' + (b.ids === '—' ? ' muted' : '') + '">' + b.ids + '</td>' +
          (ev ? '<td' + (b.reset === '—' ? ' class="muted"' : '') + ' title="The reset flags the board found at this boot">' + esc(b.reset) + '</td>' +
            '<td class="r' + (b.dropped === '—' ? ' muted' : '') + '" title="Records lost when the RAM ring overran before the flash caught up">' + b.dropped + '</td>' : '') +
          '</tr>').join('') +
        '</tbody></table></div>'
      : '<p class="empty">The report lists no boots.</p>';
    const transfer = D.transferRows(state.report).map((t) =>
      '<div><span class="grow">' + t.label + '</span><b class="val">' + esc(t.value) + '</b>' + (t.short ? mark('warn', t.short) : '') + '</div>').join('');
    return '<div class="pane"><h3>This pull, boot by boot' +
        '<span class="note">' + boots.length + (boots.length === 1 ? ' boot' : ' boots') + (undated ? ', ' + undated + ' not dated' : '') +
          (D.boardIdWords(state.report.csids) ? ', ' + D.boardIdWords(state.report.csids) : '') + ', from ' + esc(row.from) + '</span></h3>' + bootTable + '</div>' +
      '<div class="pane"><h3>How the transfer went</h3><div class="rows">' + transfer + '</div></div>';
  }

  function render() {
    const kept = D.keptIn(hub.log);
    paint($('data-where'), kept ? 'Kept in <span class="path">' + esc(kept) + '</span>' : '');
    paint($('data-table').querySelector('tbody'), tableHtml());
    $('data-empty').hidden = state.rows.length > 0;
    $('data-empty').textContent = state.failed || 'No files yet.';
    const row = state.rows.find((r) => r.name === state.sel);
    const detail = row && row.kind === 'pull';
    $('data-detail').hidden = !detail;
    if (detail) paint($('data-detail'), detailHtml(row));
  }

  $('data-look').addEventListener('click', async () => {
    await load();
    const n = state.rows.length;
    A.ui.toast(state.failed || n + (n === 1 ? ' file' : ' files'));
  });

  $('screen-data').addEventListener('click', (e) => {
    const at = (name) => e.target.getAttribute(name);
    const row = (name) => state.rows.find((r) => r.name === name);
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
