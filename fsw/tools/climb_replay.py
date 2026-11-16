#!/usr/bin/env python3
"""Replays a take-off rule over pulled flight logs: a climb with the IMU moving now and then.

usage: ./climb_replay.py [--window S] [--toa MS] PULL_DIR [PULL_DIR ...]

Wherever the firmware was on the ground or in boot-air, the rule calls the air when the last whole
second's lowest barometer reading is 15 m over the ground baseline and some IMU reading in the last
WINDOW seconds (10 by default) moved: 2 m/s2 or 20 deg/s. The baseline is the firmware's: it follows
the altitude only while the vertical rate is under 0.3 m/s, and starts again where the board came to
rest at each toground or bootairend.

Each trigger is printed against the firmware's own toair, with the IMU around it (how many records
moved in the window, the longest stretch with none in the 30 s before, the largest rotation), how far
apart the records were, and how much height was lost in the minute after, so a drone climb can be
told from a board on the bench.

The firmware no longer hands the phase a reading whose conversion overlapped a transmission. The
records don't say which did, but the packet counter they carry changes as each transmission starts,
so with --toa (the time on air: 143 at SF7, 860 at SF10, 3187 at SF12) the readings recorded from the
record before a change until the time on air and 150 ms later are left out here too. That is an estimate: the
console's wait before each packet moved the start by up to about 70 ms. On the ground
the records are only one per log gap, so moving readings between them are missing: the rule fires
later here, or not at all, where the board had more.
"""
import sys

import flight_review

RISE_M = 15.0
MOVING_ACCEL = 2.0
MOVING_DPS = 20.05   # kQuietGyroRadPerSec, 0.35 rad/s
RATE_WINDOW_MS = 800
BASELINE_RATE = 0.3
BASELINE_GAIN = 0.02


def moving(p):
    return p['valid_linacc'] and (p['accel'] >= MOVING_ACCEL or p['gyro'] >= MOVING_DPS)


def phase_events(events):
    out = []
    for e in events:
        if e.get('source') == 'phase' and e.get('what') in ('toair', 'toground', 'bootairend'):
            out.append((int(e['first_ms']), e['what'], e.get('reason', '')))
    return sorted(out)


def replay_boot(n, boot, window_ms, toa_ms):
    packets = boot['packets']
    events = phase_events(boot['events'])
    print(f'  boot {n}: {len(packets)} records, up {packets[0]["up"] / 1000:.1f} to {packets[-1]["up"] / 1000:.1f} s')
    for ms, what, why in events:
        print(f'    firmware {what} at {ms / 1000:.1f} s ({why})')

    state = 'boot-air'
    ev = 0
    history = []   # (ms, alt) of the last eight readings
    baseline = None
    alt = None
    seen = None
    second = None
    second_low = None
    low = None
    last_moving = None
    fired = False
    tx_from = None
    for i, p in enumerate(packets):
        up = p['up']
        while ev < len(events) and events[ev][0] <= up:
            state = 'air' if events[ev][1] == 'toair' else 'ground'
            if state == 'ground' and alt is not None:
                baseline = alt
            fired = False
            ev += 1
        if moving(p):
            last_moving = up
        if i > 0 and p['counter'] != packets[i - 1]['counter']:
            tx_from = packets[i - 1]['up']   # it started after the record before; sparse ground records keep theirs
        # each record repeats the newest reading, so a reading is judged once, where it first shows
        if p['valid_env'] and p['pressure_alt_m'] != seen:
            seen = p['pressure_alt_m']
            if not (toa_ms and tx_from is not None and up - tx_from <= toa_ms + 150):
                alt = seen
                history = (history + [(up, alt)])[-8:]
                rate = 0.0
                for ms, a in reversed(history):
                    if up - ms >= RATE_WINDOW_MS:
                        rate = (alt - a) * 1000.0 / (up - ms)
                        break
                if baseline is None:
                    baseline = alt
                elif state != 'air' and abs(rate) < BASELINE_RATE:
                    baseline += (alt - baseline) * BASELINE_GAIN
                if up // 1000 != second:
                    if second_low is not None:
                        low = second_low
                    second = up // 1000
                    second_low = alt
                else:
                    second_low = min(second_low, alt)
        if state == 'air' or fired or low is None or low - baseline < RISE_M:
            continue
        if last_moving is None or up - last_moving > window_ms:
            continue
        fired = True
        before = [q for q in packets[:i + 1] if up - window_ms <= q['up']]
        last30 = [q for q in packets[:i + 1] if up - 30000 <= q['up']]
        stretch, since = 0, None
        for q in last30:
            if moving(q):
                since = q['up']
            else:
                stretch = max(stretch, q['up'] - (since if since is not None else last30[0]['up']))
        gaps = [b['up'] - a['up'] for a, b in zip(before, before[1:])]
        after = [q['pressure_alt_m'] for q in packets[i:] if q['valid_env'] and q['up'] - up <= 60000]
        firmware_air = next((ms for ms, what, _ in events if what == 'toair' and ms >= up - 60000), None)
        print(f'    RULE air at {up / 1000:.1f} s ({state}): lowest {low - baseline:+.1f} m over the baseline '
              f'{baseline:.1f} m; {sum(moving(q) for q in before)} of {len(before)} records moving in the last '
              f'{window_ms // 1000} s, longest still stretch in 30 s {stretch / 1000:.1f} s, rotation up to '
              f'{max(q["gyro"] for q in before):.0f} deg/s; record gap up to {max(gaps, default=0)} ms; '
              f'height lost in the next minute {low - min(after):.1f} m; firmware toair '
              + (f'{firmware_air / 1000:.1f} s' if firmware_air is not None else 'none near'))


def main():
    args = sys.argv[1:]
    window_ms = 10000
    toa_ms = 0
    while args[:1] in (['--window'], ['--toa']):
        if args[0] == '--window':
            window_ms = int(float(args[1]) * 1000)
        else:
            toa_ms = int(args[1])
        args = args[2:]
    if not args:
        sys.exit(__doc__)
    for path in args:
        print(path)
        for n, boot in sorted(flight_review.read_pull(path).items()):
            if boot['packets']:
                replay_boot(n, boot, window_ms, toa_ms)


if __name__ == '__main__':
    main()
