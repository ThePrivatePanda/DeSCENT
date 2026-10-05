#!/usr/bin/env python3
"""Pulls the flash log off a ChipSat: capture, csv, a log for the ground software, and a replay.

    ./flash_replay.py --port /dev/ttyUSB0        the whole round trip, one command
    ./flash_replay.py --port /dev/ttyUSB0 --no-flash --no-restore   leave the board alone
    ./flash_replay.py --port /dev/ttyUSB0 --no-browser      write the files and open nothing
    ./flash_replay.py --file dump.txt             the same from a capture taken any other way
    ./flash_replay.py --port /dev/ttyUSB0 --start 2026-09-23T20:05Z    when the log has no gps time

One pull leaves dump.txt, records.csv and ground.log together in a dated folder under data/, then
opens it in the ground software with the log already loaded. A page can't read a file off your disk,
so the log is served from a local port and the page fetches it. That only works with the ground app,
which is http like the server: the hosted site is https and Chrome blocks it from fetching http on
this machine, so it opens empty and the log has to be picked by hand.

With --port it puts bench/flash_dump on the board first and the flight software back afterwards,
both through build.sh: --no-flash and --no-restore skip either half. --plot brings back the old
matplotlib window, which needs matplotlib, installed into tools/.venv on the first run.
"""

import argparse
import os
import re
import struct
from datetime import datetime, timezone
import subprocess
import sys
import venv
from pathlib import Path

HERE = Path(__file__).resolve().parent
VENV = HERE / ".venv"
DATA = HERE.parent / "data"   # captures live with the project, not next to the tools


def in_venv_with_matplotlib():
    try:
        import matplotlib  # noqa: F401
        import serial  # noqa: F401
        return True
    except ImportError:
        return False


def bootstrap():
    """First run: build tools/.venv with what we need, then start again inside it."""
    python = VENV / "bin" / "python"
    if not python.exists():
        print("first run, making tools/.venv", flush=True)
        venv.EnvBuilder(with_pip=True).create(VENV)
        subprocess.check_call([str(python), "-m", "pip", "install", "-q", "matplotlib", "pyserial"])
    os.execv(str(python), [str(python), str(Path(__file__).resolve())] + sys.argv[1:])


# ----------------------------------------------------------------------------------------------
# the packet, as Telemetry.h lays it out

PACKET = struct.Struct("<iiii3h3h3h4hhHHHBBBH")
PACKET_BYTES = 55


def crc16(data):
    crc = 0xFFFF
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc


def decode(packet):
    if len(packet) != PACKET_BYTES:
        return None
    fields = PACKET.unpack(packet)
    (lat, lon, gps_alt_mm, env_alt_cm,
     ax, ay, az, gx, gy, gz, mx, my, mz,
     qi, qj, qk, qr, temp, pressure, humidity,
     counter, csid, cell, valid, crc) = fields
    return {
        "counter": counter, "csid": csid, "validity": valid,
        "crc_ok": crc16(packet[:53]) == crc,
        "lat": lat / 1e7, "lon": lon / 1e7,
        "gps_alt_m": gps_alt_mm / 1000.0, "baro_alt_m": env_alt_cm / 100.0,
        "ax": ax / 100.0, "ay": ay / 100.0, "az": az / 100.0,
        "gx": gx / 10.0, "gy": gy / 10.0, "gz": gz / 10.0,
        "mx": mx / 10.0, "my": my / 10.0, "mz": mz / 10.0,
        "qi": qi / 32767.0, "qj": qj / 32767.0, "qk": qk / 32767.0, "qr": qr / 32767.0,
        "temp_c": temp / 100.0, "pressure_hpa": pressure / 10.0, "humidity": humidity / 100.0,
        "cell_pct": cell / 2.0,
        "accel_valid": bool(valid & 0x01), "gyro_valid": bool(valid & 0x02),
        "mag_valid": bool(valid & 0x04), "quat_valid": bool(valid & 0x08),
        "gps_valid": bool(valid & 0x10), "soc_valid": bool(valid & 0x20),
        "env_valid": bool(valid & 0x40), "all_fresh": bool(valid & 0x80),
    }


# ----------------------------------------------------------------------------------------------
# reading the dump

RECORD_BYTES = 64
RECORD_CRC_AT = 62


