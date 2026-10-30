// The Data screen: the filter functions on their own, then the screen's own script run
// against test/mock/fakepage.js. This checks what it paints and what it asks the app for,
// not how the page looks.
const test = require('node:test');
const assert = require('node:assert');
const { page } = require('./mock/fakepage.js');
const G = require('../web/js/datascreen.js');

// Thursday 29 Oct 2026, 22:00 local. Rows are the six in the design frame, newest first.
const NOW = new Date(2026, 9, 29, 22, 0, 0);
const at = (mo, d, h, mi) => Math.floor(new Date(2026, mo, d, h, mi).getTime() / 1000);
const row = (name, kind, from, when) => ({ name, kind, from, when, what: { session: 'Live session', pull: 'Flight log pull', image: 'Firmware image' }[kind] });
const SIX = [
  row('2026-10-29/logs/2157-ground.log', 'session', 'Mast, Handheld', at(9, 29, 21, 57)),
  row('2026-10-29/pulls/1402-falcon-2/ground.log', 'pull', 'Falcon 2', at(9, 29, 14, 2)),
  row('2026-10-28/pulls/1640-falcon-2/ground.log', 'pull', 'Falcon 2', at(9, 28, 16, 40)),
  row('2026-10-14/images/0912-falcon-2-0027003A.bin', 'image', 'Falcon 2', at(9, 14, 9, 12)),
  row('2026-10-14/logs/0940-ground.log', 'session', 'Mast', at(9, 14, 9, 40)),
  row('2026-10-12/pulls/1521-falcon-1/ground.log', 'pull', 'Falcon 1', at(9, 12, 15, 21)),
];
const ALL = { kind: 'all', from: 'any', when: 'all', search: '' };
const names = (rows) => rows.map((r) => r.name.split('/').pop());

test('no filter shows every row, in the order given', () => {
  assert.deepStrictEqual(G.filterRows(SIX, ALL, NOW), SIX);
});

test('kind keeps session logs, pulls or images', () => {
  assert.equal(G.filterRows(SIX, Object.assign({}, ALL, { kind: 'logs' }), NOW).length, 2);
  assert.equal(G.filterRows(SIX, Object.assign({}, ALL, { kind: 'pulls' }), NOW).length, 3);
  assert.deepStrictEqual(names(G.filterRows(SIX, Object.assign({}, ALL, { kind: 'images' }), NOW)), ['0912-falcon-2-0027003A.bin']);
});

test('from keeps one board by the name the row shows', () => {
  assert.equal(G.filterRows(SIX, Object.assign({}, ALL, { from: 'Falcon 2' }), NOW).length, 3);
  assert.equal(G.filterRows(SIX, Object.assign({}, ALL, { from: 'Mast' }), NOW).length, 1);
  assert.equal(G.filterRows(SIX, Object.assign({}, ALL, { from: 'Nobody' }), NOW).length, 0);
});

test('when counts calendar days on this computer: today, the last seven days, all', () => {
  assert.equal(G.filterRows(SIX, Object.assign({}, ALL, { when: 'today' }), NOW).length, 2);
  // today and the six days before it: yesterday is in, 14 Oct is not
  assert.equal(G.filterRows(SIX, Object.assign({}, ALL, { when: '7d' }), NOW).length, 3);
  assert.equal(G.filterRows(SIX, ALL, NOW).length, 6);
  // 00:10 today is today, 23:50 yesterday is not
  const edge = [row('a', 'session', 'x', at(9, 29, 0, 10)), row('b', 'session', 'x', at(9, 28, 23, 50))];
  assert.deepStrictEqual(names(G.filterRows(edge, Object.assign({}, ALL, { when: 'today' }), NOW)), ['a']);
  // seven calendar days back from the 29th starts on the 23rd
  const week = [row('c', 'session', 'x', at(9, 23, 0, 5)), row('d', 'session', 'x', at(9, 22, 23, 55))];
  assert.deepStrictEqual(names(G.filterRows(week, Object.assign({}, ALL, { when: '7d' }), NOW)), ['c']);
});

