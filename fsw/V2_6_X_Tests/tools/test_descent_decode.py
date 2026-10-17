"""Unit tests for descent_decode.py.

Run with:
    python3 -m unittest discover -s tools -p 'test_*.py' -v
"""

import contextlib
import io
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import descent_decode as dd  # noqa: E402


# Test vector from the software report, section 8.
VECTOR_HEX = (
    "5C 57 4C 19 50 BD 66 D2 90 D0 03 00 B4 5F 00 00 "
    "7B 00 38 FE D4 03 0A 00 EC FF 1E 00 FA 00 83 FF "
    "F4 01 00 00 00 00 00 00 FF 7F 29 09 94 27 C6 11 "
    "2A 00 09 96 FF 93 93"
)
VECTOR = bytes.fromhex(VECTOR_HEX)


class TestCrc(unittest.TestCase):
    def test_check_value(self):
        # Standard CRC-16/CCITT-FALSE check value for "123456789".
        self.assertEqual(dd.crc16_ccitt_false(b"123456789"), 0x29B1)

    def test_empty(self):
        self.assertEqual(dd.crc16_ccitt_false(b""), 0xFFFF)

    def test_vector_crc(self):
        self.assertEqual(dd.crc16_ccitt_false(VECTOR[:53]), 0x9393)


class TestDecodePacket(unittest.TestCase):
    def test_length(self):
        self.assertEqual(len(VECTOR), 55)

    def test_vector_values(self):
        p = dd.decode_packet(VECTOR)
        self.assertAlmostEqual(p["lat_deg"], 42.44335, places=7)
        self.assertAlmostEqual(p["lon_deg"], -76.50188, places=7)
        self.assertAlmostEqual(p["gps_alt_m"], 250.000, places=3)
        self.assertAlmostEqual(p["pressure_alt_m"], 245.00, places=2)
        for got, want in zip(p["accel_mps2"], (1.23, -4.56, 9.80)):
            self.assertAlmostEqual(got, want, places=6)
        for got, want in zip(p["gyro_dps"], (1.0, -2.0, 3.0)):
            self.assertAlmostEqual(got, want, places=6)
        for got, want in zip(p["mag_ut"], (25.0, -12.5, 50.0)):
            self.assertAlmostEqual(got, want, places=6)
        for got, want in zip(p["quat"], (0.0, 0.0, 0.0, 1.0)):
            self.assertAlmostEqual(got, want, places=4)
        self.assertAlmostEqual(p["temp_c"], 23.45, places=6)
        self.assertAlmostEqual(p["pressure_hpa"], 1013.2, places=6)
        self.assertAlmostEqual(p["humidity_pct"], 45.50, places=6)
        self.assertEqual(p["counter"], 42)
        self.assertEqual(p["csid"], 9)
        self.assertAlmostEqual(p["cell_v"], 4.00, places=6)   # byte 150: 75 % x2 then, 4.00 V now
        self.assertEqual(p["validity"], 0xFF)
        self.assertEqual(p["crc_rx"], 0x9393)
        self.assertEqual(p["crc_calc"], 0x9393)
        self.assertTrue(p["crc_ok"])

    def test_validity_flags_all_set(self):
        p = dd.decode_packet(VECTOR)
        for name in dd.VALIDITY_FLAGS:
            self.assertTrue(p[name], name)

    def test_validity_flags_partial(self):
        # Set validity byte to 0b0101_0010 (gyro, gnss, env) and fix up CRC.
        body = bytearray(VECTOR[:53])
        body[52] = 0x52
        crc = dd.crc16_ccitt_false(bytes(body))
        pkt = bytes(body) + crc.to_bytes(2, "little")
        p = dd.decode_packet(pkt)
        self.assertTrue(p["crc_ok"])
        self.assertEqual(p["validity"], 0x52)
        self.assertFalse(p["valid_linacc"])
        self.assertTrue(p["valid_gyro"])
        self.assertFalse(p["valid_mag"])
        self.assertFalse(p["valid_quat"])
        self.assertTrue(p["valid_gnss"])
        self.assertFalse(p["valid_soc"])
        self.assertTrue(p["valid_env"])
        self.assertFalse(p["all_fresh"])

    def test_wrong_length_rejected(self):
        for n in (0, 54, 56, 39):
            with self.subTest(n=n):
                with self.assertRaisesRegex(ValueError, "55"):
                    dd.decode_packet(bytes(n))

    def test_corrupted_crc(self):
        bad = bytearray(VECTOR)
        bad[53] ^= 0x01
        p = dd.decode_packet(bytes(bad))
        self.assertFalse(p["crc_ok"])
        self.assertEqual(p["crc_rx"], 0x9392)
        self.assertEqual(p["crc_calc"], 0x9393)

    def test_corrupted_body(self):
        bad = bytearray(VECTOR)
        bad[10] ^= 0x80
        p = dd.decode_packet(bytes(bad))
        self.assertFalse(p["crc_ok"])

    def test_negative_saturated_values(self):
        body = bytearray(53)
        # lat = INT32_MIN, accel x = INT16_MIN, quat real = -32767
        body[0:4] = (-2**31).to_bytes(4, "little", signed=True)
        body[16:18] = (-32768).to_bytes(2, "little", signed=True)
        body[40:42] = (-32767).to_bytes(2, "little", signed=True)
        pkt = bytes(body) + dd.crc16_ccitt_false(bytes(body)).to_bytes(2, "little")
        p = dd.decode_packet(pkt)
        self.assertTrue(p["crc_ok"])
        self.assertAlmostEqual(p["lat_deg"], -214.7483648, places=7)
        self.assertAlmostEqual(p["accel_mps2"][0], -327.68, places=6)
        self.assertAlmostEqual(p["quat"][3], -1.0, places=6)


