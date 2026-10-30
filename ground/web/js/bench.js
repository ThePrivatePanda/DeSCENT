// Sensor bench console per port, for the Sensor bench pane on ChipSats. Values stay as the text the board printed.
// Lines and packets come from the app's stream: {kind: "line"} for the console, {kind: "packet"} for the radio
(function (root, factory) {
  const mod = factory();
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).bench = mod;
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  const LINE = /^([EWIDV]) ([A-Z]+) (\d+) c(\d+): (\S+)(.*)$/;
  const RAW_KEPT = 5000;          // lines of raw console the screen can scroll back through
  const GAP_MS = 3000;            // heartbeat is 1 s, so a gap this long is the port, not the board
  const LOOP_BOOTS = 3;           // starts inside LOOP_MS that count as a boot loop
  const LOOP_MS = 60000;
  const BENCH_CSID = 254;         // so bench packets never look like a flight board's
  // bookkeeping tags, not parts that can stop answering
  const NOT_PARTS = new Set(['BOOT', 'RST', 'PKT', 'PHASE']);

  // null for boot noise and half lines
  function parse(text) {
    const m = LINE.exec(text.trim());
    if (!m) return null;
    const [, level, tag, ms, cycle, event, rest] = m;
    const line = { level, tag, ms: Number(ms), cycle: Number(cycle), event, fields: {} };
    if (tag === 'PKT') {
      line.hex = (event + rest).trim();
      return line;
    }
    for (const word of rest.trim().split(/\s+/)) {
      const at = word.indexOf('=');
      if (at > 0) line.fields[word.slice(0, at)] = word.slice(at + 1);
    }
    return line;
  }

  // A digit lost on the wire can drop the clock on one line, but hardly ever the count too
  function wentBack(was, now) {
    return now.ms + 1000 < was.ms && (now.cycle < was.cycle || (now.cycle === 0 && was.cycle > 0));
  }

  class Session {
    constructor() {
      this.raw = [];          // {t, text, line}
      this.parts = new Map(); // tag -> part
      this.boots = [];        // {t, fields, cause, before, upMs, bench}
      this.gaps = [];         // {from, to}, port went quiet, not a restart
      this.lastT = null;
      this.last = null;       // for what came before a start
      this.bench = null;      // test=bench on the last start line
      this.heard = 0;
      this.other = 0;         // lines not in the log format
    }

    add(t, text) {
      const line = parse(text);
      this.raw.push({ t, text, line });
      if (this.raw.length > RAW_KEPT) this.raw.splice(0, this.raw.length - RAW_KEPT);
      if (this.lastT !== null && t - this.lastT > GAP_MS) this.gaps.push({ from: this.lastT, to: t });
      this.lastT = t;
      this.heard++;
      if (!line) { this.other++; return null; }

      if (line.tag === 'BOOT' && line.event === 'start') {
        this.boots.push({
          t, fields: line.fields, cause: null, bench: line.fields.test === 'bench',
          before: this.last ? this.last.text : null,
          upMs: this.last ? this.last.line.ms : null,
        });
        this.bench = line.fields.test === 'bench';
        this.parts = new Map();
      } else if (line.tag === 'RST' && this.boots.length) {
        this.boots[this.boots.length - 1].cause = line.fields;
      } else if (this.last && wentBack(this.last.line, line)) {
        // start line lost on the wire
        const latest = this.boots[this.boots.length - 1];
        if (!latest || t - latest.t > 2000) {
          this.boots.push({ t, fields: {}, cause: null, bench: null, before: this.last.text, upMs: this.last.line.ms, unseen: true });
          this.parts = new Map();
        }
      }
      if (!NOT_PARTS.has(line.tag)) this.part(line.tag).add(t, line, text);
      this.last = { text, line };
      return line;
    }

    part(tag) {
      if (!this.parts.has(tag)) this.parts.set(tag, new Part(tag));
      return this.parts.get(tag);
    }

    looping(now) {
      const recent = this.boots.filter((b) => now - b.t <= LOOP_MS);
      return recent.length >= LOOP_BOOTS ? recent.length : 0;
    }
  }

  const RECENT = 6;

  // IMU, ENV, GPS, SOC, BUS, FLASH, RADIO, TX, CYC
  class Part {
    constructor(tag) {
      this.tag = tag;
      this.init = null;       // init line fields, as printed
      this.events = new Map();// event -> {count, fields, t, ms, gaps: [ms between], n: [[ms, n]]}
      this.ok = 0;
      this.failed = 0;
      this.lastFail = null;   // {t, reason, text}, newest ok=0 line
      this.lastReason = null; // {t, reason, text}, often the driver's W line
      this.lastOk = null;     // newest ok= value
      this.errors = 0;        // E lines
      this.warnings = 0;      // W lines
      this.lastT = null;
      this.lastText = '';
      this.recent = [];       // {t, text}, last RECENT lines
    }

    add(t, line, text) {
      if (line.event === 'init') this.init = line.fields;
      const e = this.events.get(line.event) || { count: 0, fields: {}, t: null, ms: null, gaps: [], n: [] };
      if (e.t !== null) {
        e.gaps.push(t - e.t);
        if (e.gaps.length > 20) e.gaps.shift();
      }
      e.count++;
      e.fields = line.fields;
      e.t = t;
      e.ms = line.ms;
      // rate from the board's own n and clock, a backed up console delays lines
      if (line.fields.n !== undefined && Number.isFinite(Number(line.fields.n))) {
        e.n.push([line.ms, Number(line.fields.n)]);
        while (e.n.length > 2 && line.ms - e.n[0][0] > 10000) e.n.shift();
      }
      this.events.set(line.event, e);
      if (line.fields.ok === '1') { this.ok++; this.lastOk = true; }
      if (line.fields.ok === '0') {
        this.failed++;
        this.lastOk = false;
        this.lastFail = { t, reason: line.fields.reason || line.fields.status || '', text };
      }
      if (line.fields.reason) this.lastReason = { t, reason: line.fields.reason, text };
      if (line.level === 'E') this.errors++;
      if (line.level === 'W') this.warnings++;
      this.lastT = t;
      this.lastText = text;
      this.recent.push({ t, text });
      if (this.recent.length > RECENT) this.recent.shift();
    }

    // the bench reading line if there is one, otherwise the most frequent event
    main() {
      const reading = this.events.get('bench');
      if (reading) return { name: 'bench', ...reading };
      let best = null;
      for (const [name, e] of this.events) if (!best || e.count > best[1].count) best = [name, e];
      return best ? { name: best[0], ...best[1] } : null;
    }

    // per second by the board's clock, over the last 10 s
    rate(event) {
      const e = this.events.get(event);
      if (!e || e.n.length < 2) return null;
      const [ms0, n0] = e.n[0];
      const [ms1, n1] = e.n[e.n.length - 1];
      return ms1 > ms0 ? (n1 - n0) * 1000 / (ms1 - ms0) : null;
    }

    // gone after 3 usual gaps (2 s at least), on this computer's clock
    quiet(now) {
      const m = this.main();
      if (!m || this.lastT === null) return { ms: null, usual: null, gone: false };
      const sorted = [...m.gaps].sort((a, b) => a - b);
      const usual = sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
      const ms = now - this.lastT;
      return { ms, usual, gone: usual !== null && ms > Math.max(3 * usual, 2000) };
    }
  }

  // Bench packets per receiver, two receivers hear the same counter twice
  class Radio {
    constructor() {
      this.by = new Map();    // receiver -> {heard, badCrc, missed, restarts, last, rssi, snr, t}
    }

    // m: a packet message off the stream, {csid, rx, t, crcOk, values, rssi, snr}
    add(m) {
      if (!m || m.kind !== 'packet' || !m.values) return false;
      if (Number(m.csid) !== BENCH_CSID) return false;
      const rx = String(m.rx);
      if (!this.by.has(rx)) this.by.set(rx, { heard: 0, badCrc: 0, missed: 0, restarts: 0, last: null, rssi: [], snr: [], t: null });
      const r = this.by.get(rx);
      r.heard++;
      r.t = m.t;
      if (m.crcOk === false) { r.badCrc++; return true; }
      const c = m.values.counter;
      if (r.last !== null) {
        // lower is a restart, unless the counter wrapped
        const wrapped = c < r.last && r.last > 65000 && c < 1000;
        if (c <= r.last && !wrapped) r.restarts++;
        else r.missed += ((c - r.last + 65536) % 65536) - 1;
      }
      r.last = c;
      for (const [k, v] of [['rssi', m.rssi], ['snr', m.snr]]) {
        if (!Number.isFinite(v)) continue;
        r[k].push(v);
        if (r[k].length > 200) r[k].shift();
      }
      return true;
    }
  }

  return { parse, Session, Part, Radio, BENCH_CSID, GAP_MS, LOOP_BOOTS, LOOP_MS };
});
