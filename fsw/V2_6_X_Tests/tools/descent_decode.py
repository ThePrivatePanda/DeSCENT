#!/usr/bin/env python3
"""Decoder for the Cornell SSDS DeSCENT ChipSat 55-byte telemetry packet.

Dependency-free (standard library only). Source of truth is the flight
firmware: Software/V2_5_X/V2_5_X_Code/Telemetry.h / Telemetry.cpp, and the
T-Beam receiver V2_5_X_T_BEAM_DECODE.ino.

Packet layout (little-endian, packed, 55 bytes, no header):

    bytes  field                 type     scale
    0-3    latitude              int32    /1e7 deg
    4-7    longitude             int32    /1e7 deg
    8-11   GPS altitude MSL      int32    /1000 m
    12-15  pressure altitude     int32    /100 m
    16-21  linear accel X,Y,Z    int16x3  /100 m/s^2
    22-27  gyro X,Y,Z            int16x3  /10 deg/s
    28-33  mag X,Y,Z             int16x3  /10 uT
    34-41  quat i,j,k,real       int16x4  /32767
    42-43  temperature           int16    /100 degC
    44-45  pressure              uint16   /10 hPa
    46-47  humidity              uint16   /100 %
    48-49  packet counter        uint16
    50     CSID                  uint8
    51     state of charge       uint8    /2 %
    52     validity byte         uint8    bit flags
    53-54  CRC-16/CCITT-FALSE    uint16   over bytes 0-52

Usage:
    descent_decode.py --hex "5C 57 4C 19 ..."
    descent_decode.py capture.txt [--csv]
    descent_decode.py --selftest
"""

import argparse
import csv
import re
import struct
import sys

PACKET_LEN = 55
CRC_LEN = 53  # bytes 0-52 are CRC-protected
STRUCT_FMT = "<4i3h3h3h4hhHHHBBBH"

assert struct.calcsize(STRUCT_FMT) == PACKET_LEN

# Validity byte bit assignments (Telemetry.h).
VALIDITY_FLAGS = (
    ("valid_linacc", 0),
    ("valid_gyro", 1),
    ("valid_mag", 2),
    ("valid_quat", 3),
    ("valid_gnss", 4),
    ("valid_soc", 5),
    ("valid_env", 6),
    ("all_fresh", 7),
)
VALIDITY_FLAGS = dict(VALIDITY_FLAGS)

# Test vector from the software report, section 8.
SELFTEST_HEX = (
    "5C 57 4C 19 50 BD 66 D2 90 D0 03 00 B4 5F 00 00 "
    "7B 00 38 FE D4 03 0A 00 EC FF 1E 00 FA 00 83 FF "
    "F4 01 00 00 00 00 00 00 FF 7F 29 09 94 27 C6 11 "
    "2A 00 09 96 FF 93 93"
)


def crc16_ccitt_false(data):
    """CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF, no reflection, xorout 0.

    Bit-for-bit the same algorithm as ChipSatTelemetry::calculateCRC16.
    """
    crc = 0xFFFF
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            if crc & 0x8000:
                crc = ((crc << 1) ^ 0x1021) & 0xFFFF
            else:
                crc = (crc << 1) & 0xFFFF
    return crc


