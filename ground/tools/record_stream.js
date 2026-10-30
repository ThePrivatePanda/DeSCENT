#!/usr/bin/env node
// Records a replay's stream off a running app, one JSON message a line, for gate 2's
// fixtures (tools/compare_pages.js reads them).
//
//   node tools/record_stream.js <app base url> <log path as the app sees it> <out.jsonl>
//   e.g. node tools/record_stream.js http://127.0.0.1:8791 2026-10-17/logs/1402-ground.log stream-1402-ground.jsonl
//
// Opens the log as a replay, listens on its socket, plays it as fast as the app will, and
// stops once the app says it is at the last line and nothing has arrived for 2 s. A replay
// that opens at its end is sent back to 0 first, as the page's Play does. Node 22, no
// dependencies: its own fetch and WebSocket.
const fs = require('node:fs');

const FAST = 1000000;   // replay speed: as fast as the app reads

// o: {base, path, out, fetch, WebSocket, quietMs, pollMs, stallMs, log}
async function record(o) {
  const fetchFn = o.fetch || fetch;
  const WS = o.WebSocket || WebSocket;
  const quietMs = o.quietMs === undefined ? 2000 : o.quietMs;
  const pollMs = o.pollMs === undefined ? 250 : o.pollMs;
  const stallMs = o.stallMs === undefined ? 60000 : o.stallMs;
  const base = String(o.base).replace(/\/+$/, '');

  async function call(path, body, method) {
    const r = await fetchFn(base + path, { method: method || 'POST', body: body === undefined ? undefined : JSON.stringify(body) });
    const j = await r.json().catch(() => null);
    if (!r.ok || (j && j.ok === false)) throw new Error((method || 'POST') + ' ' + path + ': ' + ((j && j.error) || 'HTTP ' + r.status));
    return j || {};
  }

  const opened = await call('/api/replay/open?' + new URLSearchParams({ path: o.path }).toString());
  if (opened.id === undefined || opened.id === null) throw new Error('the app gave no replay id');
  const id = encodeURIComponent(String(opened.id));
  const url = base.replace(/^http/, 'ws') + '/api/replay/' + id + '/stream';

  const out = fs.createWriteStream(o.out);
  let count = 0; let first = null; let last = null; let heardAt = Date.now();
  let failed = null; let closed = false;
  let snapshot = null;
  const snapshotDone = new Promise((resolve) => { snapshot = resolve; });

  const ws = new WS(url);
  ws.onmessage = (e) => {
    let m;
    try { m = JSON.parse(typeof e.data === 'string' ? e.data : String(e.data)); } catch (err) {
      failed = failed || new Error('a frame that is not JSON after ' + count + ' messages');
      return;
    }
    out.write(JSON.stringify(m) + '\n');
    count++;
    heardAt = Date.now();
    if (typeof m.seq === 'number') { if (first === null) first = m.seq; last = m.seq; }
    if (m.kind === 'snapshot-end') snapshot();
  };
  ws.onerror = (e) => { failed = failed || new Error('socket ' + url + ': ' + ((e && e.message) || 'error')); };
  ws.onclose = () => { closed = true; };

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const finish = async () => {
    ws.onclose = null;
    try { ws.close(); } catch (e) { /* gone */ }
    await call('/api/replay/' + id + '/close').catch(() => null);
    await new Promise((r) => out.end(r));
  };

  try {
    // the snapshot first, so play starts from a known state
    while (!failed && !closed) {
      const got = await Promise.race([snapshotDone.then(() => true), sleep(pollMs).then(() => false)]);
      if (got) break;
      if (Date.now() - heardAt > stallMs) throw new Error('no snapshot-end from ' + url + ' in ' + stallMs + ' ms');
    }
    if (failed) throw failed;
    if (closed) throw new Error('the app closed ' + url + ' before its snapshot ended');
    const st = await call('/api/replay/' + id, undefined, 'GET');
    if (Number(st.lines) > 0 && Number(st.position) >= Number(st.lines)) await call('/api/replay/' + id + '/seek', { position: 0 });
    await call('/api/replay/' + id + '/speed', { speed: FAST });
    await call('/api/replay/' + id + '/play');
    for (;;) {
      await sleep(pollMs);
      if (failed) throw failed;
      if (closed) throw new Error('the app closed ' + url + ' during the replay, after ' + count + ' messages');
      const s = await call('/api/replay/' + id, undefined, 'GET');
      const quiet = Date.now() - heardAt;
      if (Number(s.position) === Number(s.lines) && quiet >= quietMs) break;
      if (quiet > stallMs) throw new Error('nothing on ' + url + ' for ' + stallMs + ' ms, at line ' + s.position + ' of ' + s.lines);
    }
  } catch (e) {
    await finish().catch(() => null);
    throw e;
  }
  await finish();
  return { count, first, last, id: opened.id };
}

async function main(argv) {
  if (argv.length !== 3) {
    process.stderr.write('usage: node tools/record_stream.js <app base url> <log path as the app sees it> <out.jsonl>\n');
    return 2;
  }
  const [base, path, out] = argv;
  try {
    const r = await record({ base, path, out });
    process.stderr.write(r.count + ' messages, seq ' + r.first + '..' + r.last + ', replay ' + r.id + ', written to ' + out + '\n');
    return 0;
  } catch (e) {
    process.stderr.write('record_stream: ' + (e.message || e) + '\n');
    return 1;
  }
}

if (require.main === module) main(process.argv.slice(2)).then((code) => { process.exitCode = code; });

module.exports = { record, main, FAST };
