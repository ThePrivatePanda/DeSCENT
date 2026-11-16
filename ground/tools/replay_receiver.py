#!/usr/bin/env python3
# Prints a file of SDR receiver lines (docs/sdr-receiver.md) to stdout at the pace they came,
# so the app's command receiver can be tried without an SDR. In descent-ground.conf:
#
#   receiver = python3 ground/tools/replay_receiver.py ground/docs/sdr-receiver-examples.txt
#
# The only clock in those lines is each HB's uptime, so an HB goes out at its uptime and the
# lines between two HBs are spread evenly across the gap. Lines before the first HB go out at
# once, lines after the last one a second apart. It stops at the end of the file, and the app
# starts it again.
import os
import sys
import time


def uptime(line):
    parts = line.split(',')
    if parts[0] == 'HB' and len(parts) > 1 and parts[1].isdigit():
        return int(parts[1]) / 1000.0
    return None


# (seconds from the start, line) for every line
def schedule(lines):
    out = []
    waiting = []   # lines since the last HB, not yet given a time
    last = 0.0
    base = 0.0     # a restart in the file starts uptime again; carry on from where it was
    for line in lines:
        up = uptime(line)
        if up is None:
            waiting.append(line)
            continue
        if base + up < last:
            base = last
        at = base + up
        step = (at - last) / (len(waiting) + 1) if out else 0.0
        out += [(last + step * (i + 1), w) for i, w in enumerate(waiting)]
        out.append((at, line))
        waiting, last = [], at
    out += [(last + i + 1, w) for i, w in enumerate(waiting)]
    return out


def main():
    if len(sys.argv) != 2:
        sys.stderr.write('usage: replay_receiver.py <file of receiver lines>\n')
        return 2
    with open(sys.argv[1], encoding='ascii') as f:
        lines = [l for l in f.read().splitlines() if l]
    start = time.monotonic()
    try:
        for at, line in schedule(lines):
            time.sleep(max(0.0, start + at - time.monotonic()))
            sys.stdout.write(line + '\n')
            sys.stdout.flush()
    except BrokenPipeError:
        # Nothing reads any more. Python would complain again flushing stdout on the way out
        os.dup2(os.open(os.devnull, os.O_WRONLY), sys.stdout.fileno())
    return 0


if __name__ == '__main__':
    sys.exit(main())
