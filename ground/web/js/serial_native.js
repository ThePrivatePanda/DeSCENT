// The native app reads the ports itself and pushes lines over a WebSocket.
// Same shape as SerialHub, so app.js does not care which one it got.
(function (root, factory) {
  const mod = factory(root);
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else Object.assign(root.DG = root.DG || {}, mod);
})(typeof self !== 'undefined' ? self : globalThis, function (root) {
  'use strict';

  const RETRY_MS = 1000;

  // The whole transport rule. Not a fetch probe: from file:// a probe rejects instead of
  // answering, and app.js builds its hub synchronously.
  function isNativeHost(loc) {
    return loc.protocol === 'http:' && (loc.hostname === '127.0.0.1' || loc.hostname === 'localhost');
  }

  // One WebSocket frame. Boot noise or half a message must not take the socket down.
  function makeFrameHandler(onLine, onPorts, onFlash) {
    return function (data) {
      let m;
      try { m = JSON.parse(data); } catch (e) { return; }
      if (!m || typeof m !== 'object') return;
      if (m.type === 'line') onLine(m.rx, m.text);
      else if (m.type === 'ports') onPorts();
      else if (m.type === 'flash' && onFlash) onFlash(m);
    };
  }

  // A build of the app from before a call existed answers 404 with no JSON. That is said
  // in words on the screen, so it is told apart from the app refusing.
  async function answered(r) {
    const j = await r.json().catch(() => null);
    if (j) return j;
    const e = new Error(r.status === 404 ? 'this build of the app cannot do that yet' : 'HTTP ' + r.status);
    e.missing = r.status === 404;
    throw e;
  }

  class NativeHub {
    // onLine(rxKey, text), onChange() whenever a port opens, closes or errors.
    // note(msg) is only used if this turns out not to be our app after all.
    constructor(onLine, onChange, note) {
      this.onLine = onLine;
      this.onChange = onChange;
      this.note = note || function () {};
      this.found = [];   // {key, status, error, usb}
      this.boards = []; // every usb serial port, and what we do about it
      this.log = null;
      this.web = null;   // a SerialHub, once we know the server on this port is not ours
      this.onFlash = null;   // the setup panel sets this while it is open
      this.frame = makeFrameHandler(onLine, () => this.refresh(), (m) => { if (this.onFlash) this.onFlash(m); });
      this.refresh();
      this.connect();
    }

    get ports() { return this.web ? this.web.ports : this.found; }
    get native() { return !this.web; }
    get supported() { return this.web ? this.web.supported : true; }

    connect() {
      if (this.web) return;
      const ws = new WebSocket('ws://' + location.host + '/api/stream');
      ws.onmessage = (e) => this.frame(e.data);
      // The app outlives the tab and can be restarted under it, so keep trying.
      ws.onclose = () => setTimeout(() => this.connect(), RETRY_MS);
      ws.onopen = () => this.refresh();
      this.ws = ws;
    }

    async refresh() {
      try {
        const r = await fetch('/api/ports');
        if (!r.ok) throw new Error('HTTP ' + r.status);
        const j = await r.json();
        if (!Array.isArray(j.ports)) throw new Error('not our /api/ports');
        this.found = j.ports;
        this.boards = j.boards || [];
        this.log = j.log || null;
      } catch (err) {
        this.fall(err);
        return;
      }
      this.onChange();
    }

    // Some other local web server answered. Hand the page back to Web Serial rather than
    // leave it with no way to reach a receiver.
    fall(err) {
      if (this.web) return;
      this.found = [];
      this.web = new root.DG.SerialHub(this.onLine, this.onChange);
      try { if (this.ws) this.ws.close(); } catch (e) { /* never opened */ }
      this.note('No receivers here (' + (err.message || err) + '). Using the browser instead.');
      this.onChange();
    }

    // Discovery is automatic, so Connect is a rescan. Nothing found throws what a
    // cancelled Web Serial picker throws, which app.js already ignores.
    async add() {
      if (this.web) return this.web.add();
      await this.refresh();
      if (!this.ports.length) throw { name: 'NotFoundError', message: 'no receiver found' };
      return this.ports[0];
    }

    async close(p) {
      if (this.web) return this.web.close(p);
      await fetch('/api/receiver/' + p.key + '/close', { method: 'POST' });
      await this.refresh();
    }

    // Say what a board is. Until this, it is left alone: opening a port pulses DTR
    // and can reset whatever is on the other end, and the chip cannot tell a T-Beam
    // from a ChipSat. Remembered by board, so a replug needs no second answer.
    async setBoard(port, isReceiver) {
      const where = isReceiver ? '/api/board/receiver' : '/api/board/ignore';
      const r = await fetch(where, { method: 'POST', body: JSON.stringify({ port }) });
      const j = await r.json().catch(() => ({ ok: false }));
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      await this.refresh();
    }

    // Every T-Beam-shaped port, whether or not we are reading it: reflashing a
    // working receiver is the normal case.
    async candidates() {
      const r = await fetch('/api/flash/candidates');
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    }

    // Writes the firmware in the app unless image names a file on this machine.
    async flash(port, sf, image, keepBackup) {
      const body = { port, sf: Number(sf) };
      if (image) body.image = image;
      body.backup = keepBackup ? 1 : 0;
      const r = await fetch('/api/flash', { method: 'POST', body: JSON.stringify(body) });
      const j = await r.json().catch(() => ({ ok: false, error: 'HTTP ' + r.status }));
      if (!j.ok) throw new Error(j.error || 'flash failed');
      return j;
    }

    // json.rs reads a string up to the first quote and does not unescape, so a quote or
    // a backslash would arrive truncated. Take them out here rather than corrupt the file.
    async setName(port, name) {
      const clean = String(name).replace(/["\\\r\n]/g, '').trim().slice(0, 40);
      const r = await fetch('/api/board/name', { method: 'POST', body: JSON.stringify({ port, name: clean }) });
      const j = await r.json().catch(() => ({ ok: false }));
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      await this.refresh();
      return clean;
    }

    // Drops what the app keeps about a serial board: whether to read it, and its names.
    // Nothing is sent to the board.
    async forgetBoard(port) {
      const r = await fetch('/api/board/forget', { method: 'POST', body: JSON.stringify({ port }) });
      const j = await answered(r);
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      await this.refresh();
    }

    // A name kept against the receiver's own id, so it follows the board to any port.
    // onBoard says the name was also sent to the board, which newer receiver firmware
    // keeps; the board's own report of it arrives with the next ports frame.
    async setReceiverName(id, name) {
      const clean = String(name).replace(/["\\\r\n]/g, '').trim().slice(0, 24);
      const r = await fetch('/api/receiver/name', { method: 'POST', body: JSON.stringify({ id, name: clean }) });
      const j = await answered(r);
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      await this.refresh();
      return { name: clean, onBoard: !!j.onBoard };
    }

    // Nothing is opened for this: the app watches which port disappears and comes back.
    async identifyStart() {
      const r = await fetch('/api/board/identify', { method: 'POST' });
      return r.json();
    }

    // A build without the call answers 404; the page then just stops asking.
    async identifyStop() {
      const r = await fetch('/api/board/identify/stop', { method: 'POST' });
      return answered(r);
    }

    async identifyState() {
      const r = await fetch('/api/board/identify');
      return r.json();
    }

    // Opens the port and listens without writing to it. Five seconds.
    async testPort(port) {
      const r = await fetch('/api/board/test', { method: 'POST', body: JSON.stringify({ port }) });
      const j = await r.json().catch(() => ({ ok: false }));
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      return j;
    }

    // Pull a flight log off a chip. port is the board's serial console; keepFsw says
    // whether the flight software now on the board is saved and put back. more names the
    // probe and the board the sheet was showing: {probe, uid}, and wipe: 1 clears the
    // chip instead of reading it (the flight software always goes back on then).
    async pull(port, keepFsw, more) {
      const r = await fetch('/api/pull', {
        method: 'POST',
        body: JSON.stringify(Object.assign({ port, keepFsw: keepFsw ? 1 : 0 }, more)),
      });
      const j = await r.json().catch(() => ({ ok: false }));
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      return j;
    }

    // pull keeps what arrived and puts the flight software back
    async pullStop() {
      const r = await fetch('/api/pull/stop', { method: 'POST' });
      const j = await r.json().catch(() => ({ ok: false }));
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      return j;
    }

    // Asked for rather than pushed, so closing the panel or reloading the tab during a
    // pull that takes minutes does not lose it.
    async pullState() {
      const r = await fetch('/api/pull/state');
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    }

    // Every log sitting beside the app, newest first.
    async logs() {
      const r = await fetch('api/logs');
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return (await r.json()).logs || [];
    }

    // ---------- ChipSats, over the debug probe ----------
    // Listing probes and remembered boards attaches to nothing.
    async chipProbes() {
      return (await answered(await fetch('/api/chip/probes'))).probes || [];
    }

    async chips() {
      return (await answered(await fetch('/api/chips'))).chips || [];
    }

    // Which board is under the clip, asked without the reset a read can fall back to.
    // null when nothing answered.
    async chipPeek(probe) {
      const j = await answered(await fetch('/api/chip/peek', { method: 'POST', body: JSON.stringify(probe ? { probe } : {}) }));
      return j.uid || null;
    }

    // Attaches the probe, which can reset the board. Called from a button, and once
    // when a board is first seen under the clip.
    async chipRead(probe) {
      const r = await fetch('/api/chip/read', { method: 'POST', body: JSON.stringify(probe ? { probe } : {}) });
      const j = await answered(r);
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      return j;
    }

    // Same cleaning as setName, for the same reason.
    async chipSetName(uid, name) {
      const clean = String(name).replace(/["\\\r\n]/g, '').trim().slice(0, 40);
      const r = await fetch('/api/chip/name', { method: 'POST', body: JSON.stringify({ uid, name: clean }) });
      const j = await answered(r);
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      return clean;
    }

    // Drops a ChipSat from the list the app keeps. Nothing is done to the board.
    async chipForget(uid) {
      const r = await fetch('/api/chip/forget', { method: 'POST', body: JSON.stringify({ uid }) });
      const j = await answered(r);
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
    }

    async chipSetModule(uid, module) {
      const r = await fetch('/api/chip/module', { method: 'POST', body: JSON.stringify({ uid, module }) });
      const j = await answered(r);
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
    }

    // One of three images: bytes (a file the operator picked, with its name), saved (an
    // image the app read off a board earlier, by its name in logs()), or embedded: 'built'
    // (the flight software compiled for this board) or 'flight' (the image the app
    // carries, with the board's settings written into its block). uid is the board the
    // sheet is showing; the app checks the board under the probe is still that one
    // before it writes.
    async chipFlash(o) {
      const q = new URLSearchParams({ uid: o.uid, backup: o.backup ? '1' : '0' });
      if (o.saved) q.set('saved', o.saved);
      else if (o.embedded) q.set('embedded', o.embedded);
      else q.set('name', o.name || 'image.bin');
      if (o.force) q.set('force', '1');
      // Either way the board's settings go on the query. The module goes only with a
      // build: the carried image is picked by the module the app has for the board.
      if (o.embedded === 'bench' && o.module) q.set('module', o.module);
      if (o.embedded === 'built' || o.embedded === 'flight') {
        const keys = [['module', o.module], ['csid', o.csid], ['sf', o.sf], ['dbm', o.dbm]];
        // One rate per phase, waiting, rising, falling, landed: fixed0..3 in ms, follow0..3
        // as 0 or 1, soc0..3 the thresholds and socms0..3 the intervals, comma separated,
        // log0..3 the flash log gap in ms; logblind the gap with no altitude source.
        for (let p = 0; p < 4; p++) {
          for (const k of ['fixed', 'follow', 'soc', 'socms', 'log']) keys.push([k + p, o[k + p]]);
        }
        keys.push(['logblind', o.logblind]);
        for (const [key, value] of keys) {
          if (value !== null && value !== undefined && value !== '') q.set(key, String(value));
        }
      }
      const r = await fetch('/api/chip/flash?' + q.toString(), { method: 'POST', body: o.bytes || null });
      const j = await answered(r);
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      return j;
    }

    // sends R, the bench toggles its radio
    async benchRadio(port) {
      const j = await answered(await fetch('/api/bench/radio?' + new URLSearchParams({ port }), { method: 'POST' }));
      if (!j.ok) throw new Error(j.error || 'the board was not told');
      return j;
    }

    // Asked for rather than pushed, like pullState.
    async chipJob() {
      return answered(await fetch('/api/chip/job'));
    }

    // The powers and spreading factors the flight software accepts, written by its build.
    async radio() {
      return answered(await fetch('/api/radio'));
    }

    async health() {
      return answered(await fetch('/api/health'));
    }

    async fsw() {
      return answered(await fetch('/api/fsw'));
    }

    // A pull's report.json, by the name logs() gives the pull's folder.
    async report(name) {
      const r = await fetch('/api/log/' + String(name).split('/').map(encodeURIComponent).join('/'));
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    }

    async rotateLog() {
      const r = await fetch('/api/log/rotate', { method: 'POST' });
      const j = await r.json().catch(() => ({ ok: false }));
      if (!j.ok) throw new Error(j.error || 'HTTP ' + r.status);
      await this.refresh();
      return j;
    }

    async setSf(key, sf) {
      const r = await fetch('/api/receiver/' + key + '/config', {
        method: 'POST',
        body: JSON.stringify({ sf: Number(sf) }),
      });
      if (!r.ok) throw new Error('receiver refused SF' + sf + ' (HTTP ' + r.status + ')');
    }
  }

  function makeHub(onLine, onChange, note) {
    if (isNativeHost(location)) return new NativeHub(onLine, onChange, note);
    return new root.DG.SerialHub(onLine, onChange);
  }

  return { NativeHub, makeHub, makeFrameHandler, isNativeHost };
});