test('a file with no date is only in All time', () => {
  const undated = [row('z', 'pull', 'Falcon 2', 0)];
  assert.equal(G.filterRows(undated, Object.assign({}, ALL, { when: 'today' }), NOW).length, 0);
  assert.equal(G.filterRows(undated, Object.assign({}, ALL, { when: '7d' }), NOW).length, 0);
  assert.equal(G.filterRows(undated, ALL, NOW).length, 1);
});

test('search matches the file name, the board and what it is, whatever the case', () => {
  const f = (search) => Object.assign({}, ALL, { search });
  assert.equal(G.filterRows(SIX, f('falcon'), NOW).length, 4);
  assert.equal(G.filterRows(SIX, f('  FALCON 1 '), NOW).length, 1);
  assert.equal(G.filterRows(SIX, f('0027003a'), NOW).length, 1);
  assert.equal(G.filterRows(SIX, f('pull'), NOW).length, 3);
  assert.equal(G.filterRows(SIX, f('2026-10-14'), NOW).length, 2);
});

test('the filters combine', () => {
  const f = { kind: 'pulls', from: 'Falcon 2', when: '7d', search: '1402' };
  assert.deepStrictEqual(names(G.filterRows(SIX, f, NOW)), ['ground.log']);
  assert.equal(G.filterRows(SIX, Object.assign({}, f, { search: '0912' }), NOW).length, 0);
});

test('counts say what each pill would show given the other filters', () => {
  const c = G.filterCounts(SIX, ALL, NOW);
  assert.deepStrictEqual(c.kind, { all: 6, logs: 2, pulls: 3, images: 1 });
  assert.deepStrictEqual(c.when, { today: 2, '7d': 3, all: 6 });
  assert.equal(c.from.any, 6);
  assert.equal(c.from.boards.get('Falcon 2'), 3);
  assert.equal(c.from.boards.get('Falcon 1'), 1);
  assert.equal(c.from.boards.get('Mast'), 1);
  assert.equal(c.from.boards.get('Mast, Handheld'), 1);

  // Pulls only: the from and when counts shrink to pulls, the kind counts do not
  const p = G.filterCounts(SIX, Object.assign({}, ALL, { kind: 'pulls' }), NOW);
  assert.deepStrictEqual(p.kind, { all: 6, logs: 2, pulls: 3, images: 1 });
  assert.equal(p.from.any, 3);
  assert.equal(p.from.boards.get('Falcon 2'), 2);
  assert.equal(p.from.boards.get('Mast'), 0);
  assert.deepStrictEqual(p.when, { today: 1, '7d': 2, all: 3 });

  // A search narrows every group
  const s = G.filterCounts(SIX, Object.assign({}, ALL, { search: 'falcon 2' }), NOW);
  assert.equal(s.kind.all, 3);
  assert.equal(s.from.any, 3);
  assert.equal(s.when.all, 3);
});

test('a pill count is the table length you get by pressing it', () => {
  const f = { kind: 'all', from: 'Falcon 2', when: '7d', search: '' };
  const c = G.filterCounts(SIX, f, NOW);
  for (const k of ['all', 'logs', 'pulls', 'images']) {
    assert.equal(c.kind[k], G.filterRows(SIX, Object.assign({}, f, { kind: k }), NOW).length, k);
  }
  for (const w of ['today', '7d', 'all']) {
    assert.equal(c.when[w], G.filterRows(SIX, Object.assign({}, f, { when: w }), NOW).length, w);
  }
  assert.equal(c.from.any, G.filterRows(SIX, Object.assign({}, f, { from: 'any' }), NOW).length);
});

// Review focus 5: files from several days and a board that matches none of the named pills
test('a board with no name still counts, and Any board shows it', () => {
  const rows = SIX.concat([row('2026-10-17/pulls/1124-board9/ground.log', 'pull', 'board …003A', at(9, 17, 11, 24))]);
  const c = G.filterCounts(rows, ALL, NOW);
  // the three groups each add up to the table
  assert.equal(c.kind.all, rows.length);
  assert.equal(c.from.any, rows.length);
  assert.equal(c.when.all, rows.length);
  assert.equal(c.kind.logs + c.kind.pulls + c.kind.images, rows.length);
  assert.equal(c.when.today <= c.when['7d'] && c.when['7d'] <= c.when.all, true);
  // it has a pill of its own
  assert.equal(c.from.boards.get('board …003A'), 1);
  assert.equal(G.boardsOf(rows, ALL).includes('board …003A'), true);
  // Falcon 2 leaves it out, Any board brings it back
  assert.equal(G.filterRows(rows, Object.assign({}, ALL, { from: 'Falcon 2' }), NOW).some((r) => r.from === 'board …003A'), false);
  assert.equal(G.filterRows(rows, ALL, NOW).some((r) => r.from === 'board …003A'), true);
  // the boards in the sum over the pills equal the table
  let sum = 0;
  for (const n of c.from.boards.values()) sum += n;
  assert.equal(sum, rows.length);
});

