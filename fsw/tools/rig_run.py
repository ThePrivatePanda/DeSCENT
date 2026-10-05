#!/usr/bin/env python3
"""Drive the lab power rig (VI_Relay Arduino + INA260) and the ChipSat console unattended.

usage:
  ./rig_run.py sweep [--durations 1,2,3,5,8,12,20] [--reps 4] [--cut-after 15] [--seed N]
  ./rig_run.py t01   [--runs 2]
  common: [--ina /dev/ttyUSB0] [--chip /dev/ttyUSB1] [--tag NAME] [--channel 1]

sweep: power-cut test. Starting from a clean boot, it cuts the ChipSat's power 'cut-after' s
       after boot, keeps it off for each duration (random order), switches it back on, and
       records whether the board booted (a "t01_imu_power boot" line after the "1") and what
       current it drew. A stuck board is recovered with a long off (25 s) before the next trial.
       Output: logs/<stamp>_<tag>_ina.log, _chip.log, _sweep.csv
t01:   full t01_imu_power runs. Each run: long off, "1", wait for DONE, "0". Each run gets its
       own log pair (<stamp>_<tag>_runN_ina.log / _chip.log) for ./imu_power_analyze.py.

The relay command "1" TOGGLES relay 1 and "0" turns all relays off, so every switch is
confirmed from the rig's voltage column (on ~3300 mV) and "0" is the only way to switch off.
Opening the rig port resets the Arduino, which also switches the relays off.
Logs use the same "HH:MM:SS.mmm  text" format as serial_log.py. The relay is left OFF at exit.
"""
import argparse, csv, datetime as dt, os, random, sys, threading, time
try:
    import serial
except ImportError:
    sys.exit("pyserial missing: pip install --user pyserial")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOGS = os.path.join(ROOT, "logs")
ON_MV, OFF_MV, DRAINED_MV = 3200.0, 3000.0, 100.0
BOOT_TEXT = "t01_imu_power boot"
LONG_OFF_S = 25.0


def stamp():
    return dt.datetime.now().strftime("%H:%M:%S.%f")[:-3]


class Port:
    """Reads one serial port in a thread, writes timestamped lines to the current log file."""

    def __init__(self, dev, name, on_line=None):
        self.ser = serial.Serial(dev, 115200, timeout=0.2)
        self.name, self.on_line = name, on_line
        self.lock = threading.Lock()
        self.log = None
        self.lines = []  # (monotonic time, text) since the last mark
        threading.Thread(target=self._run, daemon=True).start()

    def open_log(self, path):
        with self.lock:
            if self.log:
                self.log.close()
            self.log = open(path, "a", buffering=1)
            self.log.write(f"# {dt.datetime.now().isoformat()} port={self.ser.port} baud=115200\n")

    def write_note(self, text):
        with self.lock:
            if self.log:
                self.log.write(f"{stamp()}  {text}\n")
        print(f"{stamp()}  [{self.name}] {text}", flush=True)

    def _run(self):
        buf = b""
        while True:
            # read what has arrived (at least 1 byte) so lines are timestamped as they come,
            # not up to one read-timeout later
            chunk = self.ser.read(self.ser.in_waiting or 1)
            if not chunk:
                continue
            buf += chunk
            while b"\n" in buf:
                raw, buf = buf.split(b"\n", 1)
                text = raw.decode("utf-8", "replace").rstrip("\r")
                now = time.monotonic()
                with self.lock:
                    if self.log:
                        self.log.write(f"{stamp()}  {text}\n")
                    self.lines.append((now, text))
                if self.on_line:
                    self.on_line(now, text)

    def since(self, t0):
        with self.lock:
            return [(t, x) for t, x in self.lines if t >= t0]


