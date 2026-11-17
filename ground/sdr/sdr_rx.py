#!/usr/bin/env python3
# SDR receiver for the ground app: decodes SF10, SF11 and SF12 at once on 915.0 MHz and prints the
# receiver lines (PKT, ERR, HB, #DG,RX) on stdout. The app starts it from descent-ground.conf:
#
#   sdr = python3 /path/to/ground/sdr/sdr_rx.py --decoder /path/to/file_lora
#
# rtl_sdr runs at 1 MS/s tuned 250 kHz above the channel, which keeps the dongle's DC notch and
# its spur at centre+190 kHz out of it. The channel is moved to 0 Hz, cut to 250 kS/s and fed to
# one rxx_lora file_lora per spreading factor through named pipes. file_lora needs
# rxx_lora-ldro.patch, or every SF11 and SF12 payload fails its CRC.
import argparse
import base64
import fcntl
import json
import os
import queue
import random
import signal
import subprocess
import sys
import tempfile
import threading
import time

import numpy as np

VERSION = '0.2.0'
SFS = (10, 11, 12)
CHANNEL_HZ = 915_000_000
TUNE_HZ = CHANNEL_HZ + 250_000
RATE = 1_000_000
DECIM = 4
CHUNK = 1 << 18                       # bytes of u8 IQ, 131 ms
ID_FILE = os.path.expanduser('~/.config/descent/sdr_rx_id')


def lowpass(n, cutoff):
    k = np.arange(n) - (n - 1) / 2
    h = np.sinc(2 * cutoff / RATE * k) * np.hamming(n)
    return (h / h.sum()).astype(np.float32)


TAPS = lowpass(64, 75e3)
QUARTER_TURNS = np.array([1, 1j, -1, -1j], np.complex64)

# What file_lora warns about once it has a LoRa header, as the spec's ERR codes
DECODER_WARNINGS = {
    'header checksum invalid': -24,
    'frame ended after': -1,
}

# rtl_sdr's last words, as the FATAL code and what to tell the operator
DONGLE_FAILURES = [
    ('No supported devices found', 2, 'No RTL-SDR plugged in'),
    ('usb_claim_interface error', 3, 'The RTL-SDR is held by another program'),
    ('Failed to open rtlsdr device', 3, 'The RTL-SDR could not be opened: held by another program, or no permission'),
    ('Library error', 4, 'The RTL-SDR went away (unplugged, or the USB hub dropped)'),
    ('samples lost, exiting', 4, 'The RTL-SDR went away (unplugged, or the USB hub dropped)'),
]

started = time.monotonic()


def uptime_ms():
    return int((time.monotonic() - started) * 1000)


def receiver_id():
    if not os.path.exists(ID_FILE):
        os.makedirs(os.path.dirname(ID_FILE), exist_ok=True)
        with open(ID_FILE, 'w') as f:
            f.write('%06X\n' % random.getrandbits(24))
    return open(ID_FILE).read().strip()


class FrontEnd:
    # The channel sits at -RATE/4, so moving it to 0 Hz is a multiply by j**n.
    def __init__(self, level):
        self.n = 0
        self.tail = np.zeros(len(TAPS) - 1, np.complex64)
        self.level = level

    def push(self, raw):
        u = np.frombuffer(raw, np.uint8)
        clipped = np.count_nonzero((u == 0) | (u == 255))
        x = (u.astype(np.float32) - 127.5) / 127.5
        x = (x[0::2] + 1j * x[1::2]).astype(np.complex64)
        x *= QUARTER_TURNS[(self.n + np.arange(len(x))) % 4]
        y = np.concatenate([self.tail, x])
        self.tail = y[-(len(TAPS) - 1):]
        first = (-self.n) % DECIM
        self.n += len(x)
        out = np.convolve(y, TAPS, mode='valid')[first::DECIM].astype(np.complex64)
        with self.level['lock']:
            self.level['clipped'] += clipped
            self.level['bytes'] += len(u)
            self.level['floors'].append(float(np.median(np.abs(out) ** 2)))
        return out


