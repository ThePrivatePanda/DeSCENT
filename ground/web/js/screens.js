// The screens, and the strip in the top bar that every one of them carries: which
// receivers are being read, the file being logged to, and any job that is running.
(function () {
  'use strict';
  const A = window.DGApp;
  const { $, esc, toast, logUrl } = A.ui;
  const B = DG.boardmodel;
  const D = DG.datamodel;
  const hub = A.hub;

  const SCREENS = ['chipsats', 'tbeams', 'live', 'data', 'replay'];
  const NEED_APP = ['chipsats', 'tbeams', 'data'];
  const parts = {};   // name -> {show(), hide()}, registered by each screen's own file
  let screen = 'live';

  // ---------- drawing without getting in the operator's way ----------
  // These screens redraw from what the app reports, a second at a time. A redraw that
  // lands between a press and its release swallows the click, one under an open select
  // shuts it, and one under a caret throws away what was typed. So a redraw waits for the
  // pointer and the select, and puts typing back.
  const waiting = new Map();   // element -> html not drawn yet
  let pressed = null;

  function paint(el, html) {
    if (el.dgHtml === html && !waiting.has(el)) return;
    const active = document.activeElement;
    const inSelect = active && active.tagName === 'SELECT' && el.contains(active);
    if ((pressed && el.contains(pressed)) || inSelect) { waiting.set(el, html); return; }
    waiting.delete(el);
    const typing = active && el.contains(active) && active.dataset && active.dataset.keep
      ? { key: active.dataset.keep, value: active.value, at: active.selectionStart } : null;
    el.innerHTML = html;
    el.dgHtml = html;
    if (!typing) return;
    const box = [...el.querySelectorAll('[data-keep]')].find((x) => x.dataset.keep === typing.key);
    if (!box) return;
    // Cards keep focus too, so a keyboard user's place survives the 250 ms repaint; they have no value
    if (typing.value !== undefined) box.value = typing.value;
    box.focus({ preventScroll: true });
    try { box.setSelectionRange(typing.at, typing.at); } catch (e) { /* not a text box */ }
  }
  function flush() {
    for (const [el, html] of [...waiting]) paint(el, html);
  }
  document.addEventListener('pointerdown', (e) => { pressed = e.target; }, true);
  document.addEventListener('pointerup', () => { pressed = null; setTimeout(flush, 0); }, true);
  document.addEventListener('pointercancel', () => { pressed = null; setTimeout(flush, 0); }, true);
  document.addEventListener('focusout', () => setTimeout(flush, 0), true);

  // ---------- pieces more than one screen draws ----------
  // A state is a word in a mark. The colour repeats the word and never stands in for it.
  const mark = (cls, word) => '<span class="mark ' + cls + '">' + esc(word) + '</span>';

  const GLYPH = { done: '✓', now: '→', failed: '✕' };
  // A running job as its steps: the finished ones ticked, the current one in bold.
  function steps(job) {
    const list = B.stepList(job.steps, job.running, job.error);
    if (!list.length) return '';
    return '<div class="steps">' + list.map((s) => {
      // The note is the app's latest word on the step it is in, when it says more than
      // the step's own name.
      const more = s.state === 'now' && job.note && job.note !== s.text ? job.note : s.state === 'failed' ? 'failed here' : '';
      return '<div class="' + s.state + '"><span>' + GLYPH[s.state] + '</span><span>' + esc(s.text) + '</span><span>' + esc(more) + '</span></div>';
    }).join('') + '</div>';
  }
  const track = (pct) => '<div class="bar-track"><div class="bar-fill" style="width:' +
    Math.max(0, Math.min(100, Number(pct) || 0)) + '%"></div></div>';

  // A board's name is typed into its own heading, so the box is as wide as the name.
  const nameWidth = (text) => 'width:calc(' + Math.max(5, String(text).length + 1) + 'ch + 14px)';
  document.addEventListener('input', (e) => {
    const t = e.target;
    if (t.classList && t.classList.contains('nameit')) t.style.cssText = nameWidth(t.value || t.placeholder);
  });

  // ---------- which screen is up ----------
  function show(name) {
    // Live and Replay are the dashboard over two different fleets. app.js switches the
    // fleet and comes back through onView, so a log opened from any screen lands here too.
    if (name === 'live' || name === 'replay') { A.showView(name); return; }
    if (!hub.native || !NEED_APP.includes(name)) return;
    setScreen(name);
  }

  function setScreen(name) {
    const was = screen;
    const wasDash = !$('dash').hidden;
    screen = name;
    document.body.dataset.screen = name;
    for (const s of SCREENS) $('view-' + s).setAttribute('aria-selected', String(s === name));
    const dash = name === 'live' || name === 'replay';
    $('dash').hidden = !dash;
    $('replay').hidden = name !== 'replay';
    for (const s of NEED_APP) $('screen-' + s).hidden = s !== name;
    if (was !== name && parts[was] && parts[was].hide) parts[was].hide();
    if (was !== name && parts[name] && parts[name].show) parts[name].show();
    if (dash && !wasDash) A.redraw();
    renderStrip();
  }

  for (const s of SCREENS) $('view-' + s).addEventListener('click', () => show(s));
  A.onView = setScreen;

  // ---------- jobs ----------
  // A pull or a flash runs for minutes inside the app and outlives this page. It is asked
  // for, not pushed, so leaving the screen or reloading the tab picks the same job back up.
  const jobs = { pull: null, chip: null, chipMissing: false, tbeam: null, unseen: null, timer: null, heard: [] };
  const running = () => !!((jobs.pull && jobs.pull.running) || (jobs.chip && jobs.chip.running));

  async function pollJobs() {
    const was = { pull: jobs.pull && jobs.pull.running, chip: jobs.chip && jobs.chip.running };
    try { jobs.pull = await hub.pullState(); } catch (e) { /* the app is restarting; ask again */ }
    // A build with no such call is not asked again every second for the length of a pull.
    if (!jobs.chipMissing) {
      try {
        jobs.chip = await hub.chipJob();
      } catch (e) {
        if (e.missing) { jobs.chip = null; jobs.chipMissing = true; }
      }
    }
    const ended = [];
    if (was.pull && jobs.pull && !jobs.pull.running) ended.push('pull');
    if (was.chip && jobs.chip && !jobs.chip.running) ended.push('chip');
    for (const kind of ended) {
      const j = jobs[kind];
      // What a finished job left is on the ChipSats screen. Until that has been looked
      // at the bar keeps saying so: a failed pull can leave a board on the wrong firmware.
      if (screen !== 'chipsats') jobs.unseen = { kind, failed: !!j.error, what: kind === 'pull' ? 'pull' : (j.what || 'flash') };
    }
    if (running() && !jobs.timer) jobs.timer = setInterval(pollJobs, 1000);
    if (!running() && jobs.timer) { clearInterval(jobs.timer); jobs.timer = null; }
    for (const fn of jobs.heard) fn(ended);
    renderStrip();
  }

  // ---------- the strip ----------
  function jobChip() {
    const name = shell.boardName ? shell.boardName() : '';
    const from = name ? ' from ' + name : '';
    const on = name ? ' ' + name : ' a ChipSat';
    if (jobs.pull && jobs.pull.running) return { to: 'chipsats', html: 'Pulling the flight log' + esc(from) + ', ' + (jobs.pull.pct || 0) + '%' };
    if (jobs.chip && jobs.chip.running) {
      const doing = jobs.chip.what === 'read' ? 'Reading the board' : 'Flashing' + esc(on);
      return { to: 'chipsats', html: doing + ', ' + (jobs.chip.pct || 0) + '%' };
    }
    if (jobs.tbeam) return { to: 'tbeams', html: 'Flashing ' + esc(jobs.tbeam.name) + ', ' + (jobs.tbeam.pct || 0) + '%' };
    if (jobs.unseen) {
      const what = { pull: 'The pull', read: 'The read' }[jobs.unseen.what] || 'The flash';
      return { to: 'chipsats', html: esc(what) + ' ' + (jobs.unseen.failed ? mark('bad', 'failed') : mark('good', 'finished')) };
    }
    return null;
  }

  function renderStrip() {
    const native = hub.native;
    $('strip').hidden = !native;
    if (!native) return;

    // On Live the receiver chips to the left already say this, with their controls.
    $('strip-rx').hidden = screen === 'live';
    const rx = B.receiverWords(hub.ports, hub.boards, A.live.receivers, Date.now());
    paint($('strip-rx'), rx.length
      ? rx.slice(0, 3).map((r) => '<span class="strip-one"><b>' + esc(r.name) + '</b>' + (r.sf ? ' ' + r.sf : '') + ' ' + mark(r.cls, r.word) + '</span>').join('') +
        (rx.length > 3 ? '<span class="strip-one">and ' + (rx.length - 3) + ' more</span>' : '')
      : mark('off', 'no receiver being read'));

    const name = D.baseName(hub.log);
    $('strip-log').hidden = false;
    $('strip-log-name').innerHTML = name ? '<span class="dot on"></span><span class="lead">Logging to </span><b>' + esc(name) + '</b>'
      : '<span class="dot"></span>' + mark('warn', 'not logging to a file');
    $('strip-log-name').title = hub.log ? 'Logging to ' + hub.log : '';
    $('strip-log-save').hidden = !name;
    if (name) { $('strip-log-save').href = logUrl(D.dataName(hub.log)); $('strip-log-save').download = name; }

    // Every build carries the same version number, so the bar says when the one on disk
    // is newer than the one running.
    $('strip-build').hidden = !(build.health && build.health.newerOnDisk === true);
    // A release newer than this one. Said here and nowhere else, and it stops nothing.
    $('strip-release').hidden = !build.release;
    if (build.release) {
      $('strip-release-version').textContent = build.release.version;
      $('strip-release-link').href = build.release.url;
      $('strip-release-link').title = 'Opens the release on GitHub. This is version ' + build.health.version + '.';
    }

    const chip = jobChip();
    $('strip-job').hidden = !chip;
    if (chip) { paint($('strip-job'), chip.html); $('strip-job').dataset.to = chip.to; }
  }

  $('strip-rx').addEventListener('click', () => show('tbeams'));
  // A link that saves a file has no handler of its own to say so.
  document.addEventListener('click', (e) => {
    const a = e.target.closest && e.target.closest('a[download]');
    if (a && a.isConnected) toast('Saved ' + a.getAttribute('download'));
  });
  $('strip-job').addEventListener('click', () => {
    const to = $('strip-job').dataset.to;
    // Before the screen is shown: showing it counts the job as seen.
    if (to === 'chipsats' && shell.showJobBoard) shell.showJobBoard();
    show(to);
  });
  $('strip-log-new').addEventListener('click', async () => {
    try {
      const j = await hub.rotateLog();
      toast('New file ' + D.baseName(j.log));
    } catch (e) {
      toast('Could not start a new file: ' + (e.message || e));
    }
    renderStrip();
  });

  // ---------- which build is running ----------
  // Asked once a minute at most. An older build does not say, and then nothing is shown.
  const build = { health: null, release: null };
  async function askBuild() {
    try { build.health = await hub.health(); } catch (e) { return; }
    const words = D.builtWords(build.health, new Date());
    if (words) document.querySelector('.brand').title = words + (build.health.version ? ', version ' + build.health.version : '');
    $('built').textContent = words;
    renderStrip();
    askRelease();
  }

  // Whether a newer release is out, see update.js. It keeps its own answer for hours, so
  // being called every minute does not mean GitHub is asked every minute.
  async function askRelease() {
    if (!build.health || !build.health.version || !DG.update) return;
    let storage = null;
    try { storage = localStorage; } catch (e) { /* storage blocked */ }
    try {
      build.release = await DG.update.check({ fetch: (url, o) => fetch(url, o), storage, now: Date.now(), running: build.health.version });
    } catch (e) {
      build.release = null;
    }
    renderStrip();
  }

  // ---------- the app's screens ----------
  // The app writes every line to disk on its own, so its bar names that file and Save a
  // copy there is the session's download.
  function applyTransport() {
    const native = hub.native;
    for (const s of NEED_APP) $('view-' + s).hidden = !native;
    if (!native && NEED_APP.includes(screen)) A.showView('live');
    renderStrip();
  }
  A.onTransport = applyTransport;

  // First run with boards plugged in and none of them claimed: show where they are
  // claimed, because an empty dashboard with no explanation is the worst of both worlds.
  let offered = false;
  A.onReceivers = () => {
    if (offered || !hub.native) return;
    const boards = hub.boards || [];
    if (!boards.length || boards.some((b) => b.state === 'receiver')) return;
    offered = true;
    if (screen === 'live') show('tbeams');
  };

  const shell = {
    show, paint, mark, steps, track, nameWidth, jobs, pollJobs, renderStrip,
    current: () => screen,
    register: (name, part) => { parts[name] = part; },
    onJobs: (fn) => { jobs.heard.push(fn); },
    seen: () => { jobs.unseen = null; renderStrip(); },
    // Set by the ChipSats screen: the name of the board a job is on, and how to show it.
    boardName: null,
    showJobBoard: null,
  };
  A.shell = shell;

  applyTransport();
  setInterval(renderStrip, 1000);
  if (hub.native) {
    pollJobs();
    askBuild();
    setInterval(askBuild, 60000);
    // ?screen=data opens on that screen, the way ?tab= picks a graph tab.
    // Once the page has loaded, so the screen's own file is there to draw it.
    const wanted = new URLSearchParams(location.search).get('screen');
    if (NEED_APP.includes(wanted)) document.addEventListener('DOMContentLoaded', () => show(wanted));
  }
})();
