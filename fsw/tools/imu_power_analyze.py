import argparse, csv, re, statistics, sys

PHASE_DESC = {
    "A": "IMU asleep",
    "B": "streaming 4 reports @50 ms",
    "C": "streaming 4 reports @500 ms",
    "D": "duty cycle 10 s, reports re-enabled each wake",
    "E": "duty cycle 10 s, WITHOUT report re-enable",
}
REPORTS = ("la", "gy", "mg", "rv")
DUTY_S = 10.0                      # duty-cycle interval used by phases D and E in the sketch
EXTRAP_T = (10, 30, 60, 120, 180)  # intervals (s) for the D/E extrapolation
MIN_SAMPLES = 50
MIN_RATE_HZ = 5.0
HALF_DAY = 43200.0

LINE_RE = re.compile(r"^\s*(\d{1,2}):(\d{2}):(\d{2})(?:[.,](\d{1,6}))?\s+(?:->\s?)?(.*?)\s*$")


def parse_hms(s):
    """'HH:MM:SS(.mmm)' -> seconds of day (float)."""
    m = re.fullmatch(r"\s*(\d{1,2}):(\d{2}):(\d{2})(?:[.,](\d{1,6}))?\s*", s)
    if not m:
        raise ValueError(f"bad time {s!r}, expected HH:MM:SS.mmm")
    h, mi, se, frac = m.groups()
    if int(h) > 23 or int(mi) > 59 or int(se) > 59:
        raise ValueError(f"bad time {s!r}, out of range")
    return int(h) * 3600 + int(mi) * 60 + int(se) + (float("0." + frac) if frac else 0.0)


def fmt_hms(t):
    t = t % 86400.0
    ms = int(round(t * 1000))
    h, ms = divmod(ms, 3600000)
    m, ms = divmod(ms, 60000)
    s, ms = divmod(ms, 1000)
    return f"{h % 24:02d}:{m:02d}:{s:02d}.{ms:03d}"


def read_stamped(path):
    """Return [(t_seconds, payload)] for every timestamped line; midnight rollover unwrapped."""
    out, offset, prev = [], 0.0, None
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            m = LINE_RE.match(line)
            if not m:
                continue
            h, mi, se, frac, payload = m.groups()
            t = int(h) * 3600 + int(mi) * 60 + int(se) + (float("0." + frac) if frac else 0.0)
            if prev is not None and t + offset < prev - HALF_DAY:
                offset += 86400.0
            t += offset
            prev = t
            out.append((t, payload))
    return out


def parse_ina(path, channel=1):
    """Return [(t, current_mA)] from an INA260 log; unparseable lines are skipped.

    Accepts the single-INA logger (`mV,mA` or `mV,mA,peak`) and the lab VI_Relay rig
    (`mV , mA , mV , mA , mV , mA`, one pair per relay); `channel` picks the relay (1-3).
    """
    samples = []
    for t, payload in read_stamped(path):
        parts = payload.split(",")
        if len(parts) in (2, 3):
            col = 1
        elif len(parts) == 6:
            col = 2 * (channel - 1) + 1
        else:
            continue
        try:
            vals = [float(p) for p in parts]
        except ValueError:
            continue
        samples.append((t, vals[col]))
    return samples


def _kv(fields):
    d = {}
    for f in fields:
        if "=" in f:
            k, v = f.split("=", 1)
            try:
                d[k.strip()] = int(v)
            except ValueError:
                pass
    return d


def parse_chipsat(path):
    """Return {'sched', 'phases', 'wakes'} from the ChipSat test-sketch log."""
    sched, phases, wakes = None, {}, []
    for t, payload in read_stamped(path):
        p = payload.strip()
        try:
            if p.startswith("SCHED,"):
                m = re.search(r"start_ms=(\d+).*?phase_ms=(\d+).*?order=([A-Z,]+)", p)
                if m:
                    sched = {"start_ms": int(m.group(1)), "phase_ms": int(m.group(2)),
                             "order": [c for c in m.group(3).split(",") if c]}
            elif p.startswith("PH,"):
                f = p.split(",")
                idx, code, kind, ms = int(f[1]), f[2], f[3], int(f[4])
                ph = phases.setdefault(idx, {"idx": idx, "code": code})
                if kind == "START":
                    ph.update(t0=t, start_ms=ms)
                elif kind == "END":
                    ph.update(t1=t, end_ms=ms, counts=_kv(f[5:]))
            elif p.startswith("WAKE,"):
                f = p.split(",")
                w = {"idx": int(f[1]), "code": f[2], "cycle": int(f[3]), "t": t}
                w.update(_kv(f[4:]))
                wakes.append(w)
        except (IndexError, ValueError):
            continue
    return {"sched": sched, "phases": [phases[k] for k in sorted(phases)], "wakes": wakes}