def decode_packet(b):
    """Decode exactly 55 bytes into engineering units.

    Raises ValueError for any other length. Never raises on bad CRC; check
    the returned ``crc_ok`` instead.
    """
    b = bytes(b)
    if len(b) != PACKET_LEN:
        raise ValueError(
            "packet must be exactly %d bytes, got %d" % (PACKET_LEN, len(b)))

    f = struct.unpack(STRUCT_FMT, b)
    (lat_e7, lon_e7, gps_alt_mm, env_alt_cm,
     ax, ay, az,
     gx, gy, gz,
     mx, my, mz,
     qi, qj, qk, qr,
     temp_cc, press_dhpa, hum_cpct,
     counter, csid, soc_x2, validity, crc_rx) = f

    crc_calc = crc16_ccitt_false(b[:CRC_LEN])

    out = {
        "lat_deg": lat_e7 / 1e7,
        "lon_deg": lon_e7 / 1e7,
        "gps_alt_m": gps_alt_mm / 1000.0,
        "pressure_alt_m": env_alt_cm / 100.0,
        "accel_mps2": (ax / 100.0, ay / 100.0, az / 100.0),
        "gyro_dps": (gx / 10.0, gy / 10.0, gz / 10.0),
        "mag_ut": (mx / 10.0, my / 10.0, mz / 10.0),
        "quat": (qi / 32767.0, qj / 32767.0, qk / 32767.0, qr / 32767.0),
        "temp_c": temp_cc / 100.0,
        "pressure_hpa": press_dhpa / 10.0,
        "humidity_pct": hum_cpct / 100.0,
        "counter": counter,
        "csid": csid,
        "soc_pct": soc_x2 / 2.0,
        "validity": validity,
        "crc_rx": crc_rx,
        "crc_calc": crc_calc,
        "crc_ok": crc_rx == crc_calc,
        "raw": {
            "latitudeE7": lat_e7,
            "longitudeE7": lon_e7,
            "gpsAltitudeMSLmm": gps_alt_mm,
            "environmentalAltitudeCm": env_alt_cm,
            "accelerationCentiMps2": (ax, ay, az),
            "gyroDeciDegPerSec": (gx, gy, gz),
            "magneticFieldDeciMicroTesla": (mx, my, mz),
            "orientationQ15": (qi, qj, qk, qr),
            "temperatureCentiC": temp_cc,
            "pressureDeciHpa": press_dhpa,
            "humidityCentiPercent": hum_cpct,
            "packetCounter": counter,
            "CSID": csid,
            "cellPercentageX2": soc_x2,
            "sensorValidity": validity,
            "crc16": crc_rx,
        },
    }
    for name, bit in VALIDITY_FLAGS.items():
        out[name] = bool(validity & (1 << bit))
    return out


# ---------------------------------------------------------------------------
# Hex parsing
# ---------------------------------------------------------------------------

_HEX_TOKEN = re.compile(r"^(?:0[xX])?[0-9A-Fa-f]{2}$")
# Leading decorations a serial monitor or the receiver may add before the
# bytes: "[12:34:56.789] ", "12:34:56.789 -> ", "12:34:56.789  " (tools/serial_log.py), "Raw bytes: ".
_LINE_PREFIX = re.compile(
    r"^\s*(?:\[[^\]]*\]\s*)?(?:\d{1,2}:\d{2}:\d{2}(?:\.\d+)?\s*(?:->)?\s+)?"
    r"(?:[A-Za-z][A-Za-z0-9 _/()-]*:\s*)?")


def parse_hex(s):
    """Parse a hex string into bytes.

    Accepts whitespace/comma separated bytes with optional 0x prefixes, a
    bare contiguous hex string, or any mix, across multiple lines.
    """
    tokens = re.split(r"[\s,;]+", s.strip())
    out = bytearray()
    for tok in tokens:
        if not tok:
            continue
        if tok[:2] in ("0x", "0X"):
            tok = tok[2:]
        if not tok or len(tok) % 2 or not re.fullmatch(r"[0-9A-Fa-f]+", tok):
            raise ValueError("bad hex token %r" % tok)
        out += bytes.fromhex(tok)
    return bytes(out)


# A hex-dump line must carry at least this many bytes unless it continues a
# run already in progress. This keeps single-value lines such as the
# firmware's "Sensor validity: 0xFF" from being glued onto a packet.
_MIN_RUN_START_BYTES = 4


