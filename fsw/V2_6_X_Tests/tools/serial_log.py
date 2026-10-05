#!/usr/bin/env python3
"""Timestamped serial logger for the ChipSat debug UART (PB6/PB7, 115200).

usage: tools/serial_log.py [PORT] [--baud 115200] [--tag NAME]
  PORT defaults to the first /dev/ttyUSB* or /dev/ttyACM*.
Echoes every line to the terminal and writes logs/YYYYmmdd_HHMMSS_<tag>.log with an ISO
timestamp prefix per line. The raw byte stream (no timestamps) is also saved as .raw so the
packet decoder (tools/descent_decode.py) can parse hex dumps unchanged. Ctrl-C to stop.
Anything typed in the terminal + Enter is sent to the port with a newline (e.g. "1" for the
lab VI_Relay rig) and logged as "# sent: ..." with its timestamp.
"""
import argparse, datetime as dt, glob, os, sys, threading, time
try:
    import serial
except ImportError:
    sys.exit("pyserial missing: pip install --user pyserial")

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOGS = os.path.join(ROOT, "logs")

def default_port():
    ports = sorted(glob.glob("/dev/ttyUSB*") + glob.glob("/dev/ttyACM*"))
    return ports[0] if ports else None

def forward_stdin(ser, emit):
    """Send each typed line to the device (runs in a background thread)."""
    for line in sys.stdin:
        cmd = line.rstrip("\r\n")
        ser.write((cmd + "\n").encode())
        emit(f"# sent: {cmd}")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("port", nargs="?", default=default_port())
    ap.add_argument("--baud", type=int, default=115200)
    ap.add_argument("--tag", default="serial")
    a = ap.parse_args()
    if not a.port:
        sys.exit("no serial port found (plug in the USB-UART) or pass one explicitly")
    os.makedirs(LOGS, exist_ok=True)
    stamp = dt.datetime.now().strftime("%Y%m%d_%H%M%S")
    base = os.path.join(LOGS, f"{stamp}_{a.tag}")
    print(f"# port={a.port} baud={a.baud} -> {base}.log / .raw   (Ctrl-C to stop)", flush=True)
    with serial.Serial(a.port, a.baud, timeout=0.2) as ser, \
         open(base + ".log", "a", buffering=1) as log, open(base + ".raw", "ab") as raw:
        log.write(f"# {dt.datetime.now().isoformat()} port={a.port} baud={a.baud}\n")
        lock = threading.Lock()
        def emit(text):
            ts = dt.datetime.now().strftime("%H:%M:%S.%f")[:-3]
            with lock:
                print(f"{ts}  {text}", flush=True)
                log.write(f"{ts}  {text}\n")
        threading.Thread(target=forward_stdin, args=(ser, emit), daemon=True).start()
        buf = b""
        try:
            while True:
                chunk = ser.read(4096)
                if not chunk:
                    continue
                raw.write(chunk); raw.flush()
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    emit(line.decode("utf-8", "replace").rstrip("\r"))
        except KeyboardInterrupt:
            print(f"\n# stopped; {base}.log", flush=True)

if __name__ == "__main__":
    main()