test('a board that is selected but has no files is still a pill, with 0', () => {
  const f = Object.assign({}, ALL, { from: 'Falcon 9' });
  assert.equal(G.boardsOf(SIX, f).includes('Falcon 9'), true);
  assert.equal(G.filterCounts(SIX, f, NOW).from.boards.get('Falcon 9'), 0);
  assert.equal(G.filterRows(SIX, f, NOW).length, 0);
});

test('written says today and yesterday by name and anything older by date', () => {
  const t = (mo, d, h, mi) => at(mo, d, h, mi);
  assert.equal(G.written(t(9, 29, 14, 2), NOW), 'Today 14:02');
  assert.equal(G.written(t(9, 28, 16, 40), NOW), 'Yesterday 16:40');
  assert.equal(G.written(t(9, 14, 9, 12), NOW), '14 Oct 09:12');
  assert.equal(G.written(Math.floor(new Date(2025, 11, 31, 8, 5).getTime() / 1000), NOW), '31 Dec 2025 08:05');
  assert.equal(G.written(0, NOW), '—');
});

test('boards are listed in the order their files first appear', () => {
  assert.deepStrictEqual(G.boardsOf(SIX, ALL), ['Mast, Handheld', 'Falcon 2', 'Mast', 'Falcon 1']);
});

test('the selection is kept in session storage, and a bad one is ignored', () => {
  const mem = {};
  const store = { getItem: (k) => (k in mem ? mem[k] : null), setItem: (k, v) => { mem[k] = String(v); } };
  assert.deepStrictEqual(G.loadFilters(store), ALL);
  G.saveFilters(store, { kind: 'pulls', from: 'Falcon 2', when: '7d', search: 'typed' });
  assert.ok('dg-data-filters' in mem);
  // the search box is a query, not a selection, so it starts empty
  assert.deepStrictEqual(G.loadFilters(store), { kind: 'pulls', from: 'Falcon 2', when: '7d', search: '' });
  mem['dg-data-filters'] = '{"kind":"everything","from":7,"when":"someday"}';
  assert.deepStrictEqual(G.loadFilters(store), ALL);
  mem['dg-data-filters'] = 'not json';
  assert.deepStrictEqual(G.loadFilters(store), ALL);
  // no storage at all, or one that throws
  assert.deepStrictEqual(G.loadFilters(null), ALL);
  const dead = { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); } };
  assert.deepStrictEqual(G.loadFilters(dead), ALL);
  G.saveFilters(dead, ALL);
  G.saveFilters(null, ALL);
});

// ---- the screen ----