def _line_hex_bytes(line):
    """Return (bytes, labeled) if this is a hex-dump line, else None.

    A hex-dump line is one whose content (after an optional label/timestamp
    prefix) consists solely of 2-digit hex tokens, e.g. the firmware's
    "5C 57 4C ..." or the T-Beam's "Raw bytes: 5C 57 4C ...". ``labeled`` is
    True when a prefix was stripped.
    """
    body = _LINE_PREFIX.sub("", line, count=1).strip()
    if not body:
        return None
    tokens = re.split(r"[\s,]+", body)
    if not all(_HEX_TOKEN.match(t) for t in tokens):
        return None
    labeled = body != line.strip()
    return bytes(int(t[-2:], 16) for t in tokens), labeled


def extract_hex_runs(text):
    """Find hex-dump runs in a serial log.

    Consecutive hex-dump lines are joined into one run (multi-line dumps);
    any non-hex line ends the run. Returns a list of (first_line_number,
    bytes), 1-based line numbers.
    """
    runs = []
    cur = bytearray()
    cur_start = None
    for lineno, line in enumerate(text.splitlines(), 1):
        hit = _line_hex_bytes(line)
        if hit is not None:
            hb, labeled = hit
            if len(hb) < _MIN_RUN_START_BYTES and (labeled or not cur):
                hit = None
        if hit is None:
            if cur:
                runs.append((cur_start, bytes(cur)))
            cur = bytearray()
            cur_start = None
            continue
        if cur_start is None:
            cur_start = lineno
        cur += hb
    if cur:
        runs.append((cur_start, bytes(cur)))
    return runs


def parse_log_text(text, warn=None):
    """Decode every well-formed packet in a log; skip and warn on the rest.

    Each returned dict is a decode_packet() result plus a "line" key.
    """
    if warn is None:
        def warn(msg):
            sys.stderr.write("warning: %s\n" % msg)
    rows = []
    for lineno, run in extract_hex_runs(text):
        if len(run) != PACKET_LEN:
            warn("line %d: hex run of %d bytes is not a %d-byte packet, skipped"
                 % (lineno, len(run), PACKET_LEN))
            continue
        p = decode_packet(run)
        if not p["crc_ok"]:
            warn("line %d: bad CRC (received 0x%04X, calculated 0x%04X), skipped"
                 % (lineno, p["crc_rx"], p["crc_calc"]))
            continue
        p["line"] = lineno
        rows.append(p)
    return rows


# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------

# (column name, formatter). Shared by the table and CSV output.
COLUMNS = (
    ("line", lambda p: str(p.get("line", ""))),
    ("counter", lambda p: str(p["counter"])),
    ("csid", lambda p: str(p["csid"])),
    ("lat_deg", lambda p: "%.7f" % p["lat_deg"]),
    ("lon_deg", lambda p: "%.7f" % p["lon_deg"]),
    ("gps_alt_m", lambda p: "%.3f" % p["gps_alt_m"]),
    ("press_alt_m", lambda p: "%.2f" % p["pressure_alt_m"]),
    ("ax", lambda p: "%.2f" % p["accel_mps2"][0]),
    ("ay", lambda p: "%.2f" % p["accel_mps2"][1]),
    ("az", lambda p: "%.2f" % p["accel_mps2"][2]),
    ("gx", lambda p: "%.1f" % p["gyro_dps"][0]),
    ("gy", lambda p: "%.1f" % p["gyro_dps"][1]),
    ("gz", lambda p: "%.1f" % p["gyro_dps"][2]),
    ("mx", lambda p: "%.1f" % p["mag_ut"][0]),
    ("my", lambda p: "%.1f" % p["mag_ut"][1]),
    ("mz", lambda p: "%.1f" % p["mag_ut"][2]),
    ("qi", lambda p: "%.5f" % p["quat"][0]),
    ("qj", lambda p: "%.5f" % p["quat"][1]),
    ("qk", lambda p: "%.5f" % p["quat"][2]),
    ("qr", lambda p: "%.5f" % p["quat"][3]),
    ("temp_c", lambda p: "%.2f" % p["temp_c"]),
    ("press_hpa", lambda p: "%.1f" % p["pressure_hpa"]),
    ("hum_pct", lambda p: "%.2f" % p["humidity_pct"]),
    ("soc_pct", lambda p: "%.1f" % p["soc_pct"]),
    ("validity", lambda p: "0x%02X" % p["validity"]),
    ("crc", lambda p: "0x%04X" % p["crc_rx"]),
    ("crc_ok", lambda p: "1" if p["crc_ok"] else "0"),
)