def shift_to(chip, ref_t):
    """Put the ChipSat timeline on the same day as the INA timeline (midnight between starts)."""
    ts = [ph[k] for ph in chip["phases"] for k in ("t0", "t1") if k in ph]
    if not ts:
        return
    d = 0.0
    if min(ts) < ref_t - HALF_DAY:
        d = 86400.0
    elif min(ts) > ref_t + HALF_DAY:
        d = -86400.0
    if d:
        for ph in chip["phases"]:
            for k in ("t0", "t1"):
                if k in ph:
                    ph[k] += d
        for w in chip["wakes"]:
            w["t"] += d


def find_poweron(samples, thr, warn):
    """First sample above thr preceded by >=3 samples below it; returns its time."""
    if samples[0][1] > thr:
        warn("INA log starts already powered (first sample above threshold): "
             "alignment is unreliable; pass --poweron HH:MM:SS.mmm")
        return samples[0][0]
    below = 0
    for t, ma in samples:
        if ma > thr:
            if below >= 3:
                return t
            below = 0
        else:
            below += 1
    for t, ma in samples:
        if ma > thr:
            warn(f"no sample above {thr} mA is preceded by 3 samples below it; using the first "
                 f"one at {fmt_hms(t)}: alignment may be unreliable; consider --poweron")
            return t
    raise SystemExit(f"no INA sample above --on-threshold-ma {thr}: cannot find power-on")