class Rig:
    def __init__(self, dev, channel):
        self.channel = channel
        self.samples = []  # (monotonic, mV, |mA|)
        self.port = Port(dev, "rig", self._on_line)

    def _on_line(self, t, text):
        f = text.split(",")
        if len(f) != 6:
            return
        try:
            mv, ma = float(f[2 * (self.channel - 1)]), float(f[2 * (self.channel - 1) + 1])
        except ValueError:
            return
        self.samples.append((t, mv, abs(ma)))  # rig channel 1 is wired reversed; use magnitude

    def last(self):
        return self.samples[-1] if self.samples else None

    def window(self, t0, t1):
        return [s for s in self.samples if t0 <= s[0] < t1]

    def send(self, cmd):
        self.port.ser.write((cmd + "\n").encode())
        self.port.write_note(f"# sent: {cmd}")
        return time.monotonic()

    def wait_voltage(self, above, timeout):
        """Wait for a sample newer than now that is above/below the threshold."""
        t0 = time.monotonic()
        while time.monotonic() - t0 < timeout:
            s = self.last()
            if s and s[0] > t0 and ((s[1] > ON_MV) if above else (s[1] < OFF_MV)):
                return True
            time.sleep(0.05)
        return False

    def off(self):
        self.send("0")
        if not self.wait_voltage(False, 3):
            raise SystemExit("relay did not switch off (voltage stayed high) - stopping")

    def on(self):
        s = self.last()
        if s and s[1] > OFF_MV:
            raise SystemExit("refusing to send '1': rig voltage says the relay is already on")
        t = self.send("1")
        if not self.wait_voltage(True, 3):
            self.send("0")
            raise SystemExit("relay did not switch on (no ~3300 mV) - sent 0 and stopping")
        return t

    def long_off(self):
        self.off()
        t0 = time.monotonic()
        while True:
            time.sleep(0.25)
            s = self.last()
            if time.monotonic() - t0 >= LONG_OFF_S and s and s[1] < DRAINED_MV:
                return
            if time.monotonic() - t0 > 90:
                return  # rail floor stays near 80 mV; do not wait forever


def mean(xs):
    return sum(xs) / len(xs) if xs else float("nan")


def power_on_and_check(rig, chip, boot_window=3.0, observe=12.0):
    """Switch on, then report boot line latency and the current signature."""
    pre = rig.last()
    residual = pre[1] if pre else float("nan")
    t_on = rig.on()
    time.sleep(observe)
    boots = [t for t, x in chip.since(t_on) if BOOT_TEXT in x]
    latency = (boots[0] - t_on) * 1000 if boots and boots[0] - t_on <= boot_window else None
    peak = max((s[2] for s in rig.window(t_on, t_on + 3)), default=float("nan"))
    early = mean([s[2] for s in rig.window(t_on + 0.3, t_on + 2.0)])
    late = mean([s[2] for s in rig.window(t_on + 5, t_on + observe)])
    return t_on, residual, latency, peak, early, late


def open_logs(rig, chip, base):
    rig.port.open_log(base + "_ina.log")
    chip.open_log(base + "_chip.log")


def start_session(args):
    os.makedirs(LOGS, exist_ok=True)
    chip = Port(args.chip, "chip")
    rig = Rig(args.ina, args.channel)
    t0 = time.monotonic()
    while not rig.last() and time.monotonic() - t0 < 15:
        time.sleep(0.2)  # the Arduino resets when the port opens; wait for readings
    if not rig.last():
        raise SystemExit("no INA readings from the rig port - check the port name")
    return rig, chip


def ensure_clean_boot(rig, chip, label, writer=None, attempts=2):
    for attempt in range(attempts):
        rig.long_off()
        t_on, residual, latency, peak, early, late = power_on_and_check(rig, chip)
        ok = latency is not None
        rig.port.write_note(f"# {label}: residual={residual:.0f}mV boot={'yes' if ok else 'NO'}")
        if writer:
            writer.writerow({"trial": label, "off_s": LONG_OFF_S, "residual_mV": f"{residual:.1f}",
                             "boot": int(ok), "boot_ms": f"{latency:.0f}" if ok else "",
                             "peak_mA": f"{peak:.2f}", "early_mA": f"{early:.2f}",
                             "late_mA": f"{late:.2f}", "result": "clean" if ok else "stuck"})
        if ok:
            return t_on
    rig.off()
    raise SystemExit(f"{label}: board did not boot after {attempts} long power cycles - relay left off")


