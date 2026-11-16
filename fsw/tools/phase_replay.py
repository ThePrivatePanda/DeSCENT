#!/usr/bin/env python3
"""Replays a candidate phase over pulled flight logs: quick into the air, slow back to the ground.

usage: ./phase_replay.py [--toa MS] [--hint MS N] PULL_DIR [PULL_DIR ...]

Into the air, from the ground and boot-air:
  tumble    rotation over 300 deg/s for 1 s, readings under it within 200 ms let through
  freefall  felt acceleration (gravity put back from the quaternion) under 3 m/s2 for 150 ms
  climb     the last whole second's lowest barometer reading 15 m over the ground baseline, no IMU
            condition. Readings taken during a transmission are left out as in climb_replay.py, from
            the record before a counter change to the time on air (--toa) and 300 ms later: the
            console's wait before the radio starts (about 70 ms on these images) and the 120 ms
            conversion
  hint      N moving readings (2 m/s2 or 20 deg/s) in the last MS (default 2000 ms and 20)
Back to the ground, as the firmware does, at any height: quiet 30 s and flat 30 s, or quiet 60 s.
Quiet means no moving reading, the orientation not turned 15 deg in 5 s and the barometer not
falling 5 m/s for 4 s. A record with no valid IMU reading is quiet, as a missing IMU is to the
firmware. The baseline follows the barometer on the ground while it moves under 0.3 m/s
and is where the board came to rest on landing.

Per boot it prints the firmware's own phase events and the replay's air spells, each with what took
it up and whether it was a flight (a climb of 15 m or a fall) or only handling, the time spent in
the air on handling and what that would cost: chip records at 20 Hz and transmissions over the
ground rate, the gaps measured from the boot's own packet counter on ground and in the air.

What this can't show: records on the ground are one per log gap, so readings between them are
missing and the hint can't form there (a board at 1 Hz couldn't either, it steps to 20 Hz on its
first moving reading); which readings overlapped a transmission is estimated from the counter; and a
board still reads quiet records only, so a single bad reading between records isn't seen.
"""
import math
import statistics
import sys

import flight_review
from fall_replay import felt

MOVING_ACCEL = 2.0
MOVING_DPS = 20.05
RISE_M = 15.0
FLAT_M = 3.0
TUMBLE_DPS = 300.0
FREEFALL = 3.0
TURN_DEG = 15.0


def moving(p):
    return p['valid_linacc'] and (p['accel'] >= MOVING_ACCEL or p['gyro'] >= MOVING_DPS)


def quat(p):
    q = p['quat']
    return q if p['valid_quat'] and any(q) else None


def angle(a, b):
    d = abs(sum(x * y for x, y in zip(a, b)))
    return math.degrees(2 * math.acos(min(1.0, d)))


def firmware_phases(events):
    out = []
    for e in events:
        if e.get('source') == 'phase' and e.get('what') in ('toair', 'toground', 'bootairend'):
            out.append((int(e['first_ms']), e['what'], e.get('reason', '')))
    return sorted(out)


def cycle_ms(packets, events):
    """Median spacing of counter changes on the ground and in the air, by the firmware's phase."""
    phases = firmware_phases(events)
    spans = {'ground': [], 'air': []}
    last = None
    for a, b in zip(packets, packets[1:]):
        if b['counter'] == a['counter']:
            continue
        state = 'ground'
        for ms, what, _ in phases:
            if ms <= b['up']:
                state = 'air' if what == 'toair' else 'ground'
        if last is not None and b['up'] - a['up'] < 500:   # dense records only
            spans[state].append(b['up'] - last)
        last = b['up']
    return {k: statistics.median(v) if v else None for k, v in spans.items()}


