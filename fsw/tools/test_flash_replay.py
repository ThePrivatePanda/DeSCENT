#!/usr/bin/env python3
"""Laptop tests for the dump parser. python3 test_flash_replay.py

Covers the raw 64-byte record format, the old readable one that captures on disk still use, the slot
CRC rejecting a damaged record, and dating from a boot's anchors when one of them is wrong.
"""

import unittest

import flash_replay as fr
import flash_to_ground as fg

PACKET = bytes(range(55))


def slot(kind=0xA5, boot=0, ms=0, payload=PACKET, epoch=0, nano=0, bits=0x07, acc=25, breakCrc=False):
    """One 64 byte record as the board writes it"""
    raw = bytearray(64)
    raw[0] = kind
    raw[1] = boot & 0xFF
    raw[2] = boot >> 8
    raw[3:7] = ms.to_bytes(4, "little")
    if kind == 0xA6:
        raw[7:11] = epoch.to_bytes(4, "little")
        raw[11:15] = nano.to_bytes(4, "little", signed=True)
        raw[15] = bits
        raw[16:20] = acc.to_bytes(4, "little")
    else:
        raw[7:7 + len(payload)] = payload
    crc = fr.crc16(bytes(raw[:62]))
    raw[62] = crc & 0xFF
    raw[63] = crc >> 8
    if breakCrc:
        raw[3] ^= 0x10        # a damaged uptime, which is the field the old format left unchecked
    return bytes(raw)


def rawLine(index, record):
    return f"raw {index} {index * 64:06X} {record.hex().upper()}"


def found(count):
    return f"found {count} records, {count} good (0 time anchors), 0 bad, from 1 boots"


class RawFormat(unittest.TestCase):
    def test_reads_a_record(self):
        records, anchors = fr.parse_dump([rawLine(0, slot(boot=3, ms=1500)), found(1)])
        self.assertEqual(len(records), 1)
        self.assertEqual(anchors, [])
        self.assertEqual(records[0]["boot"], 3)
        self.assertEqual(records[0]["ms"], 1500)
        self.assertEqual(records[0]["index"], 0)
        self.assertEqual(records[0]["address"], 0)

    def test_carries_the_address(self):
        records, _ = fr.parse_dump([rawLine(7, slot()), found(1)])
        self.assertEqual(records[0]["address"], 7 * 64)

    def test_reads_an_anchor(self):
        _, anchors = fr.parse_dump([rawLine(0, slot(kind=0xA6, boot=2, ms=900, epoch=1790000000,
                                                    nano=216000000, acc=31)), found(1)])
        self.assertEqual(len(anchors), 1)
        self.assertEqual(anchors[0]["epoch"], 1790000000)
        self.assertEqual(anchors[0]["nano"], 216000000)
        self.assertEqual(anchors[0]["bits"], 0x07)
        self.assertEqual(anchors[0]["acc_ns"], 31)

    def test_a_damaged_uptime_is_rejected(self):
        """the whole point of the format: the old one left boot and uptime outside any check"""
        records, _ = fr.parse_dump([rawLine(0, slot(ms=1000)),
                                    rawLine(1, slot(ms=2000, breakCrc=True)),
                                    rawLine(2, slot(ms=3000)), found(3)])
        self.assertEqual([r["ms"] for r in records], [1000, 3000])

    def test_a_line_that_lost_its_shape_is_rejected(self):
        records, _ = fr.parse_dump([rawLine(0, slot()), "raw 1 0040 notevenhex", found(2)])
        self.assertEqual(len(records), 1)

    def test_an_unknown_record_type_is_not_called_damage(self):
        """an intact record of a type we do not know is the tool being old, not the wire being bad"""
        import io, contextlib
        said = io.StringIO()
        with contextlib.redirect_stdout(said):
            records, anchors = fr.parse_dump([rawLine(0, slot(kind=0xA9)), rawLine(1, slot()),
                                              found(2)])
        self.assertEqual(len(records), 1)
        self.assertEqual(anchors, [])
        self.assertIn("type 0xA9", said.getvalue())
        self.assertNotIn("could not be read", said.getvalue())


class OldFormat(unittest.TestCase):
    def test_still_reads_captures_on_disk(self):
        hexed = " ".join(f"{b:02X}" for b in PACKET)
        records, _ = fr.parse_dump(["record 4 boot 1 ms 250", hexed, found(1)])
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["boot"], 1)
        self.assertEqual(records[0]["ms"], 250)

    def test_still_reads_an_old_anchor_line(self):
        _, anchors = fr.parse_dump(
            ["time 9 boot 1 ms 400 epoch 1790000000 nano 0 bits 07 accns 25", found(0)])
        self.assertEqual(len(anchors), 1)
        self.assertEqual(anchors[0]["acc_ns"], 25)


class Dating(unittest.TestCase):
    def anchorsAndRecords(self, anchorMs):
        lines = []
        for i, (ms, epoch) in enumerate(anchorMs):
            lines.append(rawLine(i, slot(kind=0xA6, boot=1, ms=ms, epoch=epoch)))
        for i, ms in enumerate((15000, 25000), start=50):
            lines.append(rawLine(i, slot(boot=1, ms=ms)))
        lines.append(found(len(lines)))
        return fr.parse_dump(lines)

    def test_one_wrong_anchor_moves_nothing(self):
        """the median offset of a boot's anchors, not the nearest one"""
        base = 1790000000
        good = [(10000, base + 10), (20000, base + 20), (30000, base + 30)]
        records, _ = self.anchorsAndRecords(good + [(999000, base + 30)])
        self.assertEqual(records[0]["utc"], base + 15)
        self.assertEqual(records[1]["utc"], base + 25)

    def test_boots_without_time_are_laid_from_1970(self):
        lines = [rawLine(0, slot(boot=0, ms=1000)), rawLine(1, slot(boot=0, ms=2000)),
                 rawLine(2, slot(boot=1, ms=500)), found(3)]
        records, anchors = fr.parse_dump(lines)
        dated, dropped, basis = fg.date_records(records, anchors, None, None)
        self.assertEqual(basis, "none")
        self.assertEqual(dropped, [])
        byBoot = {}
        for record in dated:
            byBoot.setdefault(record["boot"], []).append(record["unix_ms"])
        self.assertEqual(min(byBoot[0]), 0)
        # boot 1 starts after boot 0 ends, so the two cannot interleave in time order
        self.assertGreater(min(byBoot[1]), max(byBoot[0]))


class GroundLog(unittest.TestCase):
    def test_one_source_and_one_boot_marker_per_boot(self):
        lines = [rawLine(0, slot(boot=0, ms=0)), rawLine(1, slot(boot=1, ms=0)), found(2)]
        records, anchors = fr.parse_dump(lines)
        dated, _, basis = fg.date_records(records, anchors, None, None)
        head, body = fg.to_lines(dated, anchors, "flash", "dump.txt", basis or "none")
        self.assertEqual(len(head), 2)
        self.assertEqual(sum(1 for l in body if "#DG,BOOT" in l), 2)
        self.assertEqual(sum(1 for l in body if l.count("PKT,") == 1), 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
