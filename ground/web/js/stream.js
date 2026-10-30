// The page's side of the app's stream (/api/stream live, /api/replay/{id}/stream for a
// replay). The app decodes every line and keeps the fleet; this keeps what the page draws:
// the unit rows as the app sent them, the receivers, the last lines of each receiver, and
// the graph history built from the packet messages. No DOM, so node can test it.
(function (root, factory) {
  const mod = factory();
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).stream = mod;
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  // Validity bits (bit 0 = LSB) and the packet's fields with the bit that says whether each
  // can be trusted. Display tables only: the bytes are decoded in the app (decode.rs), whose
  // names these are.
  const VALIDITY_BITS = [
    { bit: 0, key: 'accel', short: 'L', name: 'Linear acceleration' },
    { bit: 1, key: 'gyro', short: 'G', name: 'Gyroscope' },
    { bit: 2, key: 'mag', short: 'M', name: 'Magnetometer' },
    { bit: 3, key: 'quat', short: 'Q', name: 'Quaternion' },
    { bit: 4, key: 'gps', short: 'P', name: 'GPS' },
    { bit: 5, key: 'soc', short: 'S', name: 'State of charge' },
    { bit: 6, key: 'env', short: 'E', name: 'Environment' },
    { bit: 7, key: 'fresh', short: 'F', name: 'All data fresh' },
  ];
  const FIELDS = [
    ['lat', 'gps'], ['lon', 'gps'], ['gpsAlt', 'gps'], ['envAlt', 'env'],
    ['ax', 'accel'], ['ay', 'accel'], ['az', 'accel'], ['gx', 'gyro'], ['gy', 'gyro'], ['gz', 'gyro'],
    ['mx', 'mag'], ['my', 'mag'], ['mz', 'mag'], ['qi', 'quat'], ['qj', 'quat'], ['qk', 'quat'], ['qr', 'quat'],
    ['temp', 'env'], ['pressure', 'env'], ['humidity', 'env'],
    ['counter', null], ['csid', null], ['battery', 'soc'], ['validity', null],
  ].map(([key, group]) => ({ key, group }));
  const SERIES = ['counter', 'rssi', 'snr'].concat(FIELDS.filter((f) => f.group).map((f) => f.key));

  // What the app starts with; the drawer shows the app's own once it has answered.
  const MODEL_DEFAULTS = {
    lostAfterMs: 600000, staleIntervals: 3, staleMinMs: 5000, lowBatteryV: 3.5,
    weakRssiDbm: -115, accelSatMs2: 75, dedupeWindowMs: 1500,
  };
  const STALE_DEFAULT_MS = 30000;   // fleet.js's, before the interval is known
  const SLACK = 2000;               // points kept past the cap before one tidy-up
  const LINES_KEPT = 200;           // per receiver, for the raw boxes

  // JSON has no NaN, so the app's NaN arrives as null
  const nan = (v) => (v === null || v === undefined ? NaN : Number(v));

  function newHistory() {
    const h = { t: [] };
    for (const k of SERIES) h[k] = [];
    return h;
  }

  // A run is one boot of a board as the app split it. Its label moves when it is retired
  // (64 becomes 64a) but its first packet does not, so that is what history is kept by.
  const runIdOf = (row) => (row.firstT === null || row.firstT === undefined ? 'label:' + row.label : row.csid + '@' + row.firstT);

  function keyValues(parts) {
    const info = {};
    for (const p of parts) {
      const eq = p.indexOf('=');
      if (eq > 0) info[p.slice(0, eq)] = p.slice(eq + 1);
    }
    return info;
  }

  // A unit message as the fleet row app.js and livecards.js read: fleet.js's unit fields,
  // plus state, issues and staleAfterMs, which the app works out from its model settings.
  function unitFrom(m) {
    const u = Object.assign({}, m);
    delete u.kind; delete u.seq;
    u.csid = Number(m.csid);
    u.label = String(m.label === undefined ? m.csid : m.label);
    u.live = m.live === undefined ? u.label === String(u.csid) : !!m.live;
    u.firstT = m.firstT === undefined ? null : m.firstT;
    u.lastT = m.lastT === undefined ? null : m.lastT;
    for (const k of ['intervalMs', 'bestRssi', 'bestSnr']) u[k] = nan(m[k]);
    for (const k of ['packets', 'missed', 'resets', 'repeats', 'saturatedCount']) u[k] = Number(m[k]) || 0;
    u.boot = m.boot === undefined ? null : m.boot;
    u.receptions = Array.isArray(m.receptions) ? m.receptions : [];
    u.lastValid = m.lastValid || {};
    u.issues = Array.isArray(m.issues) ? m.issues : [];
    u.state = m.state || 'OK';
    // A row that carries the packet flat rather than under latest
    if (!u.latest && m.values) u.latest = { crcOk: m.crcOk !== false, values: m.values, valid: m.valid || {}, saturated: !!m.saturated };
    return u;
  }

  // A receiver message as the fleet.js receiver app.js, livecards.js and boardmodel.js read.
  // packets is every packet line it gave, as in fleet.js; unique is what was new.
  function receiverFrom(m, was) {
    const packets = Number(m.packets) || 0;
    const dups = Number(m.dups) || 0;
    const badCrc = Number(m.badCrc) || 0;
    const lastT = typeof m.lastT === 'number' ? m.lastT : typeof m.lastLine === 'number' ? m.lastLine : (was ? was.lastT : null);
    return {
      key: String(m.id), id: m.rxId || m.boardId || null, name: m.name || null, state: m.state || null,
      info: Object.assign({}, m.info, { sf: m.sf, name: m.name || undefined }),
      firstT: m.firstT === undefined ? (was ? was.firstT : null) : m.firstT, lastT, lastLine: m.lastLine,
      packets, dups, badCrc, unique: m.unique !== undefined ? Number(m.unique) : packets - dups - badCrc,
      radioErrors: Number(m.errors) || 0,
    };
  }

  // The fleet as this page sees it. The same read calls fleet.js had, over the app's rows.
  // Hidden units, joined and deleted runs and Clear are this page's own: the contract has no
  // call for them yet, so they change what this page shows, never what the app keeps.
  class StreamFleet {
    constructor(cfg) {
      this.cfg = {};
      this.setConfig(cfg || {});
      this.hidden = new Set();      // csid
      this.gone = new Set();        // run ids joined or deleted here; the app still sends them
      this.overlay = new Map();     // run id -> counts joined into it here
      this.rxBase = new Map();      // rx -> counts at the last Clear
      this.clear();
    }

    // cfg: {model: {...MODEL_DEFAULTS}, maxPoints, retainMin}. The old names stay on it for
    // livecards.js and charts.js.
    setConfig(cfg) {
      const model = Object.assign({}, MODEL_DEFAULTS, this.cfg.model, cfg.model);
      this.cfg = Object.assign({ maxPoints: 20000, retainMin: 0 }, this.cfg, cfg, {
        model,
        lowBatteryV: model.lowBatteryV, weakRssiDbm: model.weakRssiDbm, saturationMps2: model.accelSatMs2,
        lostMin: model.lostAfterMs / 60000, staleFactor: model.staleIntervals, staleMinS: model.staleMinMs / 1000,
        dedupeMs: model.dedupeWindowMs,
      });
      if (this.runs) for (const r of this.runs.values()) this.trim(r, true);
    }

    clear() {
      this.appRows = new Map();     // label -> the row as the app last sent it
      this.units = new Map();       // csid -> the run transmitting now, as shown
      this.retired = new Map();     // label -> an earlier run, as shown
      this.runs = new Map();        // run id -> {history, rxHistory, ...}
      this.labelRun = new Map();    // label -> run id its packets go to
      this.receivers = new Map();   // rx -> receiver
      this.rawRx = new Map();       // rx -> the receiver as the app sent it
      this.sources = new Map();     // rx -> #DG,SRC fields, when its lines came off a chip
      this.rxSource = new Map();    // rx -> 'raw' or 'csv', the receiver firmware's format
      this.lines = new Map();       // rx -> [{t, text}], newest last
      this.packetsSeen = 0;
    }

    // An empty fleet that keeps this page's own choices, for a snapshot to fill
    fresh() {
      const f = new StreamFleet(this.cfg);
      f.hidden = this.hidden; f.gone = this.gone; f.overlay = this.overlay; f.rxBase = this.rxBase;
      return f;
    }

    // Take a finished snapshot's state, keeping this object, which the screens hold on to.
    // keepHistory carries graphs over for runs the snapshot sent no packets for: a live
    // reconnect, where the app's rows are the same runs.
    adopt(next, keepHistory) {
      if (keepHistory) {
        for (const [id, r] of this.runs) {
          const n = next.runs.get(id);
          const empty = !n || (!n.history.t.length && !Object.keys(n.rxHistory).length);
          // asked for again after a reconnect: the app may have more than this page saw
          if (empty && !id.startsWith('label:')) { r.fetched = false; next.runs.set(id, r); }
        }
        for (const [rx, l] of this.lines) if (!next.lines.has(rx)) next.lines.set(rx, l);
        for (const [rx, s] of this.sources) if (!next.sources.has(rx)) next.sources.set(rx, s);
        for (const [rx, s] of this.rxSource) if (!next.rxSource.has(rx)) next.rxSource.set(rx, s);
      }
      for (const k of ['cfg', 'appRows', 'units', 'retired', 'runs', 'labelRun', 'receivers', 'rawRx', 'sources', 'rxSource', 'lines', 'packetsSeen']) this[k] = next[k];
      for (const label of this.appRows.keys()) this.show(label);
    }

    // One message off the stream. Returns its kind, or null for one that is not ours.
    apply(m) {
      switch (m.kind) {
        case 'packet': this.packet(m); return 'packet';
        case 'unit': this.unitRow(m); return 'unit';
        case 'receiver': this.receiverRow(m); return 'receiver';
        case 'line': this.line(m); return 'line';
        case 'settings': {
          const model = {};
          for (const k of Object.keys(MODEL_DEFAULTS)) if (typeof m[k] === 'number') model[k] = m[k];
          this.setConfig({ model });
          return 'settings';
        }
        default: return null;
      }
    }

    run(id) {
      let r = this.runs.get(id);
      if (!r) {
        r = { history: newHistory(), rxHistory: {}, lastHex: null, lastHexT: null, pointT: null, bestRssi: NaN, bestSnr: NaN };
        this.runs.set(id, r);
      }
      return r;
    }

    // Graph points for one reception. A packet two receivers heard is one point and two
    // receptions; the app's dup flag says which, and without it the same bytes again inside
    // the dedupe window are taken as the same packet, as fleet.js did.
    packet(m) {
      this.packetsSeen++;
      if (m.crcOk === false || !m.values) return;
      // A packet carries its csid only. It belongs to the run transmitting now, which is the
      // one the csid's own label (9, never 9a) names in the unit rows.
      const label = String(m.csid);
      let id = this.labelRun.get(label);
      if (!id) { id = 'label:' + label; this.labelRun.set(label, id); }
      if (this.gone.has(id)) return;
      const r = this.run(id);
      const t = Number(m.t);
      const dup = m.dup !== undefined ? !!m.dup
        : m.new !== undefined ? !m.new
          : !!m.hex && m.hex === r.lastHex && t - r.lastHexT <= this.cfg.model.dedupeWindowMs;
      const h = r.history;
      if (!dup) {
        r.lastHex = m.hex || null; r.lastHexT = t;
        r.pointT = Math.round(t) / 1000;
        r.bestRssi = NaN; r.bestSnr = NaN;
        h.t.push(r.pointT);
        h.counter.push(m.values.counter);
        h.rssi.push(null); h.snr.push(null);
        const valid = m.valid || {};
        for (const f of FIELDS) if (f.group) h[f.key].push(valid[f.group] ? m.values[f.key] : null);
      }
      const rssi = nan(m.rssi); const snr = nan(m.snr);
      const rx = String(m.rx);
      const rh = r.rxHistory[rx] || (r.rxHistory[rx] = { t: [], rssi: [], snr: [] });
      rh.t.push(t / 1000); rh.rssi.push(rssi); rh.snr.push(snr);
      if (Number.isFinite(rssi) && !(rssi <= r.bestRssi)) r.bestRssi = rssi;
      if (Number.isFinite(snr) && !(snr <= r.bestSnr)) r.bestSnr = snr;
      const i = h.t.length - 1;
      if (i >= 0 && h.t[i] === r.pointT) {
        h.rssi[i] = Number.isFinite(r.bestRssi) ? r.bestRssi : null;
        h.snr[i] = Number.isFinite(r.bestSnr) ? r.bestSnr : null;
      }
      if (!dup) this.trim(r);
    }

    unitRow(m) {
      const row = unitFrom(m);
      const id = runIdOf(row);
      if (this.gone.has(id)) return;
      const was = this.labelRun.get(row.label);
      if (was && was !== id) {
        if (was.startsWith('label:')) {
          // packets that came before their row
          if (!this.runs.has(id) && this.runs.has(was)) this.runs.set(id, this.runs.get(was));
          this.runs.delete(was);
        } else if (this.runs.has(was) && row.firstT !== null) {
          // The label has a new run: the board restarted. A packet of the new run can have
          // arrived before this row, and went on the old run's graph.
          this.split(was, id, row.firstT / 1000);
        }
      }
      this.labelRun.set(row.label, id);
      // A retired run is no longer at its csid; drop the live entry if it was this run
      const live = this.appRows.get(String(row.csid));
      if (!row.live && live && runIdOf(live) === id) this.appRows.delete(String(row.csid));
      this.appRows.set(row.label, row);
      this.show(row.label);
      if (!row.live) {
        const cur = this.units.get(row.csid);
        if (cur && runIdOf(cur) === id) this.units.delete(row.csid);
      }
    }

    // Points from `from` on move from one run's graph to another's
    split(fromId, toId, from) {
      const a = this.runs.get(fromId);
      const b = this.run(toId);
      const cut = (h, into) => {
        let n = h.t.length;
        while (n > 0 && h.t[n - 1] >= from) n--;
        if (n === h.t.length) return;
        for (const k of Object.keys(h)) {
          const moved = h[k].splice(n);
          into[k] = moved.concat(into[k] || []);
        }
      };
      cut(a.history, b.history);
      for (const [rx, h] of Object.entries(a.rxHistory)) cut(h, b.rxHistory[rx] || (b.rxHistory[rx] = { t: [], rssi: [], snr: [] }));
      if (a.pointT !== null && a.pointT >= from) {
        Object.assign(b, { lastHex: a.lastHex, lastHexT: a.lastHexT, pointT: a.pointT, bestRssi: a.bestRssi, bestSnr: a.bestSnr });
        a.lastHex = null; a.pointT = null;
      }
    }

    // The series the app holds for a run, for a page that opened after it started. Points
    // that came on the stream while it was being asked for are kept on the end.
    setHistory(label, series) {
      const row = this.appRows.get(String(label));
      if (!row || !series || !Array.isArray(series.t)) return false;
      const r = this.run(runIdOf(row));
      const h = newHistory();
      const n = series.t.length;
      for (const k of Object.keys(h)) {
        const col = Array.isArray(series[k]) ? series[k] : [];
        h[k] = col.length === n ? col.slice() : new Array(n).fill(null);
      }
      const last = n ? h.t[n - 1] : -Infinity;
      const old = r.history;
      for (let i = 0; i < old.t.length; i++) {
        if (old.t[i] <= last) continue;
        for (const k of Object.keys(h)) h[k].push(old[k][i]);
      }
      r.history = h;
      r.fetched = true;
      this.trim(r);
      this.show(row.label);
      return true;
    }

    // The row as shown: the app's, with any runs joined into it on this page, and the
    // graph history.
    show(label) {
      const row = this.appRows.get(label);
      if (!row) return null;
      const id = runIdOf(row);
      const u = Object.assign({}, row);
      const o = this.overlay.get(id);
      if (o) {
        for (const k of ['packets', 'missed', 'repeats', 'resets', 'saturatedCount']) u[k] += o[k];
        if (o.firstT !== null && (u.firstT === null || o.firstT < u.firstT)) u.firstT = o.firstT;
        u.lastValid = Object.assign({}, o.lastValid, u.lastValid);
      }
      const r = this.run(id);
      u.history = r.history;
      u.rxHistory = r.rxHistory;
      u.runId = id;
      if (u.live) this.units.set(u.csid, u); else { this.retired.set(u.label, u); }
      return u;
    }

    receiverRow(m) {
      const key = String(m.id);
      const raw = receiverFrom(m, this.rawRx.get(key));
      this.rawRx.set(key, raw);
      this.showRx(key);
    }

    showRx(key) {
      const raw = this.rawRx.get(key);
      if (!raw) return;
      const r = Object.assign({}, raw, { info: Object.assign({}, raw.info) });
      const base = this.rxBase.get(key);
      if (base) {
        for (const k of ['packets', 'dups', 'badCrc', 'unique', 'radioErrors']) r[k] = Math.max(0, r[k] - base[k]);
        r.firstT = base.firstT;
      }
      this.receivers.set(key, r);
    }

    line(m) {
      const rx = String(m.rx);
      const text = String(m.text === undefined ? '' : m.text);
      const t = Number(m.t);
      let l = this.lines.get(rx);
      if (!l) { l = []; this.lines.set(rx, l); }
      l.push({ t, text });
      if (l.length > LINES_KEPT) l.splice(0, l.length - LINES_KEPT);
      const raw = this.rawRx.get(rx);
      if (raw && Number.isFinite(t) && !(raw.lastT >= t)) { raw.lastT = t; this.showRx(rx); }
      // Which firmware a receiver runs and whether its lines came off a chip are said in its
      // own lines; the receiver message does not carry them.
      if (text.startsWith('#DG,SRC,')) this.sources.set(rx, keyValues(text.split(',').slice(3)));
      else if (/^(PKT|HB|ERR),|^#DG,RX,/.test(text)) this.rxSource.set(rx, 'raw');
      else if (text.split(',').length === 35 && !text.startsWith('Latitude_deg,')) this.rxSource.set(rx, 'csv');
    }

    trim(r, exact) {
      const cut = (h) => {
        let n = 0;
        if (this.cfg.retainMin > 0 && h.t.length) {
          const oldest = h.t[h.t.length - 1] - this.cfg.retainMin * 60;
          while (n < h.t.length && h.t[n] < oldest) n++;
        }
        const over = h.t.length - n - this.cfg.maxPoints;
        if (over > 0 && (exact || over >= SLACK)) n += over;
        if (n > 0) for (const k in h) h[k].splice(0, n);
      };
      cut(r.history);
      for (const k in r.rxHistory) cut(r.rxHistory[k]);
    }

    // ---------- fleet.js's read calls ----------
    get total() {
      let n = 0;
      for (const u of this.units.values()) n += u.packets;
      for (const u of this.retired.values()) n += u.packets;
      return n;
    }

    allUnits() {
      const out = new Map();
      for (const [csid, u] of this.units) out.set(String(csid), u);
      for (const [label, u] of this.retired) out.set(label, u);
      return out;
    }

    unit(label) {
      if (label === null || label === undefined) return null;
      return this.units.get(Number(label)) || this.retired.get(String(label)) || null;
    }

    earlierRuns(csid) {
      return [...this.retired.values()].filter((u) => u.csid === Number(csid))
        .sort((a, b) => (a.gen - b.gen) || (a.firstT - b.firstT));
    }

    state(u) { return u.state || 'OK'; }
    issues(u) { return u.issues || []; }

    // The app's own limit when it sends one. Otherwise fleet.js's rule on the app's settings,
    // only to size the bar on the card.
    staleAfterMs(u) {
      if (Number.isFinite(u.staleAfterMs)) return u.staleAfterMs;
      const m = this.cfg.model;
      return Number.isFinite(u.intervalMs) ? Math.max(m.staleMinMs, m.staleIntervals * u.intervalMs) : STALE_DEFAULT_MS;
    }

    rxPercent(u) {
      if (Number.isFinite(u.rxPercent)) return u.rxPercent;
      const expected = u.packets + u.missed;
      return expected ? (100 * u.packets) / expected : NaN;
    }

    rows(now, sort) {
      const rank = { LOST: 0, STALE: 1, OK: 2 };
      const rows = [];
      for (const u of this.units.values()) {
        if (this.hidden.has(u.csid)) continue;
        rows.push({ unit: u, state: this.state(u), age: now - u.lastT, issues: this.issues(u) });
      }
      rows.sort((a, b) => {
        if (sort === 'attention') {
          const r = (rank[a.state] === undefined ? 2 : rank[a.state]) - (rank[b.state] === undefined ? 2 : rank[b.state]);
          if (r) return r;
          const i = b.issues.length - a.issues.length;
          if (i) return i;
        }
        return a.unit.csid - b.unit.csid;
      });
      return rows;
    }

    // Counted off the rows the app sent: their states and issue words, never re-judged here
    summary() {
      const s = { heard: 0, active: 0, stale: 0, lost: 0, packets: this.total, lowBattery: 0,
        validityIssues: 0, weakRf: 0, resets: 0, badCrc: 0, saturated: 0 };
      for (const r of this.receivers.values()) s.badCrc += r.badCrc;
      for (const u of this.units.values()) {
        if (this.hidden.has(u.csid)) continue;
        s.heard++;
        const st = this.state(u);
        if (st === 'OK') s.active++; else if (st === 'STALE') s.stale++; else s.lost++;
        const issues = this.issues(u);
        if (issues.includes('low battery')) s.lowBattery++;
        if (issues.includes('weak RF')) s.weakRf++;
        const d = u.latest;
        if (d && d.values && (d.values.validity & 0x7f) !== 0x7f) s.validityIssues++;
        if (d && d.saturated) s.saturated++;
        s.resets += u.resets;
      }
      return s;
    }

    // ---------- this page's own changes ----------
    // Join an earlier run into the one after it, on this page.
    mergeRun(label) {
      const a = this.retired.get(String(label));
      if (!a) return null;
      const later = [...this.retired.values()]
        .filter((u) => u.csid === a.csid && u.gen > a.gen)
        .sort((x, y) => x.gen - y.gen)[0] || this.units.get(a.csid);
      if (!later) return null;
      const into = later.runId;
      const o = this.overlay.get(into) || { packets: 0, missed: 0, repeats: 0, resets: 0, saturatedCount: 0, firstT: null, lastValid: {} };
      o.packets += a.packets; o.missed += a.missed; o.repeats += a.repeats;
      // the board did restart, whatever the operator calls it
      o.resets += a.resets + 1;
      o.saturatedCount += a.saturatedCount;
      if (a.firstT !== null && (o.firstT === null || a.firstT < o.firstT)) o.firstT = a.firstT;
      o.lastValid = Object.assign({}, a.lastValid, o.lastValid);
      this.overlay.set(into, o);
      const from = this.runs.get(a.runId);
      const to = this.run(into);
      if (from) {
        for (const k of Object.keys(to.history)) to.history[k] = (from.history[k] || []).concat(to.history[k]);
        for (const [rx, h] of Object.entries(from.rxHistory)) {
          const t = to.rxHistory[rx] || (to.rxHistory[rx] = { t: [], rssi: [], snr: [] });
          for (const k of ['t', 'rssi', 'snr']) t[k] = h[k].concat(t[k]);
        }
      }
      this.drop(a);
      this.show(later.label);
      return later.label;
    }

    mergeAllRuns(csid) {
      let n = 0;
      for (const u of this.earlierRuns(csid).reverse()) if (this.mergeRun(u.label)) n++;
      return n;
    }

    // An earlier run is never sent again except in a snapshot, so it stays gone. The run
    // transmitting now comes back with its next row, as fleet.js started it again.
    drop(u) {
      if (!u.live) this.gone.add(u.runId);
      this.appRows.delete(u.label);
      this.runs.delete(u.runId);
      if (u.live) this.units.delete(u.csid); else this.retired.delete(u.label);
    }

    clearUnit(label) {
      const u = this.unit(label);
      if (!u) return false;
      this.drop(u);
      this.hidden.delete(u.csid);
      return true;
    }

    clearUnits() {
      for (const u of [...this.units.values(), ...this.retired.values()]) this.drop(u);
      this.hidden.clear();
    }

    clearReceivers() {
      for (const [key, raw] of this.rawRx) {
        this.rxBase.set(key, { packets: raw.packets, dups: raw.dups, badCrc: raw.badCrc, unique: raw.unique, radioErrors: raw.radioErrors, firstT: null });
        this.showRx(key);
      }
    }
  }

  // One stream: the socket, the snapshot, seq gaps and reconnects. The socket is opened with
  // o.WebSocket so node can hand it a fake.
  // o: {WebSocket, setTimeout, clearTimeout, cfg, keepHistory, log,
  //     onLine(m), onPacket(m), onSettings(m), onReady(), onChange(kind), onPorts(), onFlash(m)}
  class StreamClient {
    constructor(url, o) {
      this.url = url;
      this.o = Object.assign({ keepHistory: true, log: () => {} }, o);
      this.fleet = new StreamFleet(this.o.cfg);
      this.next = null;          // the snapshot being received
      this.ready = false;        // a snapshot has been taken
      this.lastSeq = null;
      this.gaps = 0;             // messages the seq says never arrived
      this.connects = 0;
      this.delay = StreamClient.RETRY_MS;
      this.stopped = true;
      this.asked = new Map();    // history asks in flight
    }

    start() {
      this.stopped = false;
      this.connect();
      return this;
    }

    stop() {
      this.stopped = true;
      clearTimeout(this.retry);
      if (this.ws) { this.ws.onclose = null; try { this.ws.close(); } catch (e) { /* never opened */ } }
      this.ws = null;
    }

    // Close and open again, for a new snapshot
    reconnect() {
      this.stop();
      this.start();
    }

    connect() {
      if (this.stopped) return;
      const WS = this.o.WebSocket || (typeof WebSocket === 'function' ? WebSocket : null);
      if (!WS) return;
      let ws;
      try { ws = new WS(this.url); } catch (e) { this.later(); return; }
      this.ws = ws;
      ws.onopen = () => this.opened();
      ws.onmessage = (e) => this.handle(e.data);
      // The app outlives the tab and can be restarted under it, so keep trying
      ws.onclose = () => { if (this.ws === ws) { this.ws = null; this.later(); } };
    }

    later() {
      if (this.stopped) return;
      const set = this.o.setTimeout || setTimeout;
      const wait = this.delay;
      this.delay = Math.min(StreamClient.RETRY_MAX_MS, this.delay * 2);
      this.retry = set(() => this.connect(), wait);
    }

    // A connection has opened: everything until snapshot-end is the snapshot
    opened() {
      this.connects++;
      this.lastSeq = null;
      this.next = this.fleet.fresh();
    }

    handle(data) {
      let m;
      try { m = typeof data === 'string' ? JSON.parse(data) : data; } catch (e) { return; }
      if (!m || typeof m !== 'object') return;
      // The app's port list and flash progress, which are not part of the model
      if (m.type === 'ports' || m.kind === 'ports') { if (this.o.onPorts) this.o.onPorts(m); return; }
      if (m.type === 'flash' || m.kind === 'flash') { if (this.o.onFlash) this.o.onFlash(m); return; }
      if (typeof m.seq === 'number') {
        if (this.lastSeq !== null && m.seq > this.lastSeq + 1) {
          const lost = m.seq - this.lastSeq - 1;
          this.gaps += lost;
          this.o.log('stream: ' + lost + ' message(s) missing before seq ' + m.seq + ', ' + this.gaps + ' so far');
        }
        if (this.lastSeq === null || m.seq > this.lastSeq) this.lastSeq = m.seq;
      }
      if (m.kind === 'reset') { this.next = this.fleet.fresh(); return; }
      if (m.kind === 'snapshot-end') {
        if (this.next) this.fleet.adopt(this.next, this.o.keepHistory && this.ready);
        this.next = null;
        this.ready = true;
        this.delay = StreamClient.RETRY_MS;
        if (this.o.onReady) this.o.onReady();
        if (this.o.onChange) this.o.onChange('snapshot');
        return;
      }
      if (this.next) {
        // Lines and packets in a snapshot are history, not news: the taps do not hear them
        if (this.next.apply(m) === 'settings' && this.o.onSettings) this.o.onSettings(m);
        return;
      }
      const kind = this.fleet.apply(m);
      if (!kind) return;
      if (kind === 'line' && this.o.onLine) this.o.onLine(m);
      if (kind === 'packet' && this.o.onPacket) this.o.onPacket(m);
      if (kind === 'settings' && this.o.onSettings) this.o.onSettings(m);
      if (this.o.onChange) this.o.onChange(kind);
    }
  }
  // Asks the app for a unit's graph once per run per snapshot: on selection, and again for
  // the selected one after a reconnect. o.historyUrl(label) says where; o.fetch does it.
  StreamClient.prototype.wantHistory = function (label) {
    if (label === null || label === undefined || !this.ready || !this.o.historyUrl) return null;
    const row = this.fleet.appRows.get(String(label));
    if (!row) return null;
    const r = this.fleet.runs.get(runIdOf(row));
    if (r && r.fetched) return null;
    const key = this.connects + ':' + runIdOf(row);
    if (this.asked.has(key)) return this.asked.get(key);
    const at = this.connects;
    const go = (this.o.fetch || fetch)(this.o.historyUrl(String(label)))
      .then((res) => (res.ok ? res.json() : null))
      .then((series) => {
        // a reconnect since: that snapshot's own ask is the one that counts
        if (!series || at !== this.connects) return false;
        const ok = this.fleet.setHistory(label, series);
        if (ok && this.o.onChange) this.o.onChange('history');
        return ok;
      })
      .catch(() => false)
      .finally(() => this.asked.delete(key));
    this.asked.set(key, go);
    return go;
  };
  StreamClient.RETRY_MS = 500;
  StreamClient.RETRY_MAX_MS = 8000;

  // ws:// or wss:// for a path on the server the page came from
  function wsUrl(loc, path) {
    return (loc.protocol === 'https:' ? 'wss://' : 'ws://') + loc.host + path;
  }

  return { VALIDITY_BITS, FIELDS, SERIES, MODEL_DEFAULTS, StreamFleet, StreamClient, unitFrom, receiverFrom, wsUrl, LINES_KEPT };
});