def write_csv(rows, out):
    w = csv.writer(out, lineterminator="\n")
    w.writerow([name for name, _ in COLUMNS])
    for p in rows:
        w.writerow([fmt(p) for _, fmt in COLUMNS])


def write_table(rows, out):
    cells = [[fmt(p) for _, fmt in COLUMNS] for p in rows]
    header = [name for name, _ in COLUMNS]
    widths = [len(h) for h in header]
    for row in cells:
        for i, c in enumerate(row):
            widths[i] = max(widths[i], len(c))
    out.write("  ".join(h.rjust(w) for h, w in zip(header, widths)) + "\n")
    for row in cells:
        out.write("  ".join(c.rjust(w) for c, w in zip(row, widths)) + "\n")


def format_packet_detail(p):
    """Multi-line human-readable dump of one packet (mirrors the T-Beam)."""
    v = p["validity"]
    lines = [
        "Packet counter: %d" % p["counter"],
        "ChipSat ID (CSID): %d" % p["csid"],
        "GPS:",
        "  Latitude:  %.7f deg" % p["lat_deg"],
        "  Longitude: %.7f deg" % p["lon_deg"],
        "  GPS altitude MSL: %.3f m" % p["gps_alt_m"],
        "  Pressure altitude: %.2f m" % p["pressure_alt_m"],
        "Linear acceleration (m/s^2): X %.2f  Y %.2f  Z %.2f" % p["accel_mps2"],
        "Gyroscope (deg/s):           X %.1f  Y %.1f  Z %.1f" % p["gyro_dps"],
        "Magnetometer (uT):           X %.1f  Y %.1f  Z %.1f" % p["mag_ut"],
        "Quaternion: I %.5f  J %.5f  K %.5f  Real %.5f" % p["quat"],
        "Environmental:",
        "  Temperature: %.2f deg C" % p["temp_c"],
        "  Pressure: %.1f hPa" % p["pressure_hpa"],
        "  Humidity: %.2f %%RH" % p["humidity_pct"],
        "Cell state of charge: %.1f %%" % p["soc_pct"],
        "Sensor validity: 0x%02X" % v,
        "  Linear acceleration valid: %s" % ("YES" if p["valid_linacc"] else "NO"),
        "  Gyroscope valid: %s" % ("YES" if p["valid_gyro"] else "NO"),
        "  Magnetometer valid: %s" % ("YES" if p["valid_mag"] else "NO"),
        "  Quaternion valid: %s" % ("YES" if p["valid_quat"] else "NO"),
        "  GPS valid: %s" % ("YES" if p["valid_gnss"] else "NO"),
        "  State of charge valid: %s" % ("YES" if p["valid_soc"] else "NO"),
        "  Environmental valid: %s" % ("YES" if p["valid_env"] else "NO"),
        "  All data fresh this cycle: %s" % ("YES" if p["all_fresh"] else "NO"),
        "CRC: received 0x%04X, calculated 0x%04X -> %s"
        % (p["crc_rx"], p["crc_calc"], "OK" if p["crc_ok"] else "MISMATCH"),
    ]
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
# Self test
# ---------------------------------------------------------------------------