class TestParseHex(unittest.TestCase):
    def test_plain(self):
        self.assertEqual(dd.parse_hex(VECTOR_HEX), VECTOR)

    def test_0x_prefix_and_commas(self):
        s = ", ".join("0x%02x" % b for b in VECTOR)
        self.assertEqual(dd.parse_hex(s), VECTOR)

    def test_no_spaces(self):
        self.assertEqual(dd.parse_hex(VECTOR.hex()), VECTOR)

    def test_multiline(self):
        lines = [VECTOR_HEX[i:i + 48] for i in range(0, len(VECTOR_HEX), 48)]
        self.assertEqual(dd.parse_hex("\n".join(lines)), VECTOR)

    def test_garbage(self):
        with self.assertRaises(ValueError):
            dd.parse_hex("5C 57 ZZ")


def _firmware_dump(pkt):
    """Exactly what ChipSatTelemetry::printPacketHex emits."""
    return "Packet size: 55 bytes\n" + " ".join("%02X" % b for b in pkt) + "\n"


class TestLogParser(unittest.TestCase):
    def _make_log(self):
        good = VECTOR
        bad_crc = bytearray(VECTOR)
        bad_crc[54] ^= 0xFF
        bad_crc = bytes(bad_crc)
        short = VECTOR[:40]

        # Second good packet with counter 43 and a different CSID.
        body = bytearray(VECTOR[:53])
        body[48:50] = (43).to_bytes(2, "little")
        body[50] = 3
        good2 = bytes(body) + dd.crc16_ccitt_false(bytes(body)).to_bytes(2, "little")

        log = io.StringIO()
        log.write("Boot OK\n")
        log.write("GPS: lat 42.4433500 lon -76.5018800\n")
        log.write("  X: 1.23 m/s^2\n")
        log.write(_firmware_dump(good))
        log.write("Sensor validity: 0xFF\n")
        log.write("  Linear acceleration valid: 1\n")
        log.write("Telemetry packet transmitted\n")
        log.write(_firmware_dump(bad_crc))
        log.write("Packet size: 55 bytes\n")
        log.write(" ".join("%02X" % b for b in short) + "\n")
        log.write("Telemetry packet transmitted\n")
        # T-Beam receiver style line.
        log.write("Received 55 bytes.\n")
        log.write("Raw bytes: " + " ".join("%02X" % b for b in good2) + "\n")
        log.write("\n========== RECEIVED TELEMETRY ==========\n")
        # Multi-line dump, 16 bytes per line, with 0x prefixes.
        log.write("Multi-line dump follows\n")
        for i in range(0, 55, 16):
            log.write(" ".join("0x%02x" % b for b in good[i:i + 16]) + "\n")
        log.write("done\n")
        return log.getvalue(), good, good2

    def test_extract_packets(self):
        text, good, good2 = self._make_log()
        runs = dd.extract_hex_runs(text)
        lengths = [len(r) for _, r in runs]
        self.assertEqual(lengths, [55, 55, 40, 55, 55])
        self.assertEqual(runs[0][1], good)
        self.assertEqual(runs[3][1], good2)
        self.assertEqual(runs[4][1], good)

    def test_parse_log_skips_bad(self):
        text, good, good2 = self._make_log()
        warnings = []
        rows = dd.parse_log_text(text, warn=warnings.append)
        self.assertEqual([r["counter"] for r in rows], [42, 43, 42])
        self.assertEqual([r["csid"] for r in rows], [9, 3, 9])
        self.assertEqual(len(warnings), 2)
        self.assertTrue(any("CRC" in w for w in warnings), warnings)
        self.assertTrue(any("40" in w for w in warnings), warnings)

    def test_cli_log_csv(self):
        text, good, good2 = self._make_log()
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "capture.txt")
            with open(path, "w") as f:
                f.write(text)
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = dd.main([path, "--csv"])
            self.assertEqual(rc, 0)
            lines = out.getvalue().strip().splitlines()
            self.assertEqual(len(lines), 4)  # header + 3 rows
            self.assertTrue(lines[0].startswith("line,"), lines[0])
            self.assertIn("42.44335", lines[1])
            self.assertIn("warning", err.getvalue().lower())

    def test_cli_log_table(self):
        text, good, good2 = self._make_log()
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "capture.txt")
            with open(path, "w") as f:
                f.write(text)
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = dd.main([path])
            self.assertEqual(rc, 0)
            self.assertIn("42.44335", out.getvalue())
            self.assertIn("-76.50188", out.getvalue())


