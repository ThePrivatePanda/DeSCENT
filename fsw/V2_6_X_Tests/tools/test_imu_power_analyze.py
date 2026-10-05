"""Unit tests for imu_power_analyze.py on synthetic INA260 + ChipSat logs.

Run with:
    python3 tools/test_imu_power_analyze.py
    python3 -m unittest discover -s tools -p 'test_*.py' -v
"""

import contextlib
import io
import os
import random
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import imu_power_analyze as ipa  # noqa: E402

LEVEL = {"A": 20.0, "B": 31.0, "C": 27.0, "D": 22.5, "E": 21.5}
ORDER = ["A", "B", "C", "D", "E", "A", "B", "C", "D", "E", "A"]
START_MS, PHASE_MS = 10000, 60000
INA_T0 = 23 * 3600 + 58 * 60 + 50.0   # 23:58:50.000 -> the run crosses midnight
POWERON = INA_T0 + 10.037             # true power-on edge, between two 10 Hz samples
LATENCY = 0.02                        # host receive latency of ChipSat lines


def hms(t):
    return ipa.fmt_hms(t)


def make_logs(d, powered_at_start=False):
    """Write ina.log (serial_log.py format) and chipsat.log (Arduino IDE '->' format)."""
    rng = random.Random(1234)
    end = POWERON + (START_MS + len(ORDER) * PHASE_MS) / 1000.0 + 5.0
    ina = [f"# 2026-09-10T23:58:49 port=/dev/ttyACM0 baud=115200",
           f"{hms(INA_T0 - 0.5)}  Found INA260 chip",
           f"{hms(INA_T0 - 0.4)}  busVoltage_mV,current_mA,peak_mA", ""]
    peak, k = 0.0, 0
    while True:
        t = INA_T0 + k * 0.1
        if t > end:
            break
        rel = t - POWERON
        if rel < 0:
            ma = 25.0 if powered_at_start else abs(rng.gauss(0.0, 0.05))
        elif rel < START_MS / 1000.0:
            ma = 25.0 + rng.gauss(0, 0.2)
        else:
            i = int((rel - START_MS / 1000.0) // (PHASE_MS / 1000.0))
            ma = LEVEL[ORDER[i] if i < len(ORDER) else "A"] + rng.gauss(0, 0.2)
        peak = max(peak, ma)
        ina.append(f"{hms(t)}  {3300 + rng.gauss(0, 2):.2f},{ma:.2f},{peak:.2f}")
        if k == 500:
            ina.append(f"{hms(t)}  garbage ,, line")
        k += 1

    def cs(ms, text):
        return f"{hms(POWERON + ms / 1000.0 + LATENCY)} -> {text}"

    chip = [cs(200, "IMU power test boot"), "",
            cs(300, f"SCHED,start_ms={START_MS},phase_ms={PHASE_MS},order={','.join(ORDER)}")]
    for i, c in enumerate(ORDER):
        s = START_MS + i * PHASE_MS
        e = s + PHASE_MS
        chip.append(cs(s, f"PH,{i},{c},START,{s}"))
        if c in "DE":
            for cyc in range(6):
                if c == "D":
                    w = (f"on=1,en=5,la={100 + 10 * cyc},gy={105 + 10 * cyc},mg={200 + 10 * cyc},"
                         f"rv={150 + 10 * cyc},awake={250 + 10 * cyc},ok=1")
                else:
                    w = "on=1,en=-1,la=-1,gy=-1,mg=-1,rv=-1,awake=1000,ok=0"
                chip.append(cs(s + 1000 + cyc * 10000, f"WAKE,{i},{c},{cyc},{w}"))
        n = {"A": 0, "B": 1200, "C": 120, "D": 6, "E": 0}[c]
        vla = n // 10 if c == "B" else n
        chip.append(cs(e, f"PH,{i},{c},END,{e},la={n},gy={n},mg={n},rv={n},vla={vla},vgy={n},"
                          f"vmg={n},vrv={n},svc={n * 2},rst=0"))
        chip.append(cs(e + 1, "some debug text"))
    ina_p, chip_p = os.path.join(d, "ina.log"), os.path.join(d, "chipsat.log")
    with open(ina_p, "w") as f:
        f.write("\n".join(ina) + "\n")
    with open(chip_p, "w") as f:
        f.write("\n".join(chip) + "\n")
    return ina_p, chip_p


class Base(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = self._tmp.name
        self.ina, self.chip = make_logs(self.dir)

    def tearDown(self):
        self._tmp.cleanup()

    def check_means(self, res):
        self.assertEqual([r["code"] for r in res["rows"]], ORDER)
        for r in res["rows"]:
            self.assertGreaterEqual(r["n"], 500)
            self.assertAlmostEqual(r["mean"], LEVEL[r["code"]], delta=0.3, msg=f"phase {r['idx']}")
        bc = res["by_code"]
        for c, lvl in LEVEL.items():
            self.assertAlmostEqual(bc[c]["mean"], lvl, delta=0.3)
            self.assertAlmostEqual(bc[c]["delta"], lvl - LEVEL["A"], delta=0.3)
        self.assertEqual(bc["A"]["inst"], 3)
        self.assertEqual(bc["B"]["inst"], 2)
        v = res["verdict"]
        self.assertAlmostEqual(v["B_minus_D"], 8.5, delta=0.3)
        self.assertAlmostEqual(v["wake_mAs"]["D"], 25.0, delta=3.0)
        est60 = dict(v["extrap"])[60]["D"]
        self.assertAlmostEqual(est60, v["wake_mAs"]["D"] / 60.0, places=9)
        self.assertEqual([T for T, _ in v["extrap"]], [10, 30, 60, 120, 180])


class TestParsing(Base):
    def test_formats_garbage_and_midnight(self):
        s = ipa.parse_ina(self.ina)
        self.assertTrue(all(s[i][0] < s[i + 1][0] for i in range(len(s) - 1)))
        self.assertGreater(s[-1][0], 86400.0)          # rolled over midnight
        self.assertAlmostEqual(s[0][0], INA_T0, places=3)
        chip = ipa.parse_chipsat(self.chip)
        self.assertEqual(chip["sched"]["order"], ORDER)
        self.assertEqual(len(chip["phases"]), len(ORDER))
        self.assertEqual(len(chip["wakes"]), 24)
        self.assertTrue(all("t0" in p and "t1" in p for p in chip["phases"]))

    def test_hms(self):
        self.assertAlmostEqual(ipa.parse_hms("14:03:07.123"), 14 * 3600 + 187.123, places=6)
        self.assertEqual(ipa.fmt_hms(86400 + 5.5), "00:00:05.500")


class TestChipsatMode(Base):
    def test_means_deltas(self):
        res = ipa.analyze(self.ina, self.chip)
        self.check_means(res)
        self.assertIn("PH START/END", res["mode"])
        self.assertEqual(res["warnings"], [])
        self.assertAlmostEqual(res["rows"][0]["t0"], POWERON + START_MS / 1000 + LATENCY, places=3)

    def test_wake_summary(self):
        ws = ipa.analyze(self.ina, self.chip)["wake_summary"]
        d, e = ws["D"], ws["E"]
        self.assertEqual((d["count"], d["ok"]), (12, 12))
        self.assertEqual(d["la"]["median"], 125)
        self.assertEqual(d["gy"]["median"], 130)
        self.assertEqual(d["mg"]["median"], 225)
        self.assertEqual(d["rv"]["median"], 175)
        self.assertEqual(d["awake"]["median"], 275)
        self.assertEqual(d["awake"]["max"], 300)
        self.assertEqual(d["la"]["missing"], 0)
        self.assertEqual((e["count"], e["ok"]), (12, 0))
        self.assertIsNone(e["la"]["median"])
        self.assertEqual(e["la"]["missing"], 12)
        self.assertEqual(e["awake"]["median"], 1000)

    def test_visible_drop_flag(self):
        res = ipa.analyze(self.ina, self.chip)
        flags = res["drop_flag"]
        self.assertEqual(sorted((i, c, k) for i, c, k, _ in flags), [(1, "B", "la"), (6, "B", "la")])
        for *_, pct in flags:
            self.assertAlmostEqual(pct, 10.0)
        b = [r for r in res["reports"] if r["idx"] == 1][0]
        self.assertAlmostEqual(b["la"][0], 20.0)
        self.assertAlmostEqual(b["la"][1], 2.0)
        text = ipa.format_report(res)
        self.assertIn("library shared-slot drops reports", text)
        self.assertIn("IMU cost above asleep baseline", text)
        self.assertIn("assumes the per-wake cost is independent of interval".lower(), text.lower())


class TestInaOnlyMode(Base):
    def test_means_deltas(self):
        res = ipa.analyze(self.ina)
        self.check_means(res)
        self.assertAlmostEqual(res["poweron"], POWERON, delta=0.11)
        self.assertIsNone(res["reports"])
        self.assertEqual(res["warnings"], [])

    def test_poweron_override(self):
        res = ipa.analyze(self.ina, poweron=hms(POWERON))
        self.assertAlmostEqual(res["poweron"], POWERON, places=3)
        self.check_means(res)

    def test_starts_powered_warns(self):
        ina, _ = make_logs(os.path.join(self.dir), powered_at_start=True)
        res = ipa.analyze(ina)
        self.assertTrue(any("alignment is unreliable" in w for w in res["warnings"]))

    def test_poweron_skips_glitch(self):
        s = [(i * 0.1, ma) for i, ma in enumerate([0.1, 0.1, 5.0, 0.1, 0.1, 0.1, 25, 25, 25])]
        warns = []
        self.assertAlmostEqual(ipa.find_poweron(s, 3.0, warns.append), 0.6)
        self.assertEqual(warns, [])

    def test_short_window_warns(self):
        res = ipa.analyze(self.ina, phase_ms=5000)
        self.assertTrue(any("fewer than" in w or "only" in w for w in res["warnings"]))

    def test_cli_csv(self):
        out_csv = os.path.join(self.dir, "t1.csv")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            ipa.main(["--ina", self.ina, "--csv", out_csv])
        self.assertIn("1. Per phase instance", buf.getvalue())
        with open(out_csv) as f:
            lines = f.read().splitlines()
        self.assertEqual(lines[0].split(",")[:2], ["idx", "code"])
        self.assertEqual(len(lines), 1 + len(ORDER))


if __name__ == "__main__":
    unittest.main()
