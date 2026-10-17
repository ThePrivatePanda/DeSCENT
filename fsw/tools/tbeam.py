#!/usr/bin/env python3
"""Sets a T-Beam receiver's radio settings, and flashes its firmware if asked.

    ./tbeam.py                          find the receivers and print what each is set to
    ./tbeam.py --sf 12                  set the spreading factor on the one receiver found
    ./tbeam.py --sf 9 --port /dev/ttyUSB1
    ./tbeam.py --freq 914.0 --sf 12     several at once
    ./tbeam.py --flash ../../ground/receiver/firmware/descent-raw-receiver-1.2.1.bin
    ./tbeam.py --name 01                set the name this receiver answers by, kept on the board

The settings live in the T-Beam's own flash and survive a power cycle, so this is a one-off per
receiver rather than something to do before every session. Nothing is stored until the first set,
so a receiver nobody has configured is running the firmware's compiled defaults.

They do NOT survive a flash. The images in firmware/ are whole-flash images and the span they cover
includes the settings partition, so writing one wipes it: a flashed receiver comes back on the
compiled spreading factor with no name, whatever it was set to before. --flash therefore reads the
settings off the board first and puts them back afterwards, and says what it restored. Anything
given on the same command line wins over what was read.

Close the ground software first: it holds the port while it is running.
"""

import argparse
import glob
import os
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
VENV = HERE / ".venv"

KEYS = ("sf", "freq", "bw", "cr", "sync", "pre", "name")


def bootstrap():
    python = VENV / "bin" / "python"
    if not python.exists():
        import venv as venvmod

        print("first run, making tools/.venv", flush=True)
        venvmod.EnvBuilder(with_pip=True).create(VENV)
        subprocess.check_call([str(python), "-m", "pip", "install", "-q", "pyserial"])
    os.execv(str(python), [str(python), str(Path(__file__).resolve())] + sys.argv[1:])


def talk(port, lines, waitS=4.0, settleS=2.0):
    """Sends lines and returns whatever comes back. Opening the port resets an ESP32, so it needs a
    moment before it is listening"""
    import serial

    with serial.Serial(port, 115200, timeout=1) as link:
        time.sleep(settleS)
        link.reset_input_buffer()
        for line in lines:
            link.write((line + "\n").encode())
        out = []
        deadline = time.monotonic() + waitS
        while time.monotonic() < deadline:
            got = link.readline().decode("ascii", "replace").strip()
            if got:
                out.append(got)
                if got.startswith("#DG,RX") or got.startswith("#DG,ERR"):
                    break
        return out


HEADER_TO_KEY = {"f": "freq", "bw": "bw", "sf": "sf", "cr": "cr", "sync": "sync", "pre": "pre",
                 "name": "name"}


def read_settings(port):
    """What a receiver is set to, from its own header. Returns {} if nothing answered - an older
    firmware has no name= and a very old one may have neither, so only what is there is returned"""
    for line in talk(port, ["#GET"], waitS=3.0):
        if not line.startswith("#DG,RX"):
            continue
        found = {}
        for field in line.split(",")[3:]:
            key, _, value = field.partition("=")
            if key in HEADER_TO_KEY and value != "":
                found[HEADER_TO_KEY[key]] = value
        return found
    return {}


def receivers(given):
    """The T-Beams, found by asking rather than by device name: a receiver answers #GET with its
    header line. A ChipSat on the same kind of port does not"""
    if given:
        return [given]
    found = []
    for name in sorted(glob.glob("/dev/ttyUSB*")) + sorted(glob.glob("/dev/ttyACM*")):
        try:
            if any(l.startswith("#DG,RX") for l in talk(name, ["#GET"], waitS=3.0)):
                found.append(name)
        except Exception:
            continue
    return found


def flash_address(path):
    """Where this .bin has to be written. Two kinds are in the firmware folder and they go to
    different places, which is not something you can see by looking at the filename: arduino-cli
    produces the application on its own, starting with the image magic 0xE9, and that goes at
    0x10000 on top of the bootloader already on the board. The 1.0.0 and 1.1.0 files are whole-flash
    images instead - 0x1000 of padding, then the bootloader, then the application at 0x10000 - and
    those go at 0x0. Writing one at the other's address leaves a board that does not boot, so the
    file is asked rather than assumed"""
    with open(path, "rb") as f:
        head = f.read(0x1001)
    if head[:1] == b"\xE9":
        return "0x10000"
    if head[:0x1000] == b"\xFF" * 0x1000 and head[0x1000:0x1001] == b"\xE9":
        return "0x0"
    sys.exit(f"{path} does not look like an ESP32 image: it starts with "
             f"{head[:4].hex()} and has nothing at 0x1000 either")


def main():
    before = {}
    parser = argparse.ArgumentParser(description="T-Beam receiver settings")
    parser.add_argument("--port", help="which receiver, if more than one is plugged in")
    parser.add_argument("--flash", help="an ESP32 firmware .bin to write first")
    for key in KEYS:
        parser.add_argument(f"--{key}", help="the name this receiver answers by, up to 24 characters"
                            if key == "name" else f"set {key}")
    args = parser.parse_args()

    if args.flash:
        if not args.port:
            found = receivers(None)
            if len(found) != 1:
                sys.exit(f"say which port to flash, found {found or 'none'}")
            args.port = found[0]
        if not Path(args.flash).exists():
            sys.exit(f"no such firmware: {args.flash}")
        try:
            import esptool  # noqa: F401
        except ImportError:
            print("installing esptool into tools/.venv", flush=True)
            subprocess.check_call([sys.executable, "-m", "pip", "install", "-q", "esptool"])
        # read before writing: the write erases the settings partition, so this is the only chance
        before = read_settings(args.port)
        if before:
            print("settings on the board now: "
                  + ", ".join(f"{k}={v}" for k, v in before.items()), flush=True)
        else:
            print("the board did not say what it is set to, so nothing can be put back", flush=True)
        address = flash_address(args.flash)
        print(f"flashing {args.flash} to {args.port} at {address}", flush=True)
        code = subprocess.call([sys.executable, "-m", "esptool", "--chip", "esp32", "--port",
                                args.port, "--baud", "921600", "write_flash", address, args.flash])
        if code != 0:
            sys.exit("flashing failed")
        time.sleep(3)

    asked = {key: getattr(args, key) for key in KEYS if getattr(args, key) is not None}
    # what was on the board goes back, except where this command line says otherwise
    restored = {k: v for k, v in before.items() if k not in asked}
    if restored:
        print("putting back: " + ", ".join(f"{k}={v}" for k, v in restored.items()), flush=True)
    wanted = [(key, (asked | restored)[key]) for key in KEYS if key in (asked | restored)]

    ports = receivers(args.port)
    if not ports:
        sys.exit("no receiver answered. Is one plugged in, and is the ground software closed?")

    if wanted and len(ports) > 1:
        sys.exit(f"more than one receiver: {ports}. Say which with --port")

    for port in ports:
        commands = [f"#SET,{key},{value}" for key, value in wanted] or ["#GET"]
        for line in talk(port, commands):
            print(f"{port}  {line}")


if __name__ == "__main__":
    try:
        import serial  # noqa: F401
    except ImportError:
        bootstrap()
    sys.exit(main())