def run_sweep(args):
    rig, chip = start_session(args)
    base = os.path.join(LOGS, f"{dt.datetime.now():%Y%m%d_%H%M%S}_{args.tag}")
    open_logs(rig, chip, base)
    durations = [float(x) for x in args.durations.split(",")]
    order = [d for d in durations for _ in range(args.reps)]
    seed = args.seed if args.seed is not None else random.randrange(10**6)
    random.Random(seed).shuffle(order)
    rig.port.write_note(f"# sweep seed={seed} cut_after={args.cut_after} order={order}")
    fields = ["trial", "off_s", "residual_mV", "boot", "boot_ms", "peak_mA", "early_mA", "late_mA", "result"]
    with open(base + "_sweep.csv", "w", newline="") as fcsv:
        w = csv.DictWriter(fcsv, fieldnames=fields)
        w.writeheader()
        t_boot = ensure_clean_boot(rig, chip, "start", w)
        for i, d in enumerate(order):
            while time.monotonic() - t_boot < args.cut_after:
                time.sleep(0.05)
            rig.off()
            time.sleep(d)
            t_on, residual, latency, peak, early, late = power_on_and_check(rig, chip)
            ok = latency is not None
            w.writerow({"trial": i, "off_s": d, "residual_mV": f"{residual:.1f}", "boot": int(ok),
                        "boot_ms": f"{latency:.0f}" if ok else "", "peak_mA": f"{peak:.2f}",
                        "early_mA": f"{early:.2f}", "late_mA": f"{late:.2f}",
                        "result": "clean" if ok else "stuck"})
            fcsv.flush()
            rig.port.write_note(f"# trial {i}: off={d}s residual={residual:.0f}mV "
                                f"boot={'yes' if ok else 'NO'} late={late:.1f}mA")
            t_boot = t_on if ok else ensure_clean_boot(rig, chip, f"recover-{i}", w)
    rig.off()
    rig.port.write_note("# sweep done; relay left OFF")
    print(f"# wrote {base}_sweep.csv", flush=True)


def run_t01(args):
    rig, chip = start_session(args)
    stamp_ = f"{dt.datetime.now():%Y%m%d_%H%M%S}"
    for n in range(1, args.runs + 1):
        base = os.path.join(LOGS, f"{stamp_}_{args.tag}_run{n}")
        open_logs(rig, chip, base)
        t_on = ensure_clean_boot(rig, chip, f"run{n}")
        t_end = t_on + 720
        while time.monotonic() < t_end:
            if any(x.strip() == "DONE" for _, x in chip.since(t_on)):
                break
            time.sleep(1)
        else:
            rig.port.write_note(f"# run{n}: no DONE within 720 s")
        rig.off()
        rig.port.write_note(f"# run{n} finished; relay OFF")
        print(f"# run{n}: {base}_ina.log / _chip.log", flush=True)
    rig.port.write_note("# all runs done; relay left OFF")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("mode", choices=("sweep", "t01"))
    ap.add_argument("--ina", default="/dev/ttyUSB0")
    ap.add_argument("--chip", default="/dev/ttyUSB1")
    ap.add_argument("--channel", type=int, default=1, choices=(1, 2, 3))
    ap.add_argument("--tag", default=None)
    ap.add_argument("--durations", default="1,2,3,5,8,12,20")
    ap.add_argument("--reps", type=int, default=4)
    ap.add_argument("--cut-after", type=float, default=15.0)
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--runs", type=int, default=2)
    args = ap.parse_args()
    args.tag = args.tag or args.mode
    try:
        (run_sweep if args.mode == "sweep" else run_t01)(args)
    except KeyboardInterrupt:
        print("\n# interrupted", flush=True)
        try:
            serial.Serial(args.ina, 115200, timeout=0.5).write(b"0\n")
        except Exception:
            pass


if __name__ == "__main__":
    main()