const DAY = 86400;
const TODAY0 = Math.floor(new Date().setHours(0, 0, 1, 0) / 1000);   // a second into today
const KEPT = '/home/op/DeSCENT';
const LOGS = [
  { name: '2026-10-29/logs/2157-ground.log', bytes: 1.2 * 1048576, when: TODAY0 + 20, kind: 'session' },
  { name: '2026-10-29/pulls/1402-falcon-2/ground.log', bytes: 3.8 * 1048576, when: TODAY0 + 10, kind: 'pull', uid: 'AAAA0001' },
  { name: '2026-10-26/pulls/1640-falcon-2/ground.log', bytes: 3.1 * 1048576, when: TODAY0 - 3 * DAY, kind: 'pull', uid: 'AAAA0001' },
  { name: '2026-10-14/images/0912-falcon-2-AAAA0001.bin', bytes: 212 * 1024, when: TODAY0 - 15 * DAY, kind: 'image', uid: 'AAAA0001' },
  { name: '2026-10-14/logs/0940-ground.log', bytes: 640 * 1024, when: TODAY0 - 15 * DAY, kind: 'session' },
  { name: '2026-10-12/pulls/1521-board9/ground.log', bytes: 2.4 * 1048576, when: TODAY0 - 17 * DAY, kind: 'pull', uid: 'BBBB0002' },
];
const FIRST = '2026-10-29/pulls/1402-falcon-2/ground.log';
const SECOND = '2026-10-26/pulls/1640-falcon-2/ground.log';
const NOREP = '2026-10-12/pulls/1521-board9/ground.log';
const boot = (n, o) => Object.assign({ n, records: 100, firstMs: 0, lastMs: 60000, timeSource: 'none', uptime0Ms: null, anchorsUsed: 0, anchorsRejected: 0, clockFastPpm: null, csids: [9] }, o);
const REPORT = {
  found: 12480, read: 12480, failedCheck: 0, missing: 0, complete: true, bytes: 200000, longestReaderGapMs: 0, csids: [9],
  firmware: { dump: { words: 'LE dump 2.6.3' } },
  boots: [
    boot(1, { records: 6210, timeSource: 'anchor', uptime0Ms: 1793300883000, anchorsUsed: 3, clockFastPpm: 11, reset: 'power on', dropped: 0 }),
    boot(2, { records: 420, reset: 'pin', dropped: 14 }),
  ],
};
const SHORT = Object.assign({}, REPORT, { complete: false, read: 12268, missing: 212 });
const REPORTS = {
  '2026-10-29/pulls/1402-falcon-2/report.json': REPORT,
  '2026-10-26/pulls/1640-falcon-2/report.json': SHORT,
};
const rowsIn = (html) => (html.match(/<tr data-row=/g) || []).length;

async function screen(o) {
  o = Object.assign({ logs: LOGS, chips: [{ uid: 'XXAAAA0001', name: 'Falcon 2' }], reports: REPORTS, reported: [] }, o);
  const hub = {
    native: true, log: KEPT + '/2026-10-29/logs/2157-ground.log',
    logs: async () => o.logs, chips: async () => o.chips,
    report: async (name) => { o.reported.push(name); if (!o.reports[name]) throw new Error('HTTP 404'); return o.reports[name]; },
  };
  const p = page(['datamodel.js', 'datascreen.js'], hub, o);
  await p.settle();
  await p.parts.data.show();
  await p.settle();
  const press = (attr, value) => p.fire('screen-data', 'click', { [attr]: value });
  const pickRow = (name) => p.fire('screen-data', 'click', {}, { closest: (s) => (s === 'tr[data-row]' ? { dataset: { row: name } } : null) });
  const search = (value) => p.fire('data-search', 'input', {}, { value });
  return Object.assign(p, { o, press, pickRow, search, table: () => p.text('data-table'), filters: () => p.text('data-filters') });
}

test('the head says where the files are, how many and how big', async () => {
  const p = await screen();
  assert.equal(p.text('data-where'), '/home/op/DeSCENT · 6 files · 11.3 MB');
});

test('the head follows the filtered table, not the folder', async () => {
  const p = await screen();
  await p.press('data-kind', 'images');
  assert.equal(p.text('data-where'), '/home/op/DeSCENT · 1 file · 212 kB');
});

test('three pill groups with counts, and the pressed ones marked', async () => {
  const p = await screen();
  const f = p.filters();
  assert.match(f, /^Kind All · 6 Session logs · 2 Pulls · 3 Images · 1 From Any board · 6 Receivers · 2 Falcon 2 · 3 board …0002 · 1 When Today · 2 7 days · 3 All time · 6$/);
  const on = [...p.html('data-filters').matchAll(/<button class="fpill on"[^>]*>([^<]*)</g)].map((m) => m[1]);
  assert.deepStrictEqual(on, ['All · 6', 'Any board · 6', 'All time · 6']);
  assert.equal((p.html('data-filters').match(/aria-pressed="true"/g) || []).length, 3);
  assert.equal((p.html('data-filters').match(/aria-pressed="false"/g) || []).length, 8);
});