def resolve_hms(hms, samples):
    """Map a HH:MM:SS.mmm override onto the (possibly >24 h) INA timeline."""
    t = parse_hms(hms)
    first, last = samples[0][0], samples[-1][0]
    cands = [t + 86400.0 * k for k in range(int(last // 86400) + 2)]
    inside = [c for c in cands if first <= c <= last]
    return inside[0] if inside else min(cands, key=lambda c: abs(c - first))


def window_stats(samples, t0, t1, trim):
    vals = [ma for t, ma in samples if t0 + trim <= t <= t1 - trim]
    n = len(vals)
    if not n:
        return {"n": 0, "mean": None, "std": None, "median": None, "min": None, "max": None,
                "vals": vals}
    return {"n": n, "mean": statistics.fmean(vals),
            "std": statistics.stdev(vals) if n > 1 else 0.0,
            "median": statistics.median(vals), "min": min(vals), "max": max(vals), "vals": vals}


def analyze(ina_path, chipsat_path=None, start_ms=10000, phase_ms=60000,
            order="A,B,C,D,E,A,B,C,D,E,A", on_threshold_ma=3.0, poweron=None, trim_s=3.0,
            channel=1):
    """Run the whole analysis; returns a result dict (see format_report)."""
    warnings = []
    samples = parse_ina(ina_path, channel)
    if len(samples) < 2:
        raise SystemExit(f"{ina_path}: fewer than 2 INA samples parsed")
    if statistics.median(ma for _, ma in samples) < -on_threshold_ma:
        samples = [(t, -ma) for t, ma in samples]
        warnings.append("current is mostly negative (INA260 Vin+/Vin- swapped?): sign flipped")
    span = samples[-1][0] - samples[0][0]
    rate = (len(samples) - 1) / span if span > 0 else 0.0
    if rate < MIN_RATE_HZ:
        warnings.append(f"INA sample rate {rate:.2f} Hz is below {MIN_RATE_HZ:g} Hz")

    chip, windows, mode, pon = None, [], "", None
    if chipsat_path:
        chip = parse_chipsat(chipsat_path)
        shift_to(chip, samples[0][0])
        for ph in chip["phases"]:
            if "t0" in ph and "t1" in ph:
                windows.append((ph["idx"], ph["code"], ph["t0"], ph["t1"]))
            else:
                warnings.append(f"phase {ph['idx']} ({ph['code']}) lacks START or END; skipped")
        mode = "ChipSat PH START/END host timestamps"
        if not windows:
            warnings.append("ChipSat log has no complete PH START/END pairs; "
                            "falling back to schedule + INA power-on")
            if chip["sched"]:
                s = chip["sched"]
                start_ms, phase_ms, order = s["start_ms"], s["phase_ms"], ",".join(s["order"])
    if not windows:
        if poweron:
            pon = resolve_hms(poweron, samples)
            mode = f"schedule from power-on override {fmt_hms(pon)}"
        else:
            pon = find_poweron(samples, on_threshold_ma, warnings.append)
            mode = f"schedule from INA power-on edge {fmt_hms(pon)} (> {on_threshold_ma:g} mA)"
        codes = [c.strip() for c in order.split(",") if c.strip()]
        for i, c in enumerate(codes):
            t0 = pon + (start_ms + i * phase_ms) / 1000.0
            windows.append((i, c, t0, t0 + phase_ms / 1000.0))
        if samples[-1][0] < windows[-1][3]:
            warnings.append(f"INA log ends at {fmt_hms(samples[-1][0])}, before the last "
                            f"scheduled phase ends at {fmt_hms(windows[-1][3])}")

    rows = []
    for idx, code, t0, t1 in windows:
        st = window_stats(samples, t0, t1, trim_s)
        if st["n"] < MIN_SAMPLES:
            warnings.append(f"phase {idx} ({code}) has only {st['n']} INA samples (< {MIN_SAMPLES})")
        rows.append(dict(st, idx=idx, code=code, t0=t0, t1=t1))

    codes = {}
    for r in rows:
        if r["n"]:
            codes.setdefault(r["code"], []).append(r)
    by_code = {}
    for c, rs in sorted(codes.items()):
        pooled = [v for r in rs for v in r["vals"]]
        means = [r["mean"] for r in rs]
        by_code[c] = {"mean": statistics.fmean(pooled), "n": len(pooled), "inst": len(rs),
                      "inst_min": min(means), "inst_max": max(means)}
    base = by_code.get("A", {}).get("mean")
    for c, d in by_code.items():
        d["delta"] = d["mean"] - base if base is not None else None
    if base is None:
        warnings.append("no phase A (asleep) data: deltas cannot be computed")

    res = {"ina_path": ina_path, "chipsat_path": chipsat_path, "n_samples": len(samples),
           "rate_hz": rate, "first_t": samples[0][0], "last_t": samples[-1][0],
           "mode": mode, "poweron": pon, "trim_s": trim_s, "rows": rows, "by_code": by_code,
           "verdict": verdict(by_code), "warnings": warnings, "chip": chip,
           "reports": None, "drop_flag": [], "wake_summary": None}
    if chip:
        res["reports"], res["drop_flag"] = report_rates(chip)
        res["wake_summary"] = wake_summary(chip["wakes"])
    return res


def verdict(by_code):
    dl = {c: d["delta"] for c, d in by_code.items() if d["delta"] is not None}
    v = {c: dl.get(c) for c in "BCDE"}
    v["B_minus_D"] = v["B"] - v["D"] if v["B"] is not None and v["D"] is not None else None
    v["wake_mAs"] = {c: dl[c] * DUTY_S for c in "DE" if c in dl}
    v["extrap"] = [(T, {c: e / T for c, e in v["wake_mAs"].items()}) for T in EXTRAP_T]
    return v


def report_rates(chip):
    out, flags = [], []
    for ph in chip["phases"]:
        cnt = ph.get("counts")
        if cnt is None:
            continue
        if "start_ms" in ph and "end_ms" in ph and ph["end_ms"] > ph["start_ms"]:
            dur = (ph["end_ms"] - ph["start_ms"]) / 1000.0
        else:
            dur = ph["t1"] - ph["t0"]
        r = {"idx": ph["idx"], "code": ph["code"], "dur": dur,
             "svc": cnt.get("svc", 0) / dur if dur > 0 else 0.0, "rst": cnt.get("rst", 0)}
        for k in REPORTS:
            raw, vis = cnt.get(k, 0), cnt.get("v" + k, 0)
            pct = 100.0 * vis / raw if raw else None
            r[k] = (raw / dur if dur > 0 else 0.0, vis / dur if dur > 0 else 0.0, pct)
            if pct is not None and pct < 50.0:
                flags.append((ph["idx"], ph["code"], k, pct))
        out.append(r)
    return out, flags


def wake_summary(wakes):
    out = {}
    for c in sorted({w["code"] for w in wakes}):
        ws = [w for w in wakes if w["code"] == c]
        d = {"count": len(ws), "ok": sum(1 for w in ws if w.get("ok") == 1)}
        for k in REPORTS + ("awake",):
            vals = [w[k] for w in ws if k in w and w[k] >= 0]
            d[k] = {"median": statistics.median(vals) if vals else None,
                    "max": max(vals) if vals else None,
                    "missing": sum(1 for w in ws if w.get(k, -1) < 0)}
        out[c] = d
    return out


def _f(x, nd=2, w=7):
    return f"{'-':>{w}}" if x is None else f"{x:{w}.{nd}f}"


def _x(x):
    return "n/a" if x is None else f"{x:.2f}"


def format_report(res):
    L = []
    L.append(f"INA log   : {res['ina_path']}  ({res['n_samples']} samples, {res['rate_hz']:.2f} Hz, "
             f"{fmt_hms(res['first_t'])} .. {fmt_hms(res['last_t'])})")
    if res["chipsat_path"]:
        L.append(f"ChipSat   : {res['chipsat_path']}")
        s = res["chip"]["sched"]
        if s:
            L.append(f"SCHED     : start_ms={s['start_ms']} phase_ms={s['phase_ms']} "
                     f"order={','.join(s['order'])}")
    L.append(f"Windows   : {res['mode']}; trimmed {res['trim_s']:g} s at each end")
    L.append("Phases    : " + "; ".join(f"{c} = {d}" for c, d in PHASE_DESC.items()))
    for w in res["warnings"]:
        L.append(f"WARNING: {w}")

    L.append("")
    L.append("1. Per phase instance (mA)")
    L.append(f"{'idx':>3} {'code':<4} {'start':<12} {'n':>5} {'mean':>7} {'std':>7} "
             f"{'median':>7} {'min':>7} {'max':>7}")
    for r in res["rows"]:
        L.append(f"{r['idx']:>3} {r['code']:<4} {fmt_hms(r['t0']):<12} {r['n']:>5} "
                 f"{_f(r['mean'])} {_f(r['std'])} {_f(r['median'])} {_f(r['min'])} {_f(r['max'])}")

    L.append("")
    L.append("2. Per phase code (instances pooled; delta = IMU cost above asleep baseline A)")
    L.append(f"{'code':<4} {'inst':>4} {'n':>6} {'mean':>7} {'spread':>7} {'delta':>7}  description")
    for c, d in res["by_code"].items():
        L.append(f"{c:<4} {d['inst']:>4} {d['n']:>6} {_f(d['mean'])} "
                 f"{_f(d['inst_max'] - d['inst_min'])} {_f(d['delta'])}  {PHASE_DESC.get(c, '?')}")
    L.append("     spread = max - min of the per-instance means")

    v = res["verdict"]
    L.append("")
    L.append("3. Verdict")
    L.append(f"Streaming @50 ms (B) costs {_x(v['B'])} mA over asleep; duty-cycle with re-enable "
             f"(D) costs {_x(v['D'])} mA; without re-enable (E) costs {_x(v['E'])} mA; slower "
             f"streaming (C) costs {_x(v['C'])} mA.")
    L.append(f"At a {DUTY_S:g} s interval D saves {_x(v['B_minus_D'])} mA vs B.")
    if v["wake_mAs"]:
        L.append("Extrapolation to longer duty-cycle intervals (ASSUMES the per-wake cost is "
                 "independent of interval):")
        L.append("  " + ", ".join(f"energy per wake {c} = ({c} - A) * {DUTY_S:g} s = {e:.1f} mA*s"
                                  for c, e in v["wake_mAs"].items()))
        hdr = f"  {'T (s)':>6}" + "".join(f" {c + ' est mA':>9}" for c in v["wake_mAs"])
        hdr += f" {'B delta':>8}" + ("  D saves vs B" if "D" in v["wake_mAs"] else "")
        L.append(hdr)
        for T, est in v["extrap"]:
            line = f"  {T:>6}" + "".join(f" {est[c]:>9.3f}" for c in v["wake_mAs"])
            line += f" {_x(v['B']):>8}"
            if "D" in est and v["B"] is not None:
                line += f"  {v['B'] - est['D']:>12.3f}"
            L.append(line)

    if res["reports"] is not None:
        L.append("")
        L.append("4. IMU reports per phase (reports/s: raw / visible (visible % of raw))")
        L.append(f"{'idx':>3} {'code':<4} {'dur_s':>6}  " +
                 "  ".join(f"{k:^20}" for k in REPORTS) + f"  {'svc/s':>6} {'rst':>3}")
        for r in res["reports"]:
            cells = []
            for k in REPORTS:
                raw, vis, pct = r[k]
                cells.append(f"{raw:6.2f}/{vis:6.2f} ({'  - ' if pct is None else f'{pct:3.0f}%'})")
            L.append(f"{r['idx']:>3} {r['code']:<4} {r['dur']:>6.1f}  " + "  ".join(cells) +
                     f"  {r['svc']:>6.2f} {r['rst']:>3}")
        if res["drop_flag"]:
            L.append("FLAG: library shared-slot drops reports (visible < 50 % of raw): " +
                     ", ".join(f"phase {i} {c} {k} {p:.0f}%" for i, c, k, p in res["drop_flag"]))
        else:
            L.append("Visible rates are >= 50 % of raw in every phase.")
        rsts = [r for r in res["reports"] if r["rst"]]
        if rsts:
            L.append("NOTE: IMU resets seen in phase(s) " +
                     ", ".join(f"{r['idx']} {r['code']} (rst={r['rst']})" for r in rsts))

        L.append("")
        L.append("   WAKE summary per code (ms since wake start; median/max over arrived; miss = count of -1)")
        ws = res["wake_summary"]
        if not ws:
            L.append("   no WAKE lines in ChipSat log")
        else:
            L.append(f"{'code':<4} {'count':>5} {'ok':>4}  " +
                     "  ".join(f"{k:^17}" for k in REPORTS + ("awake",)))
            L.append(f"{'':<4} {'':>5} {'':>4}  " +
                     "  ".join(f"{'med':>6} {'max':>5} {'miss':>4}" for _ in REPORTS + ("awake",)))
            for c, d in ws.items():
                cells = [f"{_f(d[k]['median'], 0, 6)} {_f(d[k]['max'], 0, 5)} {d[k]['missing']:>4}"
                         for k in REPORTS + ("awake",)]
                L.append(f"{c:<4} {d['count']:>5} {d['ok']:>4}  " + "  ".join(cells))
    return "\n".join(L)


def write_csv(rows, path):
    with open(path, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["idx", "code", "start", "end", "n", "mean_mA", "std_mA", "median_mA",
                    "min_mA", "max_mA"])
        for r in rows:
            w.writerow([r["idx"], r["code"], fmt_hms(r["t0"]), fmt_hms(r["t1"]), r["n"]] +
                       ["" if r[k] is None else f"{r[k]:.4f}"
                        for k in ("mean", "std", "median", "min", "max")])


def main(argv=None):
    ap = argparse.ArgumentParser(description="Per-phase current for the ChipSat IMU power test.")
    ap.add_argument("--ina", required=True, help="INA260 log (HH:MM:SS.mmm prefixed)")
    ap.add_argument("--chipsat", help="ChipSat test-sketch log (PH/WAKE/SCHED lines)")
    ap.add_argument("--start-ms", type=int, default=10000)
    ap.add_argument("--phase-ms", type=int, default=60000)
    ap.add_argument("--order", default="A,B,C,D,E,A,B,C,D,E,A")
    ap.add_argument("--on-threshold-ma", type=float, default=3.0)
    ap.add_argument("--poweron", help="override power-on time HH:MM:SS.mmm (INA-only mode)")
    ap.add_argument("--trim-s", type=float, default=3.0)
    ap.add_argument("--csv", help="write the per-phase-instance table to this CSV")
    ap.add_argument("--channel", type=int, default=1, choices=(1, 2, 3),
                    help="VI_Relay rig: which relay/INA260 the ChipSat is on (default 1)")
    a = ap.parse_args(argv)
    if a.poweron:
        try:
            parse_hms(a.poweron)  # fail early on a malformed value
        except ValueError as e:
            ap.error(str(e))
    res = analyze(a.ina, a.chipsat, a.start_ms, a.phase_ms, a.order, a.on_threshold_ma,
                  a.poweron, a.trim_s, a.channel)
    print(format_report(res))
    if a.csv:
        write_csv(res["rows"], a.csv)
        print(f"\n# wrote {a.csv}")


if __name__ == "__main__":
    main()
