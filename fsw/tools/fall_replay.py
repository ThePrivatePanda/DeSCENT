#!/usr/bin/env python3
"""Replays candidate fall detectors over pulled flight logs, to choose one before it goes in the firmware.

usage: ./fall_replay.py PULL_DIR [PULL_DIR ...]

The IMU reports linear acceleration (gravity taken out) and its orientation as a quaternion. Putting
gravity back in the sensor's own frame gives what the accelerometer felt: about 9.8 m/s2 at rest and
in a steady fall, near 0 in free fall. Two signals are tried, each over 20 Hz records:

  freefall  felt acceleration under FREEFALL_MPS2 for FREEFALL_MS or more
  tumbling  rotation over TUMBLE_DPS for TUMBLE_MS or more

Each hit is printed with the firmware's own phase at the time and the barometer's lowest reading in
the seconds after, so a real fall (height lost) can be told from handling.
"""
import collections
import math
import sys

import flight_review

G = 9.80665
FREEFALL_MPS2 = 3.0
FREEFALL_MS = 150
TUMBLE_DPS = 300.0
TUMBLE_MS = 1000


def felt(p):
    """Accelerometer reading: linear acceleration plus gravity turned into the sensor frame."""
    qi, qj, qk, qr = p['quat']
    # third row of the rotation matrix from the sensor frame to the world frame: world up in sensor axes
    gx = 2 * (qi * qk - qr * qj)
    gy = 2 * (qj * qk + qr * qi)
    gz = 1 - 2 * (qi * qi + qj * qj)
    ax, ay, az = p['accel_mps2']
    return math.sqrt((ax + G * gx) ** 2 + (ay + G * gy) ** 2 + (az + G * gz) ** 2)


def runs(packets, test, min_ms):
    out, start, last = [], None, None
    for p in packets:
        if test(p):
            if start is not None and p['up'] - last > 200:
                if last - start >= min_ms:
                    out.append((start, last))
                start = None
            start = p['up'] if start is None else start
            last = p['up']
        elif start is not None:
            if last - start >= min_ms:
                out.append((start, last))
            start = None
    if start is not None and last - start >= min_ms:
        out.append((start, last))
    return out


def phase_at(events, ms):
    state = 'boot-air'
    for e in events:
        if e.get('source') != 'phase' or int(e['first_ms']) > ms:
            continue
        state = {'toair': 'air', 'toground': 'ground', 'bootairend': 'ground'}.get(e['what'], state)
    return state


def height(packets, a, b):
    bins = collections.defaultdict(list)
    for p in packets:
        if a <= p['up'] < b and p['valid_env']:
            bins[p['up'] // 1000].append(p['pressure_alt_m'])
    lows = [min(v) for _, v in sorted(bins.items())]
    return f'{lows[0]:.0f} -> {min(lows):.0f} m' if lows else 'no barometer'


def main():
    for path in sys.argv[1:]:
        print(path)
        for n, b in sorted(flight_review.read_pull(path).items()):
            imu = [p for p in b['packets'] if p['valid_linacc'] and p['valid_quat']]
            for p in imu:
                p['felt'] = felt(p)
            rest = [p['felt'] for p in imu if p['accel'] < 0.3 and p['gyro'] < 2]
            if rest:
                print(f'  boot {n}: felt at rest median {sorted(rest)[len(rest) // 2]:.2f} m/s2 over {len(rest)} readings')
            for name, test, min_ms in (('freefall', lambda p: p['felt'] < FREEFALL_MPS2, FREEFALL_MS),
                                       ('tumbling', lambda p: p['gyro'] >= TUMBLE_DPS, TUMBLE_MS)):
                for a, z in runs(imu, test, min_ms):
                    low = min(p['felt'] for p in imu if a <= p['up'] <= z)
                    print(f'    {name} {a / 1000:.2f}-{z / 1000:.2f} s ({z - a} ms, felt min {low:.1f} m/s2), '
                          f'firmware in {phase_at(b["events"], a)}, barometer lows {height(b["packets"], a - 2000, z + 15000)}')


if __name__ == '__main__':
    main()