test('a pill filters the table and moves the counts', async () => {
  const p = await screen();
  await p.press('data-kind', 'pulls');
  assert.equal(rowsIn(p.html('data-table')), 3);
  assert.match(p.filters(), /Pulls · 3/);
  assert.match(p.filters(), /Any board · 3/);
  await p.press('data-from', 'Falcon 2');
  assert.equal(rowsIn(p.html('data-table')), 2);
  await p.press('data-when', 'today');
  assert.equal(rowsIn(p.html('data-table')), 1);
  await p.press('data-from', 'any');
  await p.press('data-when', 'all');
  await p.press('data-kind', 'all');
  assert.equal(rowsIn(p.html('data-table')), 6);
});

test('a search with no hit shows the empty line and keeps the pills', async () => {
  const p = await screen();
  assert.equal(p.text('data-empty'), '');
  await p.search('no such file');
  assert.equal(rowsIn(p.html('data-table')), 0);
  assert.equal(p.text('data-empty'), 'No file matches. Loosen the filters or clear the search.');
  assert.match(p.filters(), /All · 0 Session logs · 0/);
  assert.match(p.text('data-where'), /0 files · 0 B/);
  await p.search('');
  assert.equal(rowsIn(p.html('data-table')), 6);
  assert.equal(p.text('data-empty'), '');
});

test('search finds a board by its name', async () => {
  const p = await screen();
  await p.search('falcon');
  assert.equal(rowsIn(p.html('data-table')), 3);
  assert.match(p.filters(), /Falcon 2 · 3 board …0002 · 0 When/);
});

test('an empty folder says so', async () => {
  const none = await screen({ logs: [] });
  assert.equal(none.text('data-empty'), 'No files yet.');
  assert.match(none.text('data-where'), /0 files/);
});

test('rows: what, a note word, from, written, size and the actions that exist', async () => {
  const p = await screen();
  const t = p.table();
  assert.match(t, /Session log being written now/);
  assert.match(t, /Flight log pull complete/);
  assert.match(t, /Flight log pull incomplete · 212 missing/);
  assert.match(t, /Saved image/);
  assert.match(t, /Today 00:00/);
  assert.match(t, /1\.2 MB/);
  assert.match(t, /212 kB/);
  const html = p.html('data-table');
  assert.equal((html.match(/data-open=/g) || []).length, 5);
  assert.equal((html.match(/data-csv=/g) || []).length, 5);
  assert.equal((html.match(/data-write=/g) || []).length, 1);
  assert.equal((html.match(/>Save raw<\/a>/g) || []).length, 3);
  assert.equal((html.match(/>Save log<\/a>/g) || []).length, 3);
  assert.equal((html.match(/>Save<\/a>/g) || []).length, 3);
  assert.equal((html.match(/>Write to a board…<\/button>/g) || []).length, 1);
  assert.match(html, /<a class="button" download="[^"]*" href="[^"]*">Save<\/a>/);
});