def selftest(out=None):
    """Decode the notes' test vector and compare against the documented values."""
    if out is None:
        out = sys.stdout
    pkt = parse_hex(SELFTEST_HEX)
    checks = [("length", len(pkt), PACKET_LEN)]
    p = decode_packet(pkt)
    expected = [
        ("lat_deg", p["lat_deg"], 42.44335),
        ("lon_deg", p["lon_deg"], -76.50188),
        ("gps_alt_m", p["gps_alt_m"], 250.000),
        ("pressure_alt_m", p["pressure_alt_m"], 245.00),
        ("accel_x", p["accel_mps2"][0], 1.23),
        ("accel_y", p["accel_mps2"][1], -4.56),
        ("accel_z", p["accel_mps2"][2], 9.80),
        ("gyro_x", p["gyro_dps"][0], 1.0),
        ("gyro_y", p["gyro_dps"][1], -2.0),
        ("gyro_z", p["gyro_dps"][2], 3.0),
        ("mag_x", p["mag_ut"][0], 25.0),
        ("mag_y", p["mag_ut"][1], -12.5),
        ("mag_z", p["mag_ut"][2], 50.0),
        ("quat_i", p["quat"][0], 0.0),
        ("quat_j", p["quat"][1], 0.0),
        ("quat_k", p["quat"][2], 0.0),
        ("quat_real", p["quat"][3], 1.0),
        ("temp_c", p["temp_c"], 23.45),
        ("pressure_hpa", p["pressure_hpa"], 1013.2),
        ("humidity_pct", p["humidity_pct"], 45.50),
        ("counter", p["counter"], 42),
        ("csid", p["csid"], 9),
        ("soc_pct", p["soc_pct"], 75.0),
        ("validity", p["validity"], 0xFF),
        ("crc_rx", p["crc_rx"], 0x9393),
        ("crc_calc", p["crc_calc"], 0x9393),
        ("crc_ok", p["crc_ok"], True),
        ("crc_check_123456789", crc16_ccitt_false(b"123456789"), 0x29B1),
    ]
    checks += expected
    failures = 0
    for name, got, want in checks:
        if isinstance(want, float):
            ok = abs(got - want) < 1e-6
        else:
            ok = got == want
        failures += not ok
        out.write("%-22s %-14r %-14r %s\n" % (name, got, want, "ok" if ok else "FAIL"))
    out.write("selftest: %s (%d checks, %d failed)\n"
              % ("PASS" if failures == 0 else "FAIL", len(checks), failures))
    return failures == 0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Decode DeSCENT ChipSat 55-byte telemetry packets.")
    ap.add_argument("logfile", nargs="?",
                    help="serial capture text file containing hex-dump packet lines")
    ap.add_argument("--hex", metavar="BYTES",
                    help="decode a single packet given as hex (e.g. \"5C 57 4C ...\")")
    ap.add_argument("--csv", action="store_true",
                    help="emit CSV instead of an aligned table (logfile mode)")
    ap.add_argument("--selftest", action="store_true",
                    help="decode the documented test vector and check every field")
    args = ap.parse_args(argv)

    modes = sum(bool(x) for x in (args.logfile, args.hex, args.selftest))
    if modes != 1:
        ap.error("give exactly one of: a logfile, --hex, or --selftest")

    if args.selftest:
        return 0 if selftest() else 1

    if args.hex:
        try:
            pkt = parse_hex(args.hex)
            p = decode_packet(pkt)
        except ValueError as e:
            sys.stderr.write("error: %s\n" % e)
            return 2
        sys.stdout.write(format_packet_detail(p))
        return 0 if p["crc_ok"] else 1

    try:
        with open(args.logfile, "r", errors="replace") as f:
            text = f.read()
    except OSError as e:
        sys.stderr.write("error: %s\n" % e)
        return 2
    rows = parse_log_text(text)
    if args.csv:
        write_csv(rows, sys.stdout)
    else:
        write_table(rows, sys.stdout)
    if not rows:
        sys.stderr.write("warning: no valid packets found in %s\n" % args.logfile)
    return 0


if __name__ == "__main__":
    sys.exit(main())