def read_dongle(gain, raw_q, counts):
    cmd = ['rtl_sdr', '-f', str(TUNE_HZ), '-s', str(RATE)]
    if gain != 'auto':
        cmd += ['-g', gain]
    proc = subprocess.Popen(cmd + ['-'], stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
    fcntl.fcntl(proc.stdout, fcntl.F_SETPIPE_SZ, 1 << 20)
    counts['children'].append(proc)

    def relay_stderr():
        for raw in proc.stderr:
            line = raw.decode(errors='replace').rstrip()
            print('rtl_sdr: ' + line, file=sys.stderr, flush=True)
            for text, code, why in DONGLE_FAILURES:
                if text in line and counts['fatal'] is None:
                    counts['fatal'] = (code, why)
    threading.Thread(target=relay_stderr, daemon=True).start()

    while True:
        raw = proc.stdout.read(CHUNK)
        if not raw:
            break
        if counts['first'] is None:
            counts['first'] = time.monotonic()
        counts['received'] += len(raw) // 2
        try:
            raw_q.put_nowait(raw)
        except queue.Full:
            counts['dropped'] += len(raw) // 2
    proc.wait()
    raw_q.put(None)


def read_file(path, raw_q, counts):
    # paced to real time, so the HB, the header and the app see what a dongle would give them
    with open(path, 'rb') as f:
        while raw := f.read(CHUNK):
            counts['received'] += len(raw) // 2
            raw_q.put(raw)
            ahead = counts['received'] / RATE - (time.monotonic() - started)
            if ahead > 0:
                time.sleep(ahead)
    raw_q.put(None)


def read_decoder(sf, proc, events):
    for line in proc.stdout:
        events.put((sf, json.loads(line)))
    events.put((sf, None))


def read_decoder_warnings(sf, proc, events):
    for line in proc.stderr:
        for text, code in DECODER_WARNINGS.items():
            if text in line:
                events.put((sf, {'err': code}))


def line_for(sf, frame):
    if 'err' in frame:
        return f"ERR,{frame['err']},,,sf={sf}"
    data = base64.b64decode(frame['packet']).hex().upper()
    snr = '%.2f' % frame['snr_db']
    if frame['crc_present'] and frame['crc_valid']:
        return f"PKT,{frame['len']},{data},,{snr},{round(frame['cfo_hz'])},sf={sf}"
    code = -7 if frame['crc_present'] else -1
    return f'ERR,{code},,{snr},sf={sf},hex={data}'


def level_fields(level):
    with level['lock']:
        clip = 100 * level['clipped'] / level['bytes'] if level['bytes'] else 0.0
        # A packet outlasts a chunk, so the quietest chunk stands for the floor. A board sending
        # back to back for the whole 5 s raises it to that board's level.
        floor = 10 * np.log10(min(level['floors'])) if level['floors'] else None
        level.update(clipped=0, bytes=0, floors=[])
    # sat=1 is this program's own flag: at least one sample sat on the ADC's rail since the last HB
    return f',clip={clip:.2f},sat={int(clip > 0)}' + ('' if floor is None else f',floor={floor:.1f}')


def write_lines(events, counts, level, header, live, done):
    ok = errors = 0
    next_header = next_hb = 0.0
    open_decoders = len(SFS)

    def put(line):
        try:
            sys.stdout.write(line + '\n')
            sys.stdout.flush()
        except BrokenPipeError:
            os.kill(os.getpid(), signal.SIGTERM)

    def heartbeat():
        ovf = counts['dropped']
        if live and counts['first'] is not None:
            # rtl_sdr loses samples without saying so; count what the clock says should
            # have come, less half a second of buffering, and never let the count go down
            late = (time.monotonic() - counts['first']) * RATE - counts['received'] - RATE / 2
            counts['late'] = max(counts['late'], int(late))
            ovf += counts['late']
        put(f'HB,{uptime_ms()},{ok},{errors},ovf={ovf},drop=0' + level_fields(level))

    while open_decoders:
        now = time.monotonic()
        if now >= next_header:
            put(header)
            next_header = now + 30
        if now >= next_hb:
            heartbeat()
            next_hb = now + 5
        try:
            sf, frame = events.get(timeout=0.5)
        except queue.Empty:
            continue
        if frame is None:
            open_decoders -= 1
            continue
        line = line_for(sf, frame)
        if line.startswith('PKT'):
            ok += 1
        else:
            errors += 1
        put(line)
    heartbeat()
    done.set()


def main():
    ap = argparse.ArgumentParser(description='SDR receiver for SF10-12 at 915.0 MHz')
    ap.add_argument('--decoder', required=True, help='path to rxx_lora file_lora, LDRO-patched')
    ap.add_argument('--gain', default='auto', help='rtl_sdr tuner gain in dB, or auto for its AGC')
    ap.add_argument('--cu8', help='play a rtl_sdr recording made at 915.25 MHz, 1 MS/s, in real time')
    args = ap.parse_args()

    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    header = (f'#DG,RX,v1,id={receiver_id()},f=915.0,bw=125.0,cr=7,sync=0x12,pre=8,kind=sdr,'
              f'sfs={"+".join(map(str, SFS))},gain={args.gain},fw={VERSION}')
    counts = {'received': 0, 'dropped': 0, 'late': 0, 'first': None, 'fatal': None, 'children': []}
    level = {'lock': threading.Lock(), 'clipped': 0, 'bytes': 0, 'floors': []}
    raw_q = queue.Queue(maxsize=64)
    events = queue.Queue()
    done = threading.Event()
    pipes = tempfile.mkdtemp(prefix='sdr_rx_')
    try:
        decoders = []
        for sf in SFS:
            fifo = os.path.join(pipes, f'sf{sf}.cf32')
            os.mkfifo(fifo)
            proc = subprocess.Popen(
                [args.decoder, '-s', str(sf), '-o', '2', '-w', '0x12', '--report-bad', '-q', '1',
                 '--center-freq', str(CHANNEL_HZ), '--bw', '125e3', fifo],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            counts['children'].append(proc)
            threading.Thread(target=read_decoder, args=(sf, proc, events), daemon=True).start()
            threading.Thread(target=read_decoder_warnings, args=(sf, proc, events), daemon=True).start()
            decoders.append(open(fifo, 'wb'))
        for f in decoders:
            fcntl.fcntl(f, fcntl.F_SETPIPE_SZ, 1 << 20)

        threading.Thread(target=write_lines, args=(events, counts, level, header, not args.cu8, done),
                         daemon=True).start()
        if args.cu8:
            source = threading.Thread(target=read_file, args=(args.cu8, raw_q, counts), daemon=True)
        else:
            source = threading.Thread(target=read_dongle, args=(args.gain, raw_q, counts),
                                      daemon=True)
        source.start()

        front = FrontEnd(level)
        while (raw := raw_q.get()) is not None:
            iq = front.push(raw).tobytes()
            for f in decoders:
                f.write(iq)
        for f in decoders:
            f.close()
        done.wait()
        if not args.cu8:
            code, why = counts['fatal'] or (1, 'rtl_sdr stopped without saying why')
            print(f'#DG,FATAL,{code}', flush=True)
            print(why, file=sys.stderr, flush=True)
            sys.exit(1)
    finally:
        for proc in counts['children']:
            proc.kill()
        for name in os.listdir(pipes):
            os.unlink(os.path.join(pipes, name))
        os.rmdir(pipes)


if __name__ == '__main__':
    main()
