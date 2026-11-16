// What the flight view draws, worked out from what a board sent or logged: one sample per packet
// or flash record, the gaps between them, and the few values derived from them, each with its
// rule. Nothing here smooths, fills in or fuses: a sample is what the board said, with its own
// flags, and the barometer and the GPS are never mixed. No DOM, so node can test it.
(function (root, factory) {
  const mod = factory();
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).flight = mod;
})(typeof self !== 'undefined' ? self : this, function () {
  'use strict';

  // fsw/tools/find_landing.py's rule, so the page and the script agree on a touchdown
  const STILL_ACCEL_MPS2 = 0.3;
  const STILL_GYRO_DPS = 2;
  const STILL_FOR_S = 20;
  const RELEASE_LOOKBACK_S = 30;
  const FIX_WINDOW_S = 60;
  const VSPEED_WINDOW_S = 1;
  const COPY_WITHIN_S = 60;
  const BIT = { accel: 1, gyro: 2, quat: 8, gps: 16, env: 64 };

  const hyp = (a) => Math.hypot(a[0], a[1], a[2]);

  // A radio run's graph history: one point per packet at its first receipt, a field null where
  // its flag was clear (stream.js). A counter seen again within a minute is a copy heard after the
  // dedupe window; later, it is the counter starting again after a restart.
  function fromHistory(h) {
    const out = [];
    const seen = new Map();   // counter -> when it was first heard
    for (let i = 0; i < h.t.length; i++) {
      const c = h.counter[i];
      if (c !== null && c !== undefined) {
        if (seen.has(c) && h.t[i] - seen.get(c) < COPY_WITHIN_S) continue;
        seen.set(c, h.t[i]);
      }
      const v = (k) => (h[k] && h[k][i] !== undefined ? h[k][i] : null);
      const axes = (a, b, d) => (v(a) === null ? null : [v(a), v(b), v(d)]);
      const s = {
        t: h.t[i], counter: c === undefined ? null : c, lat: v('lat'), lon: v('lon'), gpsAlt: v('gpsAlt'), baroAlt: v('envAlt'),
        q: v('qr') === null ? null : [v('qr'), v('qi'), v('qj'), v('qk')], gyro: axes('gx', 'gy', 'gz'), accel: axes('ax', 'ay', 'az'),
        held: false,
      };
      s.ok = { gps: s.lat !== null && s.lon !== null, env: s.baroAlt !== null, quat: s.q !== null, gyro: s.gyro !== null, accel: s.accel !== null };
      out.push(s);
    }
    return out;
  }

  // A boot's track from /api/check (check.rs): uptime ms, the validity byte, GPS only where its
  // flag is set and everything else as logged. held: [fromMs, toMs] spells on a held fix.
  function fromTrack(tr, held) {
    const out = [];
    for (let i = 0; i < tr.ms.length; i++) {
      const ms = tr.ms[i];
      const bits = tr.valid[i];
      const axes = (a, b, d) => (tr[a][i] === null ? null : [tr[a][i], tr[b][i], tr[d][i]]);
      out.push({
        t: ms / 1000, ms, counter: null, lat: tr.lat[i], lon: tr.lon[i], gpsAlt: tr.gpsAlt[i], baroAlt: tr.envAlt[i],
        q: tr.qr[i] === null ? null : [tr.qr[i], tr.qi[i], tr.qj[i], tr.qk[i]],
        gyro: axes('gx', 'gy', 'gz'), accel: axes('ax', 'ay', 'az'),
        ok: { gps: !!(bits & BIT.gps), env: !!(bits & BIT.env), quat: !!(bits & BIT.quat), gyro: !!(bits & BIT.gyro), accel: !!(bits & BIT.accel) },
        held: (held || []).some(([a, b]) => ms >= a && ms <= b),
      });
    }
    return out;
  }

  // Packets the counter says never arrived, as {i, missed}: a break before sample i. The counter
  // is 16 bits and wraps.
  function counterGaps(samples) {
    const out = [];
    for (let i = 1; i < samples.length; i++) {
      const a = samples[i - 1].counter;
      const b = samples[i].counter;
      if (a === null || b === null) continue;
      const missed = (b - a - 1 + 65536) % 65536;
      if (missed > 0) out.push({ i, missed, words: missed + ' missed' });
    }
    return out;
  }

  // check.rs's gaps between flash records, as breaks before the record after each
  function trackGaps(samples, gaps) {
    return gaps.map((g) => {
      const i = samples.findIndex((s) => s.ms > g.afterMs);
      return { i, words: 'gap ' + (g.ms / 1000).toFixed(1) + ' s' };
    }).filter((g) => g.i > 0);
  }

  // A fix to draw: the GPS flag set. Exactly 0, 0 with the flag set is left out and counted by the
  // caller, as find_landing.py does, so one zeroed fix does not stretch the map across an ocean.
  const isFix = (s) => s.ok.gps && s.lat !== null && s.lon !== null && !(s.lat === 0 && s.lon === 0);
  const zeroFixes = (samples) => samples.filter((s) => s.ok.gps && s.lat === 0 && s.lon === 0).length;

  // Height above the boot's first reading of that source with its flag set, never above the
  // terrain: take-off is the boot's start, since a board is switched on at the launch point.
  function heights(samples, source) {
    const key = source === 'gps' ? 'gpsAlt' : 'baroAlt';
    const flag = source === 'gps' ? 'gps' : 'env';
    const first = samples.find((s) => s.ok[flag] && s[key] !== null);
    if (!first) return { source, base: null, baseT: null, of: () => null };
    return {
      source, base: first[key], baseT: first.t,
      of: (s) => (s[key] === null ? null : s[key] - first[key]),
      ok: (s) => s.ok[flag],
    };
  }

  function still(s) {
    return s.ok.accel && s.ok.gyro && s.accel && s.gyro && hyp(s.accel) < STILL_ACCEL_MPS2 && hyp(s.gyro) < STILL_GYRO_DPS;
  }

  // The start of the last spell lying still for 20 s, after some movement: a board that never
  // moved has not landed. find_landing.py's rule, which also asks for nothing before it.
  function touchdown(samples) {
    const imu = samples.filter((s) => s.ok.accel && s.ok.gyro && s.accel && s.gyro);
    let start = null;
    let found = null;
    let moved = false;
    for (const s of imu) {
      if (still(s)) {
        if (start === null) start = s;
      } else {
        start = null;
        moved = true;
      }
      if (start !== null && moved && s.t - start.t >= STILL_FOR_S) found = start;
    }
    return found;
  }

  // The first reading in the 30 s before touchdown turning at half the biggest rotation or more
  function release(samples, td) {
    const before = samples.filter((s) => s.t >= td.t - RELEASE_LOOKBACK_S && s.t < td.t && s.ok.gyro && s.gyro);
    if (!before.length) return null;
    const peak = Math.max(...before.map((s) => hyp(s.gyro)));
    return before.find((s) => hyp(s.gyro) >= peak / 2) || null;
  }

  function metres(lat0, lon0, lat, lon) {
    return Math.hypot((lat - lat0) * 111320, (lon - lon0) * 111320 * Math.cos(lat0 * Math.PI / 180));
  }

  function bearing(lat0, lon0, lat, lon) {
    const north = (lat - lat0) * 111320;
    const east = (lon - lon0) * 111320 * Math.cos(lat0 * Math.PI / 180);
    return ((Math.atan2(east, north) * 180 / Math.PI) + 360) % 360;
  }

  const POINTS = ['N', 'NNE', 'NE', 'ENE', 'E', 'ESE', 'SE', 'SSE', 'S', 'SSW', 'SW', 'WSW', 'W', 'WNW', 'NW', 'NNW'];
  const compass = (deg) => POINTS[Math.round(deg / 22.5) % 16];

  function median(a) {
    const s = a.slice().sort((x, y) => x - y);
    const m = s.length >> 1;
    return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
  }

  // Where to walk to: the median of the fixes in [from, to] and how far they spread, the middle
  // one (half within) and the farthest, find_landing.py's numbers.
  function spot(samples, from, to) {
    const fixes = samples.filter((s) => isFix(s) && s.t >= from && s.t <= to);
    if (!fixes.length) return null;
    const lat = median(fixes.map((s) => s.lat));
    const lon = median(fixes.map((s) => s.lon));
    const spread = fixes.map((s) => metres(lat, lon, s.lat, s.lon)).sort((a, b) => a - b);
    return { lat, lon, n: fixes.length, half: spread[spread.length >> 1], all: spread[spread.length - 1], from, to };
  }

  // The fixes to find a board by: the minute after touchdown, or with no touchdown the minute
  // up to its last fix, and which of the two it is.
  function findSpot(samples, td) {
    if (td) {
      const s = spot(samples, td.t, td.t + FIX_WINDOW_S);
      if (s) return Object.assign(s, { rule: 'median of the ' + s.n + ' fixes in the first ' + FIX_WINDOW_S + ' s after touchdown' });
    }
    const fixes = samples.filter(isFix);
    if (!fixes.length) return null;
    const last = fixes[fixes.length - 1].t;
    const s = spot(samples, last - FIX_WINDOW_S, last);
    return Object.assign(s, { rule: 'median of the ' + s.n + ' fixes in the ' + FIX_WINDOW_S + ' s up to the last fix' + (td ? ', none after touchdown' : ', no touchdown found') });
  }

  // The packet quaternion is Q15 (raw / 32767), so its length is only near 1: it is normalised
  // here before use. The world frame is z up, from fall_replay.py's felt-gravity check (world up
  // in sensor axes reads 9.81 m/s2 at rest on every board). That x is east and y magnetic north is
  // the SH2 rotation vector's convention, not checked on our boards, so a heading is magnetic and
  // could be off by a quarter turn if that assumption is wrong.
  function orientation(q) {
    if (!q) return null;
    const n = Math.hypot(q[0], q[1], q[2], q[3]);
    if (!(n > 0)) return null;
    const [r, i, j, k] = q.map((v) => v / n);
    const up = [2 * (i * k - r * j), 2 * (j * k + r * i), 1 - 2 * (i * i + j * j)];
    const xAxis = [1 - 2 * (j * j + k * k), 2 * (i * j + r * k), 2 * (i * k - r * j)];
    const flat = Math.hypot(xAxis[0], xAxis[1]);
    return {
      q: [r, i, j, k], up,
      tilt: Math.acos(Math.max(-1, Math.min(1, up[2]))) * 180 / Math.PI,
      heading: flat < 0.1 ? null : ((Math.atan2(xAxis[0], xAxis[1]) * 180 / Math.PI) + 360) % 360,
    };
  }

  // The last sample at or before t, or -1
  function at(samples, t) {
    let lo = 0;
    let hi = samples.length - 1;
    let best = -1;
    while (lo <= hi) {
      const m = (lo + hi) >> 1;
      if (samples[m].t <= t) { best = m; lo = m + 1; } else hi = m - 1;
    }
    return best;
  }

  function lastWith(samples, i, test) {
    for (let k = i; k >= 0; k--) if (test(samples[k])) return k;
    return -1;
  }

  // Derived: the chosen height's difference over at least 1 s, back to the nearest reading with
  // its flag set; the time it spans is given with it.
  function vspeed(samples, i, h) {
    if (!h.ok || i < 0 || !h.ok(samples[i]) || h.of(samples[i]) === null) return null;
    const t = samples[i].t;
    const j = lastWith(samples, i - 1, (s) => s.t <= t - VSPEED_WINDOW_S && h.ok(s) && h.of(s) !== null);
    if (j < 0) return null;
    const dt = t - samples[j].t;
    return { v: (h.of(samples[i]) - h.of(samples[j])) / dt, dt };
  }

  // Derived: the distance from the fix at least 1 s before the last one, over the time between.
  // Flash records repeat a fix until the GPS gives the next, so neighbours would read 0 m/s.
  function hspeed(samples, i) {
    const a = lastWith(samples, i, isFix);
    if (a < 0) return null;
    const b = lastWith(samples, a - 1, (s) => isFix(s) && s.t <= samples[a].t - VSPEED_WINDOW_S);
    if (b < 0) return null;
    const dt = samples[a].t - samples[b].t;
    return { v: metres(samples[b].lat, samples[b].lon, samples[a].lat, samples[a].lon) / dt, dt };
  }

  // The readings at time t, each from the last sample at or before t that has one, with how long
  // before t that sample was. Nothing is carried between samples but that.
  function readingsAt(samples, h, t) {
    const i = at(samples, t);
    if (i < 0) return null;
    const hi = lastWith(samples, i, (s) => h.of(s) !== null);
    const fi = lastWith(samples, i, isFix);
    const qi = lastWith(samples, i, (s) => s.q !== null);
    const gi = lastWith(samples, i, (s) => s.gyro !== null);
    const o = qi < 0 ? null : orientation(samples[qi].q);
    const before = (k) => (k < 0 ? 0 : t - samples[k].t);
    return {
      source: h.source,
      height: hi < 0 ? null : { v: h.of(samples[hi]), ok: h.ok(samples[hi]) },
      vs: vspeed(samples, hi, h), hs: hspeed(samples, i),
      fix: fi < 0 ? null : samples[fi],
      o: o && Object.assign(o, { ok: samples[qi].ok.quat }),
      gyro: gi < 0 ? null : { v: hyp(samples[gi].gyro), ok: samples[gi].ok.gyro },
      ages: { height: before(hi), fix: before(fi), quat: before(qi) },
    };
  }

  // The trail as drawn: dots at fixes, a dashed segment between neighbouring fixes, and a break,
  // with what was lost, wherever a gap lies between them. hOf gives a dot its height for 3D, or
  // null where it has none. A flash record repeats its fix until the GPS gives the next one, so
  // a dot with the same place as the one before is marked again: one measurement on the ground,
  // a dot of its own in the air where the barometer moved.
  function trail(samples, gaps, hOf) {
    const breaks = new Map(gaps.map((g) => [g.i, g.words]));
    const dots = [];
    const lines = [];
    const marks = [];
    let line = [];
    let lost = [];
    let prev = null;
    samples.forEach((s, i) => {
      if (breaks.has(i)) lost.push(breaks.get(i));
      if (!isFix(s)) return;
      const dot = { lon: s.lon, lat: s.lat, h: hOf ? hOf(s) : null, held: s.held, t: s.t, i };
      if (lost.length && prev) {
        if (line.length > 1) lines.push(line);
        line = [];
        marks.push({ lon: (prev.lon + s.lon) / 2, lat: (prev.lat + s.lat) / 2, words: lost.join(', ') });
      }
      lost = [];
      dot.joined = line.length > 0;
      dot.again = dot.joined && prev.lat === dot.lat && prev.lon === dot.lon;
      line.push(dot);
      dots.push(dot);
      prev = dot;
    });
    if (line.length > 1) lines.push(line);
    return { dots, lines, marks };
  }

  // The flights a boot can be looked at as, from its phase events: the whole boot, then each air
  // spell (one air segment or several in a row) from 30 s before it to 60 s after it ended.
  function flightsOf(b) {
    const out = [{ kind: 'boot', n: b.n, fromMs: b.firstMs, toMs: b.lastMs, label: 'Boot ' + b.n + ', the whole boot' }];
    const segs = b.phases || [];
    let k = 0;
    for (let i = 0; i < segs.length; i++) {
      if (!segs[i].phase.startsWith('air')) continue;
      let j = i;
      while (j + 1 < segs.length && segs[j + 1].phase.startsWith('air')) j++;
      const falling = segs.slice(i, j + 1).find((s) => s.phase === 'air, falling');
      const landed = j + 1 < segs.length ? segs[j + 1] : null;
      const end = landed ? landed.fromMs : segs[j].toMs;
      k++;
      out.push({
        kind: 'air', n: b.n, spell: k, toAirMs: segs[i].fromMs, releaseMs: falling ? falling.fromMs : null,
        landedMs: landed ? landed.fromMs : null, landedPhase: landed ? landed.phase : null,
        fromMs: Math.max(b.firstMs, segs[i].fromMs - 30000), toMs: Math.min(b.lastMs, end + 60000),
        label: 'Boot ' + b.n + ', air spell ' + k,
      });
      i = j;
    }
    return out;
  }

  return {
    STILL_ACCEL_MPS2, STILL_GYRO_DPS, STILL_FOR_S, FIX_WINDOW_S, VSPEED_WINDOW_S,
    fromHistory, fromTrack, counterGaps, trackGaps, isFix, zeroFixes, heights, touchdown, release,
    metres, bearing, compass, spot, findSpot, orientation, at, vspeed, hspeed, readingsAt, trail, flightsOf,
  };
});
