#!/usr/bin/env python3
"""Where a ChipSat came down: the touchdown moment and its GPS position, drawn on satellite imagery.

usage: ./find_landing.py CSID LOG [LOG ...] [--out DIR] [--from LAT,LON]

Reads the ground app's session logs (the radio log, one PKT line per packet heard), keeps one copy of
each of the board's packets with a good CRC, and looks for its last landing:

  touchdown  the first IMU reading of the last spell where the board lies still (linear acceleration
             under 0.3 m/s2 and rotation under 2 deg/s) for at least 20 s. The IMU is the sharpest
             signal: it reads still from the moment the board stops
  release    the first reading in the 30 s before touchdown turning at half the biggest rotation or
             more, when the board left the drone
  ground     the barometer's median in the minute after touchdown, 15 s on, once it has settled. The
             barometer is printed for the fall but not trusted on its own: on board 17 it jumped 34 m
             up at release and read 30-65 m under the ground right after touchdown, with the radio
             transmitting back to back
  position   the GPS fix nearest touchdown, and the median of the fixes in the first minute still,
             with how far they spread

--from is a point to walk from, such as the ground station: the distance and compass bearing to the
landing are printed and drawn.

Writes landing-<csid>.png (Esri World Imagery, north up) and prints the numbers. Needs Pillow and a
network for the imagery; tiles are kept in DIR/tiles so a second run works offline.
"""
import argparse
import datetime
import io
import math
import os
import statistics
import urllib.request

from PIL import Image, ImageDraw, ImageFont

import descent_decode

STILL_ACCEL_MPS2 = 0.3
STILL_GYRO_DPS = 2.0
STILL_FOR_S = 20
RELEASE_LOOKBACK_S = 30
GROUND_AFTER_S = (15, 75)
FIX_WINDOW_S = 60
TILE_URL = 'https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}'


def read_packets(paths, csid):
    """One entry per packet the board sent, at the time the first receiver heard it."""
    seen = {}
    for path in paths:
        for line in open(path, errors='replace'):
            cols = line.rstrip('\n').split('\t')
            if len(cols) < 3 or not cols[2].startswith('PKT,55,'):
                continue
            fields = cols[2].split(',')
            try:
                p = descent_decode.decode_packet(bytes.fromhex(fields[2]))
            except ValueError:
                continue
            if not p['crc_ok'] or p['csid'] != csid:
                continue
            t = int(cols[0]) / 1000
            p['t'] = t
            p['rssi'] = float(fields[3]) if len(fields) > 3 and fields[3] else None
            p['accel'] = math.hypot(*p['accel_mps2'])
            p['gyro'] = math.hypot(*p['gyro_dps'])
            key = (p['counter'], fields[2])
            if key not in seen or t < seen[key]['t']:
                seen[key] = p
    return sorted(seen.values(), key=lambda p: p['t'])


def find_touchdown(packets):
    """The start of the last still spell that lasts STILL_FOR_S, or None."""
    imu = [p for p in packets if p['valid_linacc'] and p['valid_gyro']]
    start = None
    found = None
    for p in imu:
        still = p['accel'] < STILL_ACCEL_MPS2 and p['gyro'] < STILL_GYRO_DPS
        if still and start is None:
            start = p
        elif not still:
            start = None
        if start is not None and p['t'] - start['t'] >= STILL_FOR_S:
            found = start
    return found


def clock(t):
    return datetime.datetime.fromtimestamp(t).strftime('%H:%M:%S.%f')[:-4]


def metres(lat0, lon0, lat, lon):
    return math.hypot((lat - lat0) * 111320, (lon - lon0) * 111320 * math.cos(math.radians(lat0)))


def tile_xy(lat, lon, z):
    n = 256 * 2 ** z
    x = (lon + 180) / 360 * n
    y = (1 - math.log(math.tan(math.radians(lat)) + 1 / math.cos(math.radians(lat))) / math.pi) / 2 * n
    return x, y


def tile(z, x, y, cache):
    path = os.path.join(cache, f'{z}_{x}_{y}.jpg')
    if not os.path.exists(path):
        req = urllib.request.Request(TILE_URL.format(z=z, x=x, y=y), headers={'User-Agent': 'descent-find-landing'})
        with open(path, 'wb') as f:
            f.write(urllib.request.urlopen(req, timeout=20).read())
    return Image.open(path).convert('RGB')


