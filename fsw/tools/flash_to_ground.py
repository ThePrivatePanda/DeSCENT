#!/usr/bin/env python3
"""Turns a flash dump into a log the ground software opens with its own "Open log" button.

    ./flash_to_ground.py dump.txt                     writes dump.log next to it
    ./flash_to_ground.py dump.txt -o flight.log
    ./flash_to_ground.py --port /dev/ttyACM2 -o flight.log      board running bench/flash_dump

The ground software reads "# descent-ground log v1" files: one line per received line, as
<unix ms> TAB <receiver> TAB <text>. A packet off the chip goes in as a PKT line, the same shape
the T-Beam prints, with the RSSI, SNR and frequency error left empty. They arrive as NaN and the
dashboard shows them blank, which is honest: nothing on the chip was ever received over the air.

The time on each line comes from the time anchors in the log, so it is real UTC. A log with no
anchor, which is what an indoor run gives, is dated from 1970 instead: the spacing is right and the
date is obviously not. --start puts it at a real time.
"""

import argparse
import sys
from datetime import datetime, timezone
from pathlib import Path

import flash_replay

HEADER = "# descent-ground log v1"
kBootGapMs = 1000   # between boots laid end to end, so one doesn't run into the next


def rx_key(base, boot, per_boot):
    """The dashboard keeps one source line per receiver key, so a dump with more than one boot gives
    each boot its own key. Otherwise the second boot's line would replace the first's."""
    return f"{base}-boot{boot}" if per_boot else base


def clean(value):
    """A source line is split on commas before the = is read, so a comma in a value would quietly cut
    the record in half. Tabs would break the three field split as well"""
    return str(value).replace(",", "_").replace("\t", " ").strip()


def source_line(unix_ms, rx, origin, boot, packets, anchor, time_source):
    """#DG,SRC says where a log came from: the dashboard shows it as a receiver called a flash dump,
    with the anchor its times rest on, so a replay can't be mistaken for live reception"""
    fields = [f"kind=flash", f"from={clean(origin)}", f"boot={boot}", f"packets={packets}",
              f"time_source={time_source}"]
    if anchor:
        when = datetime.fromtimestamp(anchor["epoch"] + anchor["nano"] / 1e9, tz=timezone.utc)
        fields += [f"anchor_uptime_ms={anchor['ms']}",
                   f"anchor_utc={when.strftime('%Y-%m-%dT%H:%M:%S.')}{when.microsecond // 1000:03d}Z",
                   f"anchor_tacc_ns={anchor['acc_ns']}"]
    return f"{unix_ms}\t{rx}\t#DG,SRC,v1," + ",".join(fields)


def to_lines(records, anchors, base_name, origin, basis="start"):
    """A source line per receiver key, then one PKT line per packet in time order. The ground software
    sorts too, but a log that is already in order reads better in a text editor."""
    boots = sorted({r["boot"] for r in records})
    per_boot = len(boots) > 1
    head = []
    for boot in boots:
        mine = sorted((r for r in records if r["boot"] == boot), key=lambda r: r["unix_ms"])
        same_boot = sorted((a for a in anchors if a["boot"] == boot), key=lambda a: a["ms"])
        # the first record's own time, so the source line sorts ahead of what it describes
        head.append(source_line(mine[0]["unix_ms"], rx_key(base_name, boot, per_boot), origin, boot,
                                len(mine), same_boot[0] if same_boot else None,
                                "anchor" if same_boot else basis))

    body = []
    was = None
    for record in sorted(records, key=lambda r: r["unix_ms"]):
        rx = rx_key(base_name, record["boot"], per_boot)
        # a marker whenever the boot changes, so the ground software can tell one boot from the next
        # without reading anything into the packet counter, which counts transmissions and not records
        if record["boot"] != was:
            body.append(f"{record['unix_ms']}\t{rx}\t#DG,BOOT,v1,n={record['boot']}")
            was = record["boot"]
        text = f"PKT,{flash_replay.PACKET_BYTES},{record['hex']},,,"
        body.append(f"{record['unix_ms']}\t{rx}\t{text}")
    return head, body


