#!/usr/bin/env python3
"""What a pulled flight log says about phase detection and air data, boot by boot.

usage: ./flight_review.py PULL_DIR [PULL_DIR ...]

For every boot in a pull's ground.log (the decoded flash log):

  phases     the firmware's own phase events (toair, toground, bootairend, imufast, imuslow, descent,
             asleep, awake), at the time each happened
  what the sensors show, found here without the firmware's rules:
    moving   IMU readings over 2 m/s2 or 20 deg/s
    climb    the barometer more than 15 m above where the boot started
    still    20 s or more with every IMU reading under 0.3 m/s2 and 2 deg/s
  air data   for every stretch logged as air (boot-air and air): records, the gap between them,
             every gap over 100 ms, and how many records 20 Hz would give. Then the 30 s before each
             toair, which the RAM ring should have kept at full rate

Only the CRC-checked packets in the log are used. Times are uptime in the boot, in seconds.
"""
import math
import statistics
import sys

import descent_decode

MOVING_ACCEL = 2.0
MOVING_GYRO_DPS = 20.0
STILL_ACCEL = 0.3
STILL_GYRO_DPS = 2.0
STILL_FOR_MS = 20000
CLIMB_M = 15.0
FULL_RATE_MS = 50
GAP_MS = 100
RING_MS = 30000
PHASE_WHATS = ('toair', 'toground', 'bootairend', 'imufast', 'imuslow', 'descent', 'asleep', 'awake')


def read_pull(path):
    boots = {}
    for line in open(path + '/ground.log', errors='replace'):
        cols = line.rstrip('\n').split('\t')
        if len(cols) < 3 or not cols[1].startswith('flash-boot'):
            continue
        b = boots.setdefault(int(cols[1][len('flash-boot'):]), {'packets': [], 'events': []})
        text = cols[2]
        if text.startswith('PKT,55,'):
            fields = text.split(',')
            p = descent_decode.decode_packet(bytes.fromhex(fields[2]))
            if not p['crc_ok']:
                continue
            p['up'] = int(next(f for f in fields if f.startswith('up='))[3:])
            p['accel'] = math.hypot(*p['accel_mps2'])
            p['gyro'] = math.hypot(*p['gyro_dps'])
            b['packets'].append(p)
        elif text.startswith('#DG,SYS,'):
            e = dict(kv.split('=', 1) for kv in text.split(',')[2:] if '=' in kv)
            b['events'].append(e)
    for b in boots.values():
        b['packets'].sort(key=lambda p: p['up'])
    return boots


def s(ms):
    return f'{ms / 1000:.1f}'


def spells(packets, test, min_ms):
    """Stretches where every IMU reading passes test, at least min_ms long."""
    out, start, last = [], None, None
    for p in packets:
        if not (p['valid_linacc'] and p['valid_gyro']):
            continue
        if test(p):
            start = p['up'] if start is None else start
            last = p['up']
        else:
            if start is not None and last - start >= min_ms:
                out.append((start, last))
            start = None
    if start is not None and last - start >= min_ms:
        out.append((start, last))
    return out


def air_stretches(events, end_ms):
    """(start, end, label) for every stretch the log was writing as air."""
    out = []
    boot_air_end = next((int(e['first_ms']) for e in events if e.get('what') in ('bootairend', 'toair')), end_ms)
    out.append((0, boot_air_end, 'boot-air'))
    start = None
    for e in events:
        if e.get('source') != 'phase':
            continue
        if e['what'] == 'toair':
            start = int(e['first_ms'])
        elif e['what'] == 'toground' and start is not None:
            out.append((start, int(e['first_ms']), 'air'))
            start = None
    if start is not None:
        out.append((start, end_ms, 'air, to the end of the boot'))
    return out


def review_stretch(packets, a, b, label):
    inside = [p['up'] for p in packets if a <= p['up'] <= b]
    if len(inside) < 2:
        print(f'    {label} {s(a)}-{s(b)} s: {len(inside)} records')
        return
    gaps = [y - x for x, y in zip(inside, inside[1:])]
    big = [(x, y - x) for x, y in zip(inside, inside[1:]) if y - x > GAP_MS]
    span = inside[-1] - inside[0]
    print(f'    {label} {s(a)}-{s(b)} s: {len(inside)} records over {s(span)} s, '
          f'{span / FULL_RATE_MS + 1:.0f} at 20 Hz; gap median {statistics.median(gaps)} ms, max {max(gaps)} ms, '
          f'{len(big)} over {GAP_MS} ms')
    for at, gap in big[:8]:
        print(f'      gap of {gap} ms after {s(at)} s')
    if len(big) > 8:
        print(f'      and {len(big) - 8} more')


def review_boot(n, b):
    packets, events = b['packets'], b['events']
    if not packets:
        print(f'  boot {n}: no records')
        return
    end = packets[-1]['up']
    print(f'  boot {n}: {len(packets)} records, up {s(packets[0]["up"])} to {s(end)} s')
    phase = [e for e in events if e.get('source') == 'phase' and e.get('what') in PHASE_WHATS]
    for e in phase:
        when = s(int(e['first_ms'])) if e['first_ms'] == e['last_ms'] else f'{s(int(e["first_ms"]))}-{s(int(e["last_ms"]))}'
        print(f'    firmware: {e["what"]} at {when} s ({e["reason"]}, on {e["part"]})')
    if not phase:
        print('    firmware: no phase events')

    moving = [p['up'] for p in packets if p['valid_linacc'] and (p['accel'] >= MOVING_ACCEL or p['gyro'] >= MOVING_GYRO_DPS)]
    if moving:
        print(f'    sensors: {len(moving)} moving IMU readings, first {s(moving[0])} s, last {s(moving[-1])} s')
    alts = [(p['up'], p['pressure_alt_m']) for p in packets if p['valid_env']]
    if alts:
        start_alt = statistics.median(a for _, a in alts[:20])
        peak_up, peak = max(alts, key=lambda x: x[1])
        climbed = [u for u, a in alts if a - start_alt >= CLIMB_M]
        print(f'    sensors: barometer starts {start_alt:.1f} m, highest {peak:.1f} m ({peak - start_alt:+.1f} m) at {s(peak_up)} s'
              + (f'; over {CLIMB_M:.0f} m above the start from {s(climbed[0])} to {s(climbed[-1])} s' if climbed else ''))
    for a, z in spells(packets, lambda p: p['accel'] < STILL_ACCEL and p['gyro'] < STILL_GYRO_DPS, STILL_FOR_MS):
        if moving and a > moving[0]:
            print(f'    sensors: still from {s(a)} to {s(z)} s')

    for a, z, label in air_stretches(events, end):
        review_stretch(packets, a, z, label)
        if label != 'boot-air':
            review_stretch(packets, max(0, a - RING_MS), a, f'ring before {label}')
    ground = [p['up'] for p in packets]
    if len(ground) > 1:
        gaps = [y - x for x, y in zip(ground, ground[1:])]
        print(f'    whole boot: gap median {statistics.median(gaps)} ms, max {max(gaps)} ms')


def main():
    for path in sys.argv[1:]:
        print(path)
        boots = read_pull(path)
        for n in sorted(boots):
            review_boot(n, boots[n])


if __name__ == '__main__':
    main()