def replay_boot(n, boot, toa_ms, hint_ms, hint_n):
    packets = boot['packets']
    phases = firmware_phases(boot['events'])
    span = (packets[-1]['up'] - packets[0]['up']) / 1000
    print(f'  boot {n}: {len(packets)} records over {span:.0f} s')
    for ms, what, why in phases:
        print(f'    firmware {what} at {ms / 1000:.1f} s ({why})')

    air = False
    air_from = None
    why = None
    peak = None
    spells = []
    baseline = alt = None
    seen = None
    tx_from = None
    history = []
    second = second_low = second_high = None
    low = None
    bins = []          # (second, low, high) of closed seconds
    flat_since = None
    flat_lo = flat_hi = None
    still_since = packets[0]['up']
    moves = []         # ups of moving readings
    spin_from = spin_last = None
    light_from = light_last = None
    quats = {}         # second -> first quaternion in it
    fell = False

    for i, p in enumerate(packets):
        up = p['up']
        if i > 0 and p['counter'] != packets[i - 1]['counter']:
            tx_from = packets[i - 1]['up']
        if p['valid_env'] and p['pressure_alt_m'] != seen:
            seen = p['pressure_alt_m']
            if not (toa_ms and tx_from is not None and up - tx_from <= toa_ms + 300):
                alt = seen
                history = (history + [(up, alt)])[-8:]
                rate = 0.0
                for ms, a in reversed(history):
                    if up - ms >= 800:
                        rate = (alt - a) * 1000 / (up - ms)
                        break
                if baseline is None:
                    baseline = alt
                elif not air and abs(rate) < 0.3:
                    baseline += (alt - baseline) * 0.02
                if air and alt > peak:
                    peak = alt
                if up // 1000 != second:
                    if second is not None:
                        low = second_low
                        bins = (bins + [(second, second_low, second_high)])[-4:]
                        if flat_since is not None and max(flat_hi[1], second_high) - min(flat_hi[0], second_high) <= FLAT_M \
                                and max(flat_lo[1], second_low) - min(flat_lo[0], second_low) <= FLAT_M:
                            flat_lo = (min(flat_lo[0], second_low), max(flat_lo[1], second_low))
                            flat_hi = (min(flat_hi[0], second_high), max(flat_hi[1], second_high))
                        else:
                            flat_since = second * 1000
                            flat_lo = (second_low, second_low)
                            flat_hi = (second_high, second_high)
                    second, second_low, second_high = up // 1000, alt, alt
                else:
                    second_low, second_high = min(second_low, alt), max(second_high, alt)
        falling = len(bins) == 4 and all(b[0] == a[0] + 1 and b[1] <= a[1] - 5 and b[2] <= a[2] - 5
                                         for a, b in zip(bins, bins[1:]))
        q = quat(p)
        turned = False
        if q is not None:
            quats.setdefault(up // 1000, q)
            then = quats.get(up // 1000 - 5)
            turned = then is not None and angle(q, then) > TURN_DEG
        mv = moving(p)
        if mv:
            moves.append(up)
        if mv or turned or falling:
            still_since = up
        if p['gyro'] >= TUMBLE_DPS:
            if spin_last is None or up - spin_last > 200:
                spin_from = up
            spin_last = up
        f = felt(p) if q is not None else None
        if f is not None and f < FREEFALL:
            if light_last is None or up - light_last > 200:
                light_from = up
            light_last = up
        else:
            light_from = light_last = None

        if not air:
            took = None
            if spin_last == up and up - spin_from >= 1000:
                took = 'tumble'
            elif light_last == up and up - light_from >= 150:
                took = 'freefall'
            elif low is not None and baseline is not None and low - baseline >= RISE_M:
                took = 'climb'
            elif sum(1 for m in moves[-hint_n:] if up - m < hint_ms) >= hint_n:
                took = 'hint'
            if took:
                air, air_from, why, peak = True, up, took, alt if alt is not None else 0.0
                fell = took in ('tumble', 'freefall')
        else:
            if (spin_last == up and up - spin_from >= 1000) or (light_last == up and up - light_from >= 150):
                fell = True
            quiet = up - still_since
            climbed = peak - baseline if baseline is not None else 0.0
            flat = flat_since is not None and second is not None and second * 1000 - flat_since >= 30000
            if (quiet >= 30000 and flat) or quiet >= 60000:
                spells.append((air_from, up, why, climbed >= RISE_M or fell))
                air = False
                baseline = alt
                peak = None
    if air:
        spells.append((air_from, packets[-1]['up'], why, (baseline is not None and peak - baseline >= RISE_M) or fell))

    gaps = cycle_ms(packets, boot['events'])
    # boot-air logs at full rate whatever the phase, until the firmware ended it or went up
    boot_air_end = next((ms for ms, what, _ in phases if what in ('bootairend', 'toair')), min(300000, packets[-1]['up']))
    handled = beyond = 0
    for a, b, took, flight in spells:
        kind = 'flight' if flight else 'handling only'
        print(f'    replay air {a / 1000:.1f}-{b / 1000:.1f} s ({(b - a) / 1000:.0f} s), up on {took}, {kind}')
        if not flight:
            handled += b - a
            beyond += max(0, b - max(a, boot_air_end))
    if handled:
        extra_tx = ''
        if gaps['air'] and gaps['ground']:
            extra = handled / gaps['air'] - handled / gaps['ground']
            extra_tx = f', {extra:.0f} more transmissions (cycle {gaps["air"]:.0f} ms in the air, {gaps["ground"]:.0f} on the ground)'
        print(f'    in the air on handling: {handled / 60000:.1f} min, {beyond / 60000:.1f} of it after boot-air, '
              f'{beyond * 20 // 1000} records at 20 Hz the chip would not have had{extra_tx}')
    return handled, beyond


def main():
    args = sys.argv[1:]
    toa_ms, hint_ms, hint_n = 0, 2000, 20
    while args[:1] in (['--toa'], ['--hint']):
        if args[0] == '--toa':
            toa_ms = int(args[1])
            args = args[2:]
        else:
            hint_ms, hint_n = int(args[1]), int(args[2])
            args = args[3:]
    if not args:
        sys.exit(__doc__)
    total = beyond = 0
    for path in args:
        print(path)
        for n, boot in sorted(flight_review.read_pull(path).items()):
            if len(boot['packets']) > 1:
                h, b = replay_boot(n, boot, toa_ms, hint_ms, hint_n)
                total += h
                beyond += b
    print(f'in the air on handling, all boots: {total / 60000:.1f} min, {beyond / 60000:.1f} of it after boot-air, '
          f'{beyond * 20 // 1000} records at 20 Hz')


if __name__ == '__main__':
    main()
