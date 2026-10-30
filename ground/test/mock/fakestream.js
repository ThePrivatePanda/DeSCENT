// The app's stream without the app: a WebSocket that delivers the messages a test gives
// it, timers run by hand for the reconnect backoff, and a reader for the master's recorded
// stream fixtures (JSON lines, one message each).
const fs = require('node:fs');
const path = require('node:path');

// A fake WebSocket class and the timers a StreamClient is given. Every socket opened is in
// net.sockets, newest last.
function fakeNet() {
  const net = { sockets: [], timers: [], urls: [] };
  class FakeSocket {
    constructor(url) {
      this.url = url;
      this.closed = false;
      net.urls.push(url);
      net.sockets.push(this);
    }
    // what the app does on connect, then each message as a frame
    open() { if (this.onopen) this.onopen(); return this; }
    send(...msgs) {
      for (const m of msgs.flat()) if (this.onmessage) this.onmessage({ data: typeof m === 'string' ? m : JSON.stringify(m) });
      return this;
    }
    // the app went away
    drop() { this.closed = true; if (this.onclose) this.onclose(); }
    close() { this.closed = true; if (this.onclose) this.onclose(); }
  }
  net.WebSocket = FakeSocket;
  net.setTimeout = (fn, ms) => { net.timers.push({ fn, ms }); return net.timers.length; };
  net.clearTimeout = (id) => { if (net.timers[id - 1]) net.timers[id - 1].fn = null; };
  net.last = () => net.sockets[net.sockets.length - 1];
  // Run the timers set so far, oldest first; returns the waits they asked for
  net.runTimers = () => {
    const due = net.timers.splice(0);
    for (const t of due) if (t.fn) t.fn();
    return due.map((t) => t.ms);
  };
  return net;
}

// Numbered the way the app numbers them, from 1, unless a message has its own seq
function numbered(msgs, from) {
  let seq = from === undefined ? 1 : from;
  return msgs.map((m) => (m.seq === undefined ? Object.assign({ seq: seq++ }, m) : (seq = m.seq + 1, m)));
}

function loadJsonl(file) {
  return fs.readFileSync(file, 'utf8').split(/\r?\n/).filter((l) => l.trim()).map((l, i) => {
    try { return JSON.parse(l); } catch (e) { throw new Error(file + ':' + (i + 1) + ': ' + e.message); }
  });
}

// Recorded streams of the app: the head of one in test/data, and the master's whole ones,
// docs/internals/frontend/fixtures/stream-*.jsonl (gitignored), in this checkout or in the
// main one this worktree hangs off.
const HEAD = path.join(__dirname, '..', 'data', 'stream-1402-head.jsonl');
function fixtures() {
  const repo = path.resolve(__dirname, '..', '..', '..');
  const roots = [repo];
  const at = repo.indexOf(path.sep + '.worktrees' + path.sep);
  if (at > 0) roots.push(repo.slice(0, at));
  const out = fs.existsSync(HEAD) ? [HEAD] : [];
  for (const r of roots) {
    const dir = path.join(r, 'docs', 'internals', 'frontend', 'fixtures');
    if (!fs.existsSync(dir)) continue;
    for (const f of fs.readdirSync(dir)) if (/^stream-.*\.jsonl$/.test(f)) out.push(path.join(dir, f));
  }
  return out;
}

module.exports = { fakeNet, numbered, loadJsonl, fixtures, HEAD };
