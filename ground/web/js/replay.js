// A log opened in the app for replay. The app reads the file and runs its own fleet over it;
// the page drives it here and draws it from the replay's own stream, never the live one.
(function (root, factory) {
  const S = typeof require === 'function' ? require('./stream.js') : root.DG.stream;
  const mod = factory(S);
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).replay = mod;
})(typeof self !== 'undefined' ? self : this, function (S) {
  'use strict';

  // A build of the app without the call answers 404 with no JSON, said in words
  async function answered(r) {
    const j = await r.json().catch(() => null);
    if (j && j.ok !== false) return j;
    const e = new Error((j && j.error) || (r.status === 404 ? 'this build of the app cannot do that yet' : 'HTTP ' + r.status));
    e.missing = r.status === 404;
    throw e;
  }

  // The state the app gives, under the names the replay bar reads. from is the time of
  // the log's first line; a {log, raw, dir} object there is where the file is kept.
  function stateFrom(j) {
    const n = (v) => (Number.isFinite(Number(v)) ? Number(v) : 0);
    return {
      position: n(j.position), lines: n(j.lines), packets: n(j.packets),
      from: typeof j.from === 'number' ? j.from : null,
      kept: j.from && typeof j.from === 'object' ? j.from : null,
      t: typeof j.t === 'number' ? j.t : typeof j.clock === 'number' ? j.clock : null,
      playing: !!j.playing, speed: j.speed === undefined ? null : Number(j.speed),
    };
  }

  class Replay {
    // o: {fetch, WebSocket, setTimeout, clearTimeout, location, cfg, onChange, onReady, log}
    constructor(id, o) {
      this.id = String(id);
      this.o = o;
      this.state = stateFrom({});
      this.base = '/api/replay/' + encodeURIComponent(this.id);
      const loc = o.location || { protocol: 'http:', host: 'localhost' };
      this.stream = new S.StreamClient(S.wsUrl(loc, this.base + '/stream'), {
        WebSocket: o.WebSocket, setTimeout: o.setTimeout, clearTimeout: o.clearTimeout, fetch: o.fetch,
        // A log is finite and on disk: its graphs keep everything, and a seek starts again
        cfg: Object.assign({}, o.cfg, { maxPoints: Infinity, retainMin: 0 }), keepHistory: false,
        historyUrl: (label) => this.base + '/history?label=' + encodeURIComponent(label),
        onChange: o.onChange, onReady: o.onReady, log: o.log,
      });
    }

    get fleet() { return this.stream.fleet; }

    // what: {path} for a file the app wrote, or {files: [{name, size, body}]} for files picked
    // on this computer. Several files go up in one body, cut by their sizes, and the app
    // merges them by time as opening several did before.
    static async open(o, what) {
      const q = new URLSearchParams();
      let body = null;
      if (what.path) q.set('path', what.path);
      else {
        for (const f of what.files) { q.append('name', f.name); q.append('size', String(f.size)); }
        body = what.files.length === 1 ? what.files[0].body : o.join(what.files.map((f) => f.body));
      }
      const r = await o.fetch('/api/replay/open?' + q.toString(), { method: 'POST', body });
      const j = await answered(r);
      if (j.id === undefined || j.id === null) throw new Error(j.error || 'the app gave no replay id');
      return new Replay(j.id, o).start();
    }

    start() { this.stream.start(); return this; }

    async post(action, body) {
      const r = await this.o.fetch(this.base + '/' + action, { method: 'POST', body: body ? JSON.stringify(body) : null });
      return answered(r);
    }

    play() { return this.post('play'); }
    pause() { return this.post('pause'); }
    speed(n) { return this.post('speed', { speed: Number(n) }); }

    // The app's fleet starts again from the new place, so the stream is taken again
    async seek(position) {
      const j = await this.post('seek', { position: Math.max(0, Math.round(Number(position) || 0)) });
      this.stream.reconnect();
      return j;
    }

    async poll() {
      const r = await this.o.fetch(this.base);
      if (!r.ok) throw new Error('HTTP ' + r.status);
      this.state = stateFrom(await r.json());
      return this.state;
    }

    // The replay's clock: the app's, else the newest time on its stream
    clock() {
      if (this.state.t !== null) return this.state.t;
      let t = this.state.from;
      for (const u of this.fleet.allUnits().values()) if (u.lastT !== null && !(u.lastT <= t)) t = u.lastT;
      return t;
    }

    get csvUrl() { return this.base + '/csv'; }
    get logUrl() { return this.base + '/log'; }

    // The app keeps a replay until told; nothing here waits for the answer
    close() {
      this.stream.stop();
      return this.o.fetch(this.base + '/close', { method: 'POST' }).catch(() => null);
    }
  }

  return { Replay, stateFrom, answered };
});