class TestCli(unittest.TestCase):
    def test_hex_arg(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = dd.main(["--hex", VECTOR_HEX])
        self.assertEqual(rc, 0)
        text = out.getvalue()
        self.assertIn("42.4433500", text)
        self.assertIn("0x9393", text)
        self.assertIn("OK", text)

    def test_hex_arg_bad_crc(self):
        bad = bytearray(VECTOR)
        bad[53] ^= 0x01
        hexs = " ".join("%02X" % b for b in bad)
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = dd.main(["--hex", hexs])
        self.assertNotEqual(rc, 0)

    def test_hex_arg_wrong_length(self):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = dd.main(["--hex", "5C 57 4C"])
        self.assertNotEqual(rc, 0)
        self.assertIn("55", err.getvalue())

    def test_selftest(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = dd.main(["--selftest"])
        self.assertEqual(rc, 0)
        self.assertIn("PASS", out.getvalue())


class TestSerialLogPrefix(unittest.TestCase):
    """tools/serial_log.py writes 'HH:MM:SS.mmm  <text>' (two spaces, no arrow)."""
    def test_serial_log_prefix_is_stripped(self):
        vec = ("5C 57 4C 19 50 BD 66 D2 90 D0 03 00 B4 5F 00 00 7B 00 38 FE D4 03 0A 00 EC FF 1E 00 FA 00 83 FF "
               "F4 01 00 00 00 00 00 00 FF 7F 29 09 94 27 C6 11 2A 00 09 96 FF 93 93")
        log = "19:54:56.286  Packet size: 55 bytes\n19:54:56.286  " + vec + "\n19:54:56.286  Sensor validity: 0xFF\n"
        rows = dd.parse_log_text(log, warn=lambda m: None)
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["counter"], 42)


if __name__ == "__main__":
    unittest.main()