def date_records(records, anchors, start, dumped=None):
    """unix_ms for every record. add_utc has already dated the boots that have an anchor.

    A GPS lock is far from guaranteed - indoors there is never one - so a boot can have no absolute
    time in it at all. Every boot counts its own milliseconds from zero, so they are laid end to end
    in boot order, which is the order they happened in.

    The case that matters: a chip usually holds some boots with a GPS fix and some without. Those
    without are placed on the same timeline as those with, next to the boots they happened between,
    rather than at 1970 - a log holding both 1970 and a real date spans 56 years, and a viewer sorting
    it by time shreds it into fragments. The gap between boots is unknown, since the board was off, so
    a nominal one second is used and each boot's own time_source says whether its date is real."""
    for record in records:
        if record.get("utc"):
            record["unix_ms"] = round(record["utc"] * 1000.0)

    boots = sorted({r["boot"] for r in records})
    mine = {b: [r for r in records if r["boot"] == b] for b in boots}
    span = {b: (min(r["ms"] for r in mine[b]), max(r["ms"] for r in mine[b])) for b in boots}
    isDated = {b: any(r.get("utc") for r in mine[b]) for b in boots}

    def place(boot, startMs):
        first = span[boot][0]
        for record in mine[boot]:
            record["unix_ms"] = startMs + record["ms"] - first

    anyDated = [b for b in boots if isDated[b]]
    if start is not None and not anyDated:
        basis = "start"
        base = round(start.timestamp() * 1000.0)
    elif not anyDated:
        basis = "none"
        base = 0       # the first record at epoch 0, so the spacing is right and the date obviously is not
    else:
        basis = "anchor"
        base = None

    if base is not None:
        for boot in boots:
            place(boot, base)
            base += span[boot][1] - span[boot][0] + kBootGapMs
        return records, [], basis

    # Forward from the first boot that has a fix, then backward from it, so an undated boot sits
    # between the boots it actually happened between
    firstDated = anyDated[0]
    cursor = max(r["unix_ms"] for r in mine[firstDated])
    for boot in [b for b in boots if b > firstDated]:
        if isDated[boot]:
            cursor = max(r["unix_ms"] for r in mine[boot])
            continue
        place(boot, cursor + kBootGapMs)
        cursor = max(r["unix_ms"] for r in mine[boot])

    cursor = min(r["unix_ms"] for r in mine[firstDated])
    for boot in [b for b in boots if b < firstDated][::-1]:
        if isDated[boot]:
            cursor = min(r["unix_ms"] for r in mine[boot])
            continue
        length = span[boot][1] - span[boot][0]
        place(boot, cursor - kBootGapMs - length)
        cursor = min(r["unix_ms"] for r in mine[boot])

    return records, [], basis


def how_text(basis, start, dumped=None):
    return f"--start {start}" if basis == "start" else "1970, since the log has no gps time in it"


def write_log(out, records, anchors, rx, origin, basis, how):
    """The file the ground software opens. flash_replay.py calls this too, so one command can pull a
    chip and leave a log ready to open"""
    head, body = to_lines(records, anchors, rx, Path(origin).name, basis or "start")
    with open(out, "w", newline="\n") as handle:
        handle.write(f"{HEADER} from flash, written {datetime.now(timezone.utc).isoformat()}\n")
        handle.write("\n".join(head) + "\n")
        # the dashboard skips any line without two tabs, so these are just for whoever opens the file
        handle.write(f"# dump {origin}, {len(records)} packets, {len(anchors)} time anchors\n")
        if not anchors:
            handle.write(f"# no time anchor in the log, times are counted back from {how}\n")
        for anchor in anchors:
            when = datetime.fromtimestamp(anchor["epoch"], tz=timezone.utc).isoformat()
            handle.write(f"# anchor boot {anchor['boot']} uptime {anchor['ms']} ms = {when}, "
                         f"receiver accuracy {anchor['acc_ns']} ns\n")
        handle.write("\n".join(body) + "\n")


def main():
    parser = argparse.ArgumentParser(description="flash dump to a descent-ground log")
    parser.add_argument("file", nargs="?", help="a dump captured from bench/flash_dump")
    parser.add_argument("--port", help="read the dump off the board instead")
    parser.add_argument("--baud", type=int, default=115200)
    parser.add_argument("--save", help="with --port, keep the raw dump here as well")
    parser.add_argument("-o", "--out", help="output log (default: the dump's name with .log)")
    parser.add_argument("--rx", default="flash", help="receiver name shown in the dashboard")
    parser.add_argument("--start", help="UTC time of the first record, ISO 8601, for a log with no "
                                        "time anchors, e.g. 2026-09-23T14:07:00Z")
    args = parser.parse_args()

    if not args.file and not args.port:
        parser.error("give a dump file or --port")

    start = None
    if args.start:
        start = datetime.fromisoformat(args.start.replace("Z", "+00:00"))
        if start.tzinfo is None:
            start = start.replace(tzinfo=timezone.utc)

    if args.port:
        text = flash_replay.read_from_board(args.port, args.baud, args.save)
        lines = text.splitlines() if isinstance(text, str) else text
        source = args.port
        dumped = datetime.now(timezone.utc)
    else:
        lines = Path(args.file).read_text(errors="replace").splitlines()
        source = args.file
        # when the capture was written, which is about when the last record came off the chip
        dumped = datetime.fromtimestamp(Path(args.file).stat().st_mtime, tz=timezone.utc)

    records, anchors = flash_replay.parse_dump(lines)
    if not records:
        sys.exit(f"no packet records in {source}")

    records, undated, basis = date_records(records, anchors, start, dumped)
    if basis == "none":
        print("no gps time in this log, so it is dated from 1970. The spacing between records is "
              "right, the date is not. --start puts it at a real time", file=sys.stderr)
    if undated:
        boots = sorted({r["boot"] for r in undated})
        print(f"leaving out {len(undated)} records from boot(s) {boots}: the rest of the log has real "
              f"gps time and these have none, so they would sit 56 years to the left. --start places them",
              file=sys.stderr)
    if not records:
        sys.exit(f"nothing usable in {source}")

    out = Path(args.out) if args.out else Path(source).with_suffix(".log")
    write_log(out, records, anchors, args.rx, source, basis, how_text(basis, args.start, dumped))
    span = datetime.fromtimestamp(min(r["unix_ms"] for r in records) / 1000.0, tz=timezone.utc)

    print(f"{len(records)} packets, {len(anchors)} anchors -> {out}", flush=True)
    print(f"first packet {span.isoformat()}", flush=True)
    print("open it in the ground software with Open log", flush=True)


if __name__ == "__main__":
    main()