def bearing(lat0, lon0, lat, lon):
    north = (lat - lat0) * 111320
    east = (lon - lon0) * 111320 * math.cos(math.radians(lat0))
    return math.degrees(math.atan2(east, north)) % 360


def draw_map(out, csid, centre, fall, still, touch_fix, start=None, half_m=60, z=19, upscale=3):
    lat0, lon0 = centre
    cache = os.path.join(out, 'tiles')
    os.makedirs(cache, exist_ok=True)
    mpp = 156543.03392 * math.cos(math.radians(lat0)) / 2 ** z
    half = int(half_m / mpp)
    cx, cy = tile_xy(lat0, lon0, z)
    x0, y0, size = int(cx - half), int(cy - half), 2 * half
    img = Image.new('RGB', (size, size))
    for tx in range(x0 // 256, (x0 + size) // 256 + 1):
        for ty in range(y0 // 256, (y0 + size) // 256 + 1):
            img.paste(tile(z, tx, ty, cache), (tx * 256 - x0, ty * 256 - y0))
    img = img.resize((size * upscale, size * upscale), Image.LANCZOS)
    d = ImageDraw.Draw(img, 'RGBA')
    font = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf', 15)
    bold = ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf', 18)

    def px(lat, lon):
        x, y = tile_xy(lat, lon, z)
        return (x - x0) * upscale, (y - y0) * upscale

    def label(xy, text, colour):
        box = d.textbbox(xy, text, font=font)
        d.rectangle((box[0] - 4, box[1] - 3, box[2] + 4, box[3] + 3), fill=(0, 0, 0, 180))
        d.text(xy, text, font=font, fill=colour)

    per_m = upscale / mpp
    c = px(lat0, lon0)
    for r in (5, 10, 20):
        d.ellipse((c[0] - r * per_m, c[1] - r * per_m, c[0] + r * per_m, c[1] + r * per_m), outline=(255, 255, 255, 200), width=2)
        label((c[0] + r * per_m * 0.71 + 4, c[1] - r * per_m * 0.71 - 10), f'{r} m', (255, 255, 255))
    if len(fall) > 1:
        d.line([px(p['lat_deg'], p['lon_deg']) for p in fall], fill=(0, 230, 255, 230), width=4)
        label(px(fall[0]['lat_deg'], fall[0]['lon_deg']), f'release {clock(fall[0]["t"])}', (120, 240, 255))
    for p in still:
        x, y = px(p['lat_deg'], p['lon_deg'])
        d.ellipse((x - 4, y - 4, x + 4, y + 4), fill=(255, 200, 0, 220), outline=(0, 0, 0))
    d.line((c[0] - 16, c[1], c[0] + 16, c[1]), fill=(255, 255, 255), width=3)
    d.line((c[0], c[1] - 16, c[0], c[1] + 16), fill=(255, 255, 255), width=3)
    x, y = px(touch_fix['lat_deg'], touch_fix['lon_deg'])
    d.regular_polygon((x, y, 13), 5, fill=(255, 0, 60), outline=(255, 255, 255))
    label((x + 16, y - 26), f'touchdown fix {clock(touch_fix["t"])}', (255, 140, 160))
    label((c[0] + 14, c[1] + 12), f'median of {len(still)} fixes lying still', (255, 230, 120))
    if start is not None:
        s = px(*start)
        d.line((s, c), fill=(120, 255, 120, 230), width=3)
        d.ellipse((s[0] - 10, s[1] - 10, s[0] + 10, s[1] + 10), fill=(120, 255, 120), outline=(0, 0, 0))
        label((s[0] + 14, s[1] - 10), f'start: {metres(*start, lat0, lon0):.0f} m at {bearing(*start, lat0, lon0):.0f} deg', (160, 255, 160))
    W = img.size[0]
    d.rectangle((16, W - 40, 16 + 10 * per_m, W - 32), fill=(255, 255, 255))
    label((16, W - 64), '10 m', (255, 255, 255))
    d.rectangle((0, 0, W, 66), fill=(0, 0, 0, 180))
    d.text((14, 8), f'Board {csid}: where it came down', font=bold, fill=(255, 255, 255))
    d.text((14, 38), f'centre {lat0:.6f}, {lon0:.6f}   north up   Esri World Imagery', font=font, fill=(220, 220, 220))
    label((W - 470, W - 60), 'cyan: the fall   yellow: fixes while still   star: touchdown', (255, 255, 255))
    path = os.path.join(out, f'landing-{csid}.png')
    img.save(path)
    return path


def main():
    ap = argparse.ArgumentParser(description='Find where a ChipSat came down, from the ground logs.')
    ap.add_argument('csid', type=int)
    ap.add_argument('logs', nargs='+')
    ap.add_argument('--out', default='.')
    ap.add_argument('--from', dest='start', help='LAT,LON to walk from')
    a = ap.parse_args()

    packets = read_packets(a.logs, a.csid)
    print(f'board {a.csid}: {len(packets)} packets with a good CRC, {clock(packets[0]["t"])} to {clock(packets[-1]["t"])}')
    touch = find_touchdown(packets)
    if touch is None:
        raise SystemExit(f'no spell of {STILL_FOR_S} s lying still in the IMU readings')
    before = [p for p in packets if touch['t'] - RELEASE_LOOKBACK_S <= p['t'] < touch['t'] and p['valid_gyro']]
    # the first big turn, not the biggest: the board keeps tumbling after it leaves the drone
    peak = max((p['gyro'] for p in before), default=0)
    release = next((p for p in before if p['gyro'] >= peak / 2), touch)
    print(f'release   {clock(release["t"])}  rotation {release["gyro"]:.0f} deg/s')
    print(f'touchdown {clock(touch["t"])}  first still IMU reading, {touch["t"] - release["t"]:.1f} s after release')

    settled = [p['pressure_alt_m'] for p in packets
               if p['valid_env'] and GROUND_AFTER_S[0] <= p['t'] - touch['t'] <= GROUND_AFTER_S[1]]
    ground = statistics.median(settled) if settled else None
    if ground is not None:
        print(f'ground    barometer {ground:.1f} m, median of {len(settled)} readings {GROUND_AFTER_S[0]}-{GROUND_AFTER_S[1]} s after touchdown')
        print('barometer through the fall, above that ground (an offset shows as a jump that the IMU does not):')
        for p in packets:
            if release['t'] - 3 <= p['t'] <= touch['t'] + 15 and p['valid_env']:
                print(f'  {clock(p["t"])}  {p["pressure_alt_m"] - ground:+7.1f} m   accel {p["accel"]:5.1f} m/s2  rotation {p["gyro"]:4.0f} deg/s')

    fixes = [p for p in packets if p['valid_gnss'] and p['lat_deg'] != 0]
    fall = [p for p in fixes if release['t'] <= p['t'] <= touch['t']]
    still = [p for p in fixes if touch['t'] <= p['t'] <= touch['t'] + FIX_WINDOW_S]
    touch_fix = min(fixes, key=lambda p: abs(p['t'] - touch['t']))
    print(f'touchdown fix {clock(touch_fix["t"])}  {touch_fix["lat_deg"]:.6f}, {touch_fix["lon_deg"]:.6f}  (gps altitude {touch_fix["gps_alt_m"]:.0f} m)')
    if not still:
        raise SystemExit('no GPS fix in the minute after touchdown')
    lat0 = statistics.median(p['lat_deg'] for p in still)
    lon0 = statistics.median(p['lon_deg'] for p in still)
    spread = sorted(metres(lat0, lon0, p['lat_deg'], p['lon_deg']) for p in still)
    print(f'lying still {lat0:.6f}, {lon0:.6f}  median of {len(still)} fixes in the first {FIX_WINDOW_S} s, '
          f'half within {spread[len(spread) // 2]:.1f} m, all within {spread[-1]:.1f} m; '
          f'{metres(lat0, lon0, touch_fix["lat_deg"], touch_fix["lon_deg"]):.1f} m from the touchdown fix')
    last = fixes[-1]
    print(f'last fix  {clock(last["t"])}  {last["lat_deg"]:.6f}, {last["lon_deg"]:.6f}, '
          f'{metres(lat0, lon0, last["lat_deg"], last["lon_deg"]):.1f} m away; last packet {clock(packets[-1]["t"])}, '
          f'cell {packets[-1]["cell_v"]:.2f} V')
    start = tuple(float(v) for v in a.start.split(',')) if a.start else None
    if start is not None:
        print(f'from {start[0]:.6f}, {start[1]:.6f}: {metres(*start, lat0, lon0):.1f} m at {bearing(*start, lat0, lon0):.0f} deg (0 north, 90 east)')
    print('map', draw_map(a.out, a.csid, (lat0, lon0), fall, still, touch_fix, start))


if __name__ == '__main__':
    main()
