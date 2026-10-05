// A log as a table: one CSV row per packet that passes its CRC.
// Takes the replay events [{t, rx, text}], so it exports whatever is open.
// A packet two receivers heard is two rows; the source column says which.
(function (root, factory) {
  const P = typeof require === 'function' ? require('./packet.js') : root.DG.packet;
  const L = typeof require === 'function' ? require('./lines.js') : root.DG.lines;
  const mod = factory(P, L);
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).exportcsv = mod;
})(typeof self !== 'undefined' ? self : this, function (P, L) {
  'use strict';

  const LEAD = ['time_ms', 'time_utc', 'source', 'boot', 'uptime_ms', 'csid', 'counter', 'validity'];
  const LINK = ['rssi', 'snr', 'freq_err'];

  // Column names the flight-side records.csv (V2_6_X/tools/flash_replay.py) already uses
  // for the same quantity, so the two files line up. Any other field goes by its key.
  const NAMES = { gpsAlt: 'gps_alt_m', envAlt: 'baro_alt_m', temp: 'temp_c', pressure: 'pressure_hpa', battery: 'cell_pct' };

  // csid, counter and validity lead the row, so they are not repeated among the fields.
  const FIELDS = P.FIELDS.filter((f) => !LEAD.includes(f.key));
  const COLUMNS = LEAD.concat(FIELDS.map((f) => NAMES[f.key] || f.key), LINK);

  // Printed to the field's own resolution. Plain toFixed, trimmed, because String() of a
  // latitude under a millionth of a degree comes out as 1e-7.
  function fixed(v, decimals) {
    let s = v.toFixed(decimals);
    if (s.indexOf('.') >= 0) s = s.replace(/0+$/, '').replace(/\.$/, '');
    return s === '-0' ? '0' : s;
  }

  const plain = (v) => (Number.isFinite(v) ? String(v) : '');
  const quoted = (s) => (/[",\r\n]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s);

  function utc(t) {
    const d = new Date(t);
    return Number.isNaN(d.getTime()) ? '' : d.toISOString();
  }

  function toCsv(events) {
    const out = [COLUMNS.join(',')];
    const from = new Map();   // rx -> {src, boot}, as last said on that rx
    for (const e of events) {
      const ev = L.parseLine(e.text);
      if (ev.kind === 'source' || ev.kind === 'boot') {
        const f = from.get(e.rx) || {};
        if (ev.kind === 'source') f.src = ev.info; else f.boot = ev.info.n;
        from.set(e.rx, f);
        continue;
      }
      if (ev.kind !== 'packet' || !ev.decoded.crcOk) continue;
      const f = from.get(e.rx) || {};
      const src = f.src || {};
      const d = ev.decoded;
      // Same test as the dashboard: anything not anchored to GPS is a derived time, and
      // as an ISO string it would look exactly like a measured one.
      const derived = !!src.time_source && src.time_source !== 'anchor';
      // A row that says its own uptime is exact. Older logs do not, and for those it is
      // the time less the boot's start, which is only near it on a long boot.
      const uptime0 = src.uptime0_ms === undefined ? NaN : Number(src.uptime0_ms);
      const uptime = ev.more && ev.more.up !== undefined ? Number(ev.more.up) : e.t - uptime0;
      const row = [
        plain(e.t),
        derived ? '' : utc(e.t),
        quoted(String(e.rx)),
        f.boot === undefined ? '' : quoted(String(f.boot)),
        plain(uptime),
        plain(d.values.csid),
        plain(d.values.counter),
        d.values.validity.toString(16).toUpperCase().padStart(2, '0'),
      ];
      // A reading the board marked invalid is left out, not written as the number that
      // happened to be in the packet, so nobody plots it.
      for (const fd of FIELDS) row.push(!fd.group || d.valid[fd.group] ? fixed(d.values[fd.key], fd.csvDecimals) : '');
      row.push(plain(ev.rssi), plain(ev.snr), plain(ev.freqErr));
      out.push(row.join(','));
    }
    return out.join('\n') + '\n';
  }

  return { toCsv, COLUMNS };
});