def parse_raw_record(index, address, raw):
    """One 64 byte slot as it sits on the chip. Bytes 62-63 are the record's own CRC16 over 0-61, so
    the boot count and uptime are checked by the number the flight software wrote - a digit lost on
    the way up the wire is caught here instead of reading as a real timestamp"""
    if len(raw) != RECORD_BYTES:
        return None
    want = raw[RECORD_CRC_AT] | raw[RECORD_CRC_AT + 1] << 8
    if crc16(raw[:RECORD_CRC_AT]) != want:
        return {"index": index, "address": address, "damaged": True}
    kind = raw[0]
    boot = raw[1] | raw[2] << 8
    ms = int.from_bytes(raw[3:7], "little")
    if kind == 0xA6:                                    # a time anchor
        return {"index": index, "address": address, "anchor": True, "boot": boot, "ms": ms,
                "epoch": int.from_bytes(raw[7:11], "little"),
                "nano": int.from_bytes(raw[11:15], "little", signed=True),
                "bits": raw[15], "acc_ns": int.from_bytes(raw[16:20], "little")}
    if kind != 0xA5:
        return {"index": index, "address": address, "unknown": kind}
    fields = decode(raw[7:7 + PACKET_BYTES])
    if fields is None:
        return {"index": index, "address": address, "damaged": True}
    fields.update({"index": index, "address": address, "boot": boot, "ms": ms,
                   "hex": raw[7:7 + PACKET_BYTES].hex().upper()})
    return fields


def parse_dump(lines):
    """Reads either format. New: 'raw <index> <addr> <128 hex>', the whole checked record. Old:
    'record N boot B ms T' then 55 hex bytes, with the header outside any check, plus a 'time' line
    per anchor. Captures taken before 2026-10-04 are the old one."""
    records = []
    anchors = []
    pending = None
    # One dump, one format. A capture can begin with the tail of an earlier dump still draining out
    # of a buffer - CS9's first pull had four readable-format records and a boot log ahead of 31526
    # raw ones - and reading both leaves records from two different dumps in one list. The raw format
    # carries a CRC over the whole slot, so where it is present it is the only thing worth reading
    rawFormat = any(line.startswith("raw ") for line in lines)
    damaged = 0
    unknown = {}        # intact records of a type this tool does not know, counted per type
    for line in lines:
        line = line.strip()
        if line.startswith("raw "):
            parts = line.split()
            if len(parts) != 4:
                damaged += 1   # a raw line that lost its shape is still a record that did not arrive
                continue
            try:
                got = parse_raw_record(int(parts[1]), int(parts[2], 16), bytes.fromhex(parts[3]))
            except ValueError:
                damaged += 1
                continue
            kind = got.get("unknown") if got else None
            if kind is not None:
                # its CRC was good, so the record arrived whole - this tool is just older than the
                # firmware that wrote it. Saying "could not be read" about it would blame the wire
                unknown[kind] = unknown.get(kind, 0) + 1
                continue
            if got is None or got.get("damaged"):
                damaged += 1
                continue
            if got.pop("anchor", False):
                anchors.append(got)
            else:
                records.append(got)
            continue
        if rawFormat:
            continue    # anything that is not a raw line belongs to an older dump or is log noise
        if line.startswith("time ") and " epoch " in line:
            parts = line.split()
            anchors.append({"boot": int(parts[3]), "ms": int(parts[5]), "epoch": int(parts[7]),
                            "nano": int(parts[9]), "bits": int(parts[11], 16),
                            "acc_ns": int(parts[13])})
            continue
        if line.startswith("record ") and " boot " in line:
            parts = line.split()
            pending = {"index": int(parts[1]), "boot": int(parts[3]), "ms": int(parts[5])}
            continue
        if pending is not None and line and all(c in "0123456789ABCDEFabcdef " for c in line):
            raw = bytes.fromhex(line.replace(" ", ""))
            if len(raw) == PACKET_BYTES:
                fields = decode(raw)
                if fields:
                    fields.update(pending)
                    fields["hex"] = raw.hex().upper()   # flash_to_ground.py hands this to the ground software
                    records.append(fields)
            pending = None
    if damaged:
        print(f"{damaged} records could not be read and were left out: a bad slot CRC, or a line "
              f"that lost its shape on the way here", flush=True)
    for kind, count in sorted(unknown.items()):
        print(f"{count} records are type 0x{kind:02X}, which this tool does not know. They arrived "
              f"intact and are on the chip; a newer version of this tool will read them", flush=True)
    spread = {}
    add_utc(records, anchors, spread)
    for boot, seconds in sorted(spread.items()):
        print(f"boot {boot}: its anchors disagree by {seconds:.1f} s, so these dates are rough. The "
              f"ground software dates a drifting boot better", flush=True)
    return records, anchors