test('the selected row is marked, and a press picks it', async () => {
  const p = await screen();
  assert.equal((p.html('data-table').match(/<tr data-row="[^"]*" class="sel"/g) || []).length, 1);
  assert.match(p.html('data-table'), new RegExp('<tr data-row="' + FIRST.replace(/[/.]/g, '\\$&') + '" class="sel"'));
  await p.pickRow(SECOND);
  assert.match(p.html('data-table'), new RegExp('<tr data-row="' + SECOND.replace(/[/.]/g, '\\$&') + '" class="sel"'));
});

test('a row takes the keyboard: Tab reaches it, Enter or Space picks it', async () => {
  const p = await screen();
  assert.equal((p.html('data-table').match(/<tr data-row="[^"]*"[^>]* tabindex="0"/g) || []).length, 6);
  const key = (name, k) => p.fire('screen-data', 'keydown', {}, { matches: (s) => s === 'tr[data-row]', dataset: { row: name } }, { key: k });
  await key(SECOND, 'Enter');
  assert.match(p.html('data-table'), new RegExp('<tr data-row="' + SECOND.replace(/[/.]/g, '\\$&') + '" class="sel"'));
  await key(FIRST, ' ');
  assert.match(p.html('data-table'), new RegExp('<tr data-row="' + FIRST.replace(/[/.]/g, '\\$&') + '" class="sel"'));
});

test('the note on a pull is read off its report, asked for once, retried only when it failed', async () => {
  const p = await screen();
  assert.deepStrictEqual(p.o.reported.slice().sort(),
    [FIRST, SECOND, NOREP].map((n) => n.replace('ground.log', 'report.json')).sort());
  await p.parts.data.show();
  await p.settle();
  assert.equal(p.o.reported.length, 4);
  assert.equal(p.o.reported[3], NOREP.replace('ground.log', 'report.json'));
});

test('the newest pull is picked at first, and its report fills the panel', async () => {
  const p = await screen();
  const t = p.text('data-detail');
  assert.match(t, /^Flight log pull · Falcon 2 today 00:00 · 12,480 of 12,480 records · 2 boots · 1 with GPS time · board id 9 complete/);
  assert.match(t, /Boot Records Ran for Started GPS anchors Clock Reset cause Dropped/);
  assert.match(t, /1 6,210 1 min 29 Oct 2026 \d\d:\d\d:\d\d UTC 3 used 11 ppm fast power on 0/);
  assert.match(t, /2 420 1 min not dated none — pin 14/);
  assert.match(t, /Read with the LE dump 2\.6\.3\. In 2026-10-29\/pulls\/1402-falcon-2\//);
  assert.match(t, /Open in Replay Save CSV Save raw$/);
  assert.equal((p.html('data-detail').match(/<tr class="undated">/g) || []).length, 1);
});

test('a report with no event records has no reset or dropped columns', async () => {
  const bare = JSON.parse(JSON.stringify(REPORT));
  for (const b of bare.boots) { delete b.reset; delete b.dropped; }
  const p = await screen({ reports: { '2026-10-29/pulls/1402-falcon-2/report.json': bare } });
  const t = p.text('data-detail');
  assert.match(t, /Boot Records Ran for Started GPS anchors Clock 1 6,210/);
  assert.doesNotMatch(t, /Reset cause/);
  assert.doesNotMatch(t, /Dropped/);
});

test('an incomplete pull says what is short, in words beside its pill', async () => {
  const p = await screen();
  await p.pickRow(SECOND);
  await p.settle();
  const t = p.text('data-detail');
  assert.match(t, /12,268 of 12,480 records/);
  assert.match(t, /incomplete · 212 missing/);
  assert.doesNotMatch(t, / complete /);
});

test('the transfer figures open in place and close again', async () => {
  const p = await screen();
  assert.doesNotMatch(p.text('data-detail'), /Longest the reader went without reading/);
  await p.press('data-transfer', '1');
  assert.match(p.text('data-detail'), /Longest the reader went without reading 0 ms/);
  assert.match(p.html('data-detail'), /aria-expanded="true"/);
  await p.press('data-transfer', '1');
  assert.doesNotMatch(p.text('data-detail'), /Longest the reader went without reading/);
});

test('a row with no report collapses the panel to one line', async () => {
  const p = await screen();
  await p.pickRow('2026-10-29/logs/2157-ground.log');
  assert.equal(p.text('data-detail'), 'Pick a pull to see its report');
  await p.pickRow(NOREP);
  assert.match(p.text('data-detail'), /No report with this pull/);
});

test('Save CSV and Write to a board still go where they went', async () => {
  const p = await screen();
  const got = {};
  p.shell.saveCsv = (n) => { got.csv = n; };
  p.shell.writeImage = (e) => { got.write = e; };
  await p.press('data-csv', '2026-10-29/logs/2157-ground.log');
  await p.press('data-write', '2026-10-14/images/0912-falcon-2-AAAA0001.bin');
  assert.equal(got.csv, '2026-10-29/logs/2157-ground.log');
  assert.equal(got.write.name, '2026-10-14/images/0912-falcon-2-AAAA0001.bin');
  assert.equal(got.write.uid, 'AAAA0001');
  assert.equal(got.write.from, 'Falcon 2');
});

test('the report panel has its own Open in Replay, Save CSV and Save raw', async () => {
  const p = await screen();
  const html = p.html('data-detail');
  assert.ok(html.includes('<button class="primary" data-open="' + FIRST + '">Open in Replay</button>'));
  assert.ok(html.includes('<button data-csv="' + FIRST + '">Save CSV</button>'));
  assert.match(html, /<a class="button" download="[^"]*dump\.txt" href="[^"]*dump\.txt">Save raw<\/a>/);
});