def add_utc(records, anchors, spread=None):
    """uptime is a plain millisecond counter that never stops, so an anchor from the same boot dates
    every record of that boot, the ones written before the GPS had time included."""
    spread = {} if spread is None else spread
    for record in records:
        same_boot = [a for a in anchors if a["boot"] == record["boot"]]
        if not same_boot:
            record["utc"] = None
            record["utc_iso"] = ""
            continue
        # the median offset of every anchor in the boot, not the nearest one: a single damaged or
        # outlying anchor then moves nothing. The ground software interpolates between neighbouring
        # anchors instead, so its dates are the better ones when a boot's clock has drifted
        offsets = sorted(a["epoch"] + a["nano"] / 1e9 - a["ms"] / 1000.0 for a in same_boot)
        middle = offsets[len(offsets) // 2]
        if len(offsets) > 1 and offsets[-1] - offsets[0] > 2.0:
            spread[record["boot"]] = offsets[-1] - offsets[0]
        seconds = middle + record["ms"] / 1000.0
        record["utc"] = seconds
        record["utc_iso"] = datetime.fromtimestamp(seconds, tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]


CONSOLE = re.compile(r"^[EWID] [A-Za-z]+ \d+ c\d+:")   # a log line: "W GPS 166787 c49: read ok=0 ..."


def find_port(tryFor=4.0):
    """The console port, by listening rather than by guessing a name. The ChipSat is whichever port
    talks like one: the flight software's log lines, or flash_dump's own output. Which device it turns
    up as depends on the programmer - an ST-Link V3 brings its own ttyACM, a pogo lead is a ttyUSB -
    and a T-Beam looks exactly the same to the naming scheme"""
    import glob
    import serial
    import time

    for name in sorted(glob.glob("/dev/ttyACM*")) + sorted(glob.glob("/dev/ttyUSB*")):
        try:
            with serial.Serial(name, 115200, timeout=1) as link:
                deadline = time.monotonic() + tryFor
                while time.monotonic() < deadline:
                    line = link.readline().decode("ascii", "replace").strip()
                    if CONSOLE.match(line) or looks_like_dump(line):
                        print(f"found the board on {name}", flush=True)
                        return name
        except Exception:
            continue
    return None


def looks_like_dump(line):
    """A line flash_dump prints: a record header, an anchor, a 55 byte packet in hex, or the summary"""
    line = line.strip()
    if line.startswith(("record ", "time ", "found ", "send D")):
        return True
    return len(line) > 20 and all(c in "0123456789ABCDEFabcdef " for c in line)


def last_dump(lines):
    """The last complete dump in a capture, or None.

    A capture can hold more than one: the board prints its whole log when it boots and again on D, so
    what comes off the port is often the tail of one dump, then a boot, then another. Rather than try
    to decide that live, read everything and pick afterwards. A dump runs from its first record, or
    from the boot line when the chip is empty, to its "found" summary"""
    best = None
    start = None
    booted = False
    for i, line in enumerate(lines):
        text = line.strip()
        if "test=flashdump" in text:
            start = None       # a restart, so anything open belongs to the board's last life
            booted = True
        elif text.startswith("record 0 ") or text.startswith("time 0 "):
            start = i
        elif text.startswith("found "):
            if start is not None:
                best = lines[start:i + 1]
            elif booted:
                best = [line]  # an empty chip prints nothing but its summary
            start = None
            booted = False
    return best


def read_from_board(port, baud, save, trigger=None, waitMs=20000, capMs=1800000):
    """Everything the board says, until a dump has finished. The dump to keep is picked afterwards.

    The board has to be restarted for it to print, and the restart has to happen while this is already
    listening: the port's buffer is small, and D needs the adapter's TX wired to the console, which is
    not something to depend on. So the flash or reset runs on a thread and this just reads"""
    import serial
    import threading
    import time

    print(f"reading {port}, this takes a while for a full chip", flush=True)
    lines = []
    last = ""
    quiet = 0
    done = False          # a "found" has gone by, so the next silence is the end
    with serial.Serial(port, baud, timeout=2) as link:
        link.reset_input_buffer()
        working = None
        if trigger is None:
            link.write(b"D")   # no probe to reset it with, so ask the old way
        else:
            working = threading.Thread(target=trigger, daemon=True)
            working.start()

        giveUp = time.monotonic() + waitMs / 1000.0
        limit = time.monotonic() + capMs / 1000.0
        while quiet < 3 and time.monotonic() < limit:
            line = link.readline().decode("ascii", "replace")
            if not line:
                # silence means nothing until the flashing is over and the board has said something
                if (working is not None and working.is_alive()) or not lines:
                    if time.monotonic() > giveUp and not (working is not None and working.is_alive()):
                        break
                    continue
                quiet += 1
                continue
            quiet = 0
            lines.append(line)
            text = line.strip()
            last = text or last
            if text.startswith("found "):
                print(text, flush=True)
                done = True
                quiet = 2      # one more silent read and we stop
            elif not done:
                giveUp = time.monotonic() + waitMs / 1000.0

    dump = last_dump(lines)
    if save:
        # a capture with no complete dump in it is kept under another name, so it can't be mistaken
        # later for a good one
        where = Path(save) if dump else Path(str(save) + ".partial")
        where.write_text("".join(lines), encoding="utf-8", newline="")
        print(f"{'capture saved to' if dump else 'no complete dump, kept'} {where}", flush=True)
    if dump is None and lines:
        print(f"no dump finished. The last line was: {last[:80]}", flush=True)
        print("if that isn't flash_dump talking, the port may belong to something else, the T-Beam "
              "say, or the board may be running the flight software", flush=True)
    return dump or []


GROUND_SITE = "https://ground.privatepanda.co"
GROUND_PORTS = range(8765, 8775)   # it takes the first free one, so don't assume 8765


def find_ground_web(given):
    """A checked out copy of the ground software's web/ folder, for --local. It's a separate repo, so
    this only looks for it and never writes to it"""
    if given:
        return Path(given)
    # V2_6_X/data -> V2_6_X -> Software -> SSDS_DeSCENT -> the folder holding both repos
    for up in range(1, 6):
        candidate = DATA.parents[up] / "descent-ground" / "web"
        if (candidate / "index.html").exists():
            return candidate
    return None


def run_build(*arguments):
    """build.sh, from the folder it lives in. Windows gets build.cmd"""
    root = DATA.parent
    script = root / ("build.cmd" if os.name == "nt" else "build.sh")
    print(f"{script.name} {' '.join(arguments)}", flush=True)
    return subprocess.call([str(script), *arguments], cwd=str(root)) == 0


def find_ground_app(fetch=True):
    """The ground app build.sh puts in deps/. Downloads it the first time, since a log you can't open
    isn't much use. The asset is named for its platform, so take whatever is in the newest folder"""
    deps = DATA.parent / "deps"
    for attempt in (0, 1):
        for folder in sorted(deps.glob("descent-ground-*"), reverse=True):
            for exe in sorted(folder.glob("descent-ground-*")):
                # it writes its own log and a url file next to itself, both named the same way
                if exe.is_file() and exe.suffix not in (".log", ".url") and os.access(exe, os.X_OK):
                    return exe
        if attempt or not fetch or not run_build("ground"):
            return None
    return None


def start_ground_app(exe, waitMs=15000):
    """Starts it and waits for it to answer. It keeps running after this tool exits, which is what you
    want when you're pulling one chip after another"""
    import subprocess
    import time

    import re

    print(f"starting the ground software, {exe.name}", flush=True)
    try:
        # --no-browser because this opens the page itself, with the log already in it. It prints
        # where it ended up, which is the one answer that covers "started" and "already running"
        started = subprocess.Popen([str(exe), "--no-browser"], stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True)
    except OSError as problem:
        print(f"couldn't start it: {problem}", flush=True)
        return None

    said = started.stdout.readline()
    found = re.search(r"http://[0-9.]+:\d+", said or "")
    print((said or "").strip(), flush=True)

    deadline = time.monotonic() + waitMs / 1000.0
    while time.monotonic() < deadline:
        where = app_running(exe) or (found.group(0) if found and answers(found.group(0)) else None)
        if where:
            return where
        time.sleep(0.25)
    print(f"it didn't answer within {waitMs // 1000} s", flush=True)
    return None


def answers(url):
    """Anything at all on that address, health endpoint or not. A copy older than 0.4.0 has no
    /api/health, and it can still show a log"""
    import urllib.request

    try:
        urllib.request.urlopen(url, timeout=1)
        return True
    except Exception as problem:
        return getattr(problem, "code", None) is not None


def app_running(exe=None):
    """Where the ground app is, or None. It serves the dashboard over plain http, which is the whole
    reason it's the page we want: the hosted site is https and Chrome won't let it fetch a file off
    http on this machine (tested - it's mixed content, the request never leaves the browser).

    It writes descent-ground.url beside itself while it runs. Failing that it takes the first free
    port from 8765 up, so the range gets probed rather than assuming 8765 is the one"""
    import urllib.error
    import urllib.request

    def healthy(url):
        try:
            with urllib.request.urlopen(f"{url.rstrip('/')}/api/health", timeout=1) as answer:
                return b'"ok"' in answer.read(200)
        except Exception:
            return False

    # Ctrl-C leaves the url file behind, so it's a hint about where to look and never proof that
    # anything is running. /api/health is the only thing that settles it
    if exe:
        written = Path(exe).parent / "descent-ground.url"
        if written.exists():
            url = written.read_text().strip()
            if url and healthy(url):
                return url.rstrip("/")

    for port in GROUND_PORTS:
        url = f"http://127.0.0.1:{port}"
        if healthy(url):
            return url
    return None


def open_in_ground(log, web, local):
    """Opens the log in the ground software, best page first: the ground app if it's running, then a
    copy of the page next door, then the hosted site. Only the first two can be handed the log without
    anyone clicking, because a page can't read a file off your disk and the hosted site is https,
    which Chrome won't let fetch from http on this machine"""
    import functools
    import http.server
    import socketserver
    import urllib.parse
    import webbrowser

    log = Path(log).resolve()
    exe = find_ground_app()
    page = app_running(exe)
    if page is None and exe and not local:
        page = start_ground_app(exe)
    if page is None and local and web is not None:
        page = "self"     # served from the same port as the log, below
    elif page is None and local:
        print("no copy of the ground software next to this repo", flush=True)

    class Handler(http.server.SimpleHTTPRequestHandler):
        def translate_path(self, path):
            if path.split("?")[0] == "/log.log":
                return str(log)
            return super().translate_path(path)

        def end_headers(self):
            # the hosted page is a different origin, and a private network by Chrome's reckoning
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Private-Network", "true")
            super().end_headers()

        def log_message(self, *args):
            pass

    serve_from = str(web) if page == "self" else str(log.parent)
    handler = functools.partial(Handler, directory=serve_from)
    with socketserver.TCPServer(("127.0.0.1", 0), handler) as server:
        port = server.server_address[1]
        served = f"http://127.0.0.1:{port}/log.log"
        # the page reads log= with URLSearchParams, so an & in the inner url would cut it short
        if page == "self":
            url = f"http://127.0.0.1:{port}/?log={urllib.parse.quote('/log.log', safe='')}"
        elif page:
            url = f"{page}/?log={urllib.parse.quote(served, safe='')}"
        else:
            # the hosted site can't be handed the file, so it opens empty and waits for Open log
            url = GROUND_SITE
            print("the ground app isn't running, so the hosted site can't be given the log by itself.",
                  flush=True)
            print(f"start the app and run this again, or use Open log on {log}", flush=True)
        print(f"opening {url}", flush=True)
        webbrowser.open(url)
        if not page:
            return
        print("leave this running while you use it, Ctrl-C when you're done", flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            print("", flush=True)


def write_csv(records, path):
    import csv

    if not records:
        return
    with open(path, "w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(records[0].keys()))
        writer.writeheader()
        writer.writerows(records)
    print(f"{len(records)} records written to {path}", flush=True)


# ----------------------------------------------------------------------------------------------
# the replay window

class Replay:
    SPEEDS = [1, 2, 4, 10, 50]

    def __init__(self, records):
        import matplotlib.pyplot as plt
        from matplotlib.widgets import Button, Slider

        self.records = records
        self.t = [(r["ms"] - records[0]["ms"]) / 1000.0 for r in records]
        self.at = 0
        self.playing = False
        self.speed = 1

        self.figure, axes = plt.subplots(4, 1, sharex=True, figsize=(11, 9))
        self.figure.canvas.manager.set_window_title("ChipSat flash log")
        self.figure.subplots_adjust(bottom=0.17, top=0.96, hspace=0.25)
        altitude, motion, spin, house = axes

        baro = [r["baro_alt_m"] if r["env_valid"] else float("nan") for r in records]
        gps = [r["gps_alt_m"] if r["gps_valid"] else float("nan") for r in records]
        altitude.plot(self.t, baro, label="barometric")
        altitude.plot(self.t, gps, label="gps")
        altitude.set_ylabel("altitude (m)")
        altitude.legend(loc="upper right", fontsize=8)

        accel = [(r["ax"] ** 2 + r["ay"] ** 2 + r["az"] ** 2) ** 0.5 if r["accel_valid"] else float("nan")
                 for r in records]
        motion.plot(self.t, accel, color="tab:red")
        motion.set_ylabel("acceleration (m/s²)")

        for axis, colour in (("gx", "tab:blue"), ("gy", "tab:orange"), ("gz", "tab:green")):
            spin.plot(self.t, [r[axis] if r["gyro_valid"] else float("nan") for r in records],
                      color=colour, label=axis)
        spin.set_ylabel("rotation (°/s)")
        spin.legend(loc="upper right", fontsize=8, ncol=3)

        house.plot(self.t, [r["temp_c"] if r["env_valid"] else float("nan") for r in records],
                   color="tab:purple", label="temperature (°C)")
        house.set_ylabel("temperature (°C)")
        house.set_xlabel("seconds since the log started")
        battery = house.twinx()   # percent would flatten the temperature on one axis
        battery.plot(self.t, [r["cell_pct"] if r["soc_valid"] else float("nan") for r in records],
                     color="tab:brown", label="battery (%)")
        battery.set_ylabel("battery (%)")
        battery.set_ylim(0, 105)
        lines = house.get_lines() + battery.get_lines()
        house.legend(lines, [line.get_label() for line in lines], loc="upper right", fontsize=8)

        self.cursors = [axis.axvline(self.t[0], color="k", lw=1) for axis in axes]
        self.readout = self.figure.text(0.01, 0.008, "", family="monospace", fontsize=8)

        def button(left, label, action):
            axis = self.figure.add_axes([left, 0.045, 0.085, 0.04])
            widget = Button(axis, label)
            widget.on_clicked(action)
            return widget

        self.play_button = button(0.01, "play", self.toggle)
        self.buttons = [
            self.play_button,
            button(0.105, "|<<", lambda _: self.seek(0)),
            button(0.2, "<", lambda _: self.step(-1)),
            button(0.295, ">", lambda _: self.step(1)),
            button(0.39, ">>|", lambda _: self.seek(len(records) - 1)),
        ]
        for i, speed in enumerate(self.SPEEDS):
            self.buttons.append(button(0.52 + i * 0.09, f"{speed}x", self.speed_setter(speed)))

        slider_axis = self.figure.add_axes([0.06, 0.105, 0.88, 0.02])
        self.slider = Slider(slider_axis, "", 0, len(records) - 1, valinit=0, valstep=1)
        self.slider.on_changed(lambda value: self.seek(int(value), from_slider=True))
        self.slider.valtext.set_visible(False)

        self.figure.canvas.mpl_connect("key_press_event", self.key)
        self.timer = self.figure.canvas.new_timer(interval=50)
        self.timer.add_callback(self.tick)
        self.timer.start()
        self.show(0)
        plt.show()

    # -- controls

    def speed_setter(self, speed):
        def set_speed(_):
            self.speed = speed
        return set_speed

    def toggle(self, _=None):
        self.playing = not self.playing
        if self.playing and self.at >= len(self.records) - 1:
            self.at = 0
        self.play_button.label.set_text("pause" if self.playing else "play")
        self.figure.canvas.draw_idle()

    def step(self, by):
        self.playing = False
        self.play_button.label.set_text("play")
        self.seek(self.at + by)

    def seek(self, to, from_slider=False):
        self.at = max(0, min(len(self.records) - 1, to))
        if not from_slider:
            self.slider.set_val(self.at)
        self.show(self.at)

    def key(self, event):
        if event.key == " ":
            self.toggle()
        elif event.key == "left":
            self.step(-1)
        elif event.key == "right":
            self.step(1)
        elif event.key == "home":
            self.seek(0)
        elif event.key == "end":
            self.seek(len(self.records) - 1)

    # -- playback

    def tick(self):
        if not self.playing:
            return
        elapsed = 0.05 * self.speed
        target = self.t[self.at] + elapsed
        while self.at < len(self.records) - 1 and self.t[self.at + 1] <= target:
            self.at += 1
        if self.at >= len(self.records) - 1:
            self.playing = False
            self.play_button.label.set_text("play")
        self.slider.set_val(self.at)

    def show(self, at):
        record = self.records[at]
        for cursor in self.cursors:
            cursor.set_xdata([self.t[at], self.t[at]])
        bits = "".join(name for name, on in (
            ("A", record["accel_valid"]), ("G", record["gyro_valid"]), ("M", record["mag_valid"]),
            ("Q", record["quat_valid"]), ("P", record["gps_valid"]), ("B", record["soc_valid"]),
            ("E", record["env_valid"]), ("F", record["all_fresh"])) if on) or "-"
        stamp = record.get("utc_iso") or "no gps time"
        self.readout.set_text(
            f"record {at + 1}/{len(self.records)}  boot {record['boot']}  t={self.t[at]:8.2f}s  "
            f"{stamp} UTC  "
            f"counter {record['counter']}  csid {record['csid']}  valid {bits}  "
            f"alt {record['baro_alt_m']:7.1f} m  lat {record['lat']:.6f} lon {record['lon']:.6f}  "
            f"{record['temp_c']:5.2f} C  {record['pressure_hpa']:7.2f} hPa  {record['cell_pct']:5.1f} %"
            f"{'' if record['crc_ok'] else '   BAD CRC'}")
        self.figure.canvas.draw_idle()


def main():
    parser = argparse.ArgumentParser(description="pull the ChipSat flash log and replay it")
    parser.add_argument("--port", help="serial port the board is on. Found by listening if left out")
    parser.add_argument("--baud", type=int, default=115200)
    parser.add_argument("--file", help="a capture to read instead of the board")
    parser.add_argument("--save", help="write the raw capture here")
    parser.add_argument("--csv", help="write the decoded records here")
    parser.add_argument("--dir", help="folder for the capture, csv and log (default: V2_6_X/data/<date>)")
    parser.add_argument("--start", help="UTC time of the first record, ISO 8601, when the log has no "
                                        "gps time anchors. Without it they are counted back from now")
    parser.add_argument("--no-log", action="store_true", help="skip the ground software log")
    parser.add_argument("--no-browser", action="store_true", help="just write the files, open nothing")
    parser.add_argument("--local", action="store_true",
                        help="serve a checked out copy of the ground software instead of using "
                             f"{GROUND_SITE}, for working with no internet")
    parser.add_argument("--ground-dir", help="the ground software's web/ folder, if it isn't found")
    parser.add_argument("--plot", action="store_true", help="the old matplotlib window as well")
    parser.add_argument("--no-flash", action="store_true",
                        help="the board already has flash_dump on it, don't build and flash it")
    parser.add_argument("--no-restore", action="store_true",
                        help="leave flash_dump on the board instead of putting the flight software back")
    parser.add_argument("--probe", help="ST-Link serial, when more than one is plugged in")
    args = parser.parse_args()

    if not args.port and not args.file:
        args.port = find_port()
        if not args.port:
            parser.error("couldn't find a board on any serial port. Give --port, or check the lead")

    # everything from one pull lands together, under data/, not wherever the tool was run from
    if args.dir:
        out_dir = Path(args.dir)
    elif args.file:
        out_dir = Path(args.file).resolve().parent
    else:
        out_dir = DATA / datetime.now().strftime("%Y-%m-%d_%H%M")
    out_dir.mkdir(parents=True, exist_ok=True)

    # with --file there's no board in it at all, so neither half applies
    probe = ["--probe", args.probe] if args.probe else []
    flashing = bool(args.port) and not args.no_flash
    restoring = bool(args.port) and not args.no_restore

    # flashing ends in a reset and so does reset, and either way the board starts printing its log
    # from record 0 with the port already open. That is what makes the capture whole
    trouble = []

    def trigger():
        # this runs on its own thread, so a failure is reported back rather than exited on
        if flashing:
            if not run_build("flash", "--sketch", "flash_dump", *probe):
                trouble.append("couldn't put flash_dump on the board")
        elif not run_build("reset", *probe):
            print("couldn't reset the board, asking it for the log instead", flush=True)

    if args.file:
        # the same picking as a live read, so a capture holding two dumps isn't counted twice
        lines = last_dump(Path(args.file).read_text(errors="replace").splitlines(True))
        if lines is None:
            print(f"no complete dump in {args.file}: reading it as it stands", flush=True)
            lines = Path(args.file).read_text(errors="replace").splitlines(True)
        source = args.file
        dumped = datetime.fromtimestamp(Path(args.file).stat().st_mtime, tz=timezone.utc)
    else:
        source = str(args.save or out_dir / "dump.txt")
        lines = read_from_board(args.port, args.baud, source, trigger)
        if trouble:
            sys.exit(trouble[0])
        dumped = datetime.now(timezone.utc)

    records, anchors = parse_dump(lines)
    if not records:
        print("no records found. Is the board running bench/flash_dump, and is the log empty?")
        # the read failing is no reason to leave a dump sketch on a board that has to fly
        if restoring:
            run_build("flash", *probe)
        return 1

    boots = sorted({r["boot"] for r in records})
    bad = sum(1 for r in records if not r["crc_ok"])
    span = (records[-1]["ms"] - records[0]["ms"]) / 1000.0
    timed = sum(1 for r in records if r.get("utc"))
    print(f"{len(records)} records, boots {boots}, {span:.1f} s of log, {bad} with a bad CRC")
    if anchors:
        first = datetime.fromtimestamp(anchors[0]["epoch"], tz=timezone.utc)
        print(f"{len(anchors)} gps time anchors, first at {first:%Y-%m-%d %H:%M:%S} UTC, "
              f"{timed} records dated")
    else:
        print("no gps time anchors in this log, times are uptime only")

    write_csv(records, args.csv or out_dir / "records.csv")

    if restoring:
        if not run_build("flash", *probe):
            print("the flight software did NOT go back on, the board is still running flash_dump",
                  flush=True)
    elif args.port:
        print("the board is running flash_dump now, not the flight software. Put it back with "
              "./build.sh flash", flush=True)

    if not args.no_log:
        import flash_to_ground as ground   # here, not at the top: it imports this file for the decoder

        start = None
        if args.start:
            start = datetime.fromisoformat(args.start.replace("Z", "+00:00"))
            if start.tzinfo is None:
                start = start.replace(tzinfo=timezone.utc)
        dated, dropped, basis = ground.date_records(records, anchors, start, dumped)
        if dropped:
            boots = sorted({r["boot"] for r in dropped})
            print(f"leaving {len(dropped)} records from boot(s) {boots} out of the ground log: the "
                  f"rest has real gps time and these have none. --start places them", flush=True)
        log = out_dir / "ground.log"
        ground.write_log(log, dated, anchors, "flash", source, basis,
                         ground.how_text(basis, args.start, dumped))
        if basis == "none":
            print("no gps time in this log, so it is dated from 1970: the spacing is right, the date "
                  "is not. --start puts it at a real time", flush=True)
        print(f"ground log {log}", flush=True)

    if args.plot:
        Replay(records)
    if not args.no_log and not args.no_browser:
        open_in_ground(log, find_ground_web(args.ground_dir), args.local)
    return 0


if __name__ == "__main__":
    if not in_venv_with_matplotlib():
        bootstrap()
    sys.exit(main())
