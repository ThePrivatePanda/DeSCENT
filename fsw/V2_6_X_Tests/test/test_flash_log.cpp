// FlashLog.cpp on the laptop, against a chip kept in RAM that behaves like NOR flash: erase sets FF, a program can
// only clear bits, reads and programs are refused while an erase runs
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src/app test/test_flash_log.cpp ../V2_6_X_Code/src/app/{FlashLog,Telemetry}.cpp -o /tmp/flashlog && /tmp/flashlog
#include <cstdio>
#include <cstring>
#include <algorithm>
#include <vector>
#include "FlashLog.h"

using ChipSatApp::FlashLog;
using ChipSatApp::LogRecord;
using ChipSatApp::kLogRecordCount;
using ChipSatApp::kLogRecordSize;
using ChipSatApp::kLogRecordsPerSector;
using ChipSatPlatform::Status;
using ChipSatTelemetry::TelemetryPacket;

static uint32_t fakeNow = 0;

// the chip
static std::vector<uint8_t> memory(ChipSatConstants::kFlashSize, 0xFF);
static uint32_t eraseMs = 36;   // what the bench measured
static uint32_t eraseDoneMs = 0;
static bool eraseStuck = false;
static std::vector<uint32_t> erased;   // sector addresses, in order
static int programsToFail = 0;         // these fail without writing anything
static int programs = 0;

namespace ChipSatPlatform
{
uint32_t System::nowMs() const { return fakeNow; }
}

namespace ChipSatDevices
{
Flash::Flash(ChipSatPlatform::System &system) : system_(system) {}
Status Flash::begin() { ready_ = true; paused_ = false; return Status::Ok; }
bool Flash::busy() { return !paused_ && (eraseStuck || fakeNow < eraseDoneMs); }

Status Flash::read(uint32_t address, uint8_t *buffer, size_t length)
{
  if (!ready_ || paused_ || busy()) {
    return Status::NotReady;
  }
  memcpy(buffer, &memory[address], length);
  return Status::Ok;
}

Status Flash::program(uint32_t address, const uint8_t *data, size_t length)
{
  if (!ready_ || paused_ || busy()) {
    return Status::NotReady;
  }
  if (address % 256 + length > 256) {
    return Status::Failed;
  }
  ++programs;
  if (programsToFail > 0) {
    --programsToFail;
    return Status::Failed;
  }
  for (size_t i = 0; i < length; ++i) {
    memory[address + i] &= data[i];
  }
  return Status::Ok;
}

Status Flash::startSectorErase(uint32_t address)
{
  if (!ready_ || paused_ || busy()) {
    return Status::NotReady;
  }
  memset(&memory[address], 0xFF, ChipSatConstants::kFlashSectorSize);
  erased.push_back(address);
  eraseDoneMs = fakeNow + eraseMs;
  return Status::Ok;
}

Status Flash::pause() { paused_ = true; return Status::Ok; }
Status Flash::resume() { paused_ = false; return Status::Ok; }
}

static int checks = 0, failures = 0;

static void check(bool ok, const char *what)
{
  ++checks;
  if (!ok) {
    ++failures;
  }
  printf("%s %s\n", ok ? "ok  " : "FAIL", what);
}

static ChipSatPlatform::System chipSystem;
static ChipSatDevices::Flash flash(chipSystem);

static void wipe()
{
  std::fill(memory.begin(), memory.end(), 0xFF);
  erased.clear();
  fakeNow = 0;
  eraseDoneMs = 0;
  eraseMs = 36;
  eraseStuck = false;
  programsToFail = 0;
  programs = 0;
  flash.begin();
}

static TelemetryPacket packetNumbered(uint16_t counter)
{
  TelemetryPacket packet;
  memset(&packet, 0x5A, sizeof packet);
  packet.packetCounter = counter;
  packet.CSID = 9;
  return packet;
}

// A record written straight into memory, the way an earlier boot would have left it
static void putRecord(uint32_t index, uint16_t boot, uint16_t counter)
{
  LogRecord record;
  record.type = ChipSatApp::kLogTypePacket;
  record.bootCount = boot;
  record.uptimeMs = 1000u * counter;
  record.packet = packetNumbered(counter);
  ChipSatApp::encodeRecord(record, &memory[index * kLogRecordSize]);
}

// The flight loop in LE: a record every 50 ms, ensureSpace right after it
static void run(FlashLog &log, uint16_t count, uint16_t firstCounter, uint32_t spacingMs)
{
  for (uint16_t i = 0; i < count; ++i) {
    log.append(packetNumbered(static_cast<uint16_t>(firstCounter + i)), fakeNow);
    log.ensureSpace();
    fakeNow += spacingMs;
  }
  // let the last erase finish
  fakeNow += 1000;
  log.busy();
}

// What flash_dump does: walk from record 0 to the first erased one
struct Walk
{
  uint32_t good = 0;
  uint32_t bad = 0;
  bool inOrder = true;
};

static Walk walk()
{
  Walk result;
  int32_t last = -1;
  for (uint32_t index = 0; index < kLogRecordCount; ++index) {
    const uint8_t *bytes = &memory[index * kLogRecordSize];
    if (ChipSatApp::recordErased(bytes)) {
      break;
    }
    LogRecord record;
    if (!ChipSatApp::decodeRecord(bytes, record)) {
      ++result.bad;
      continue;
    }
    ++result.good;
    const int32_t order = record.bootCount * 65536 + record.packet.packetCounter;
    if (order <= last) {
      result.inOrder = false;
    }
    last = order;
  }
  return result;
}

// Reads like flash_dump: sectors oldest first by their first record, each up to its first erased
// slot. Also checks packets stay in order by boot and uptime across the wrap
static Walk ringWalk(uint32_t *packets = nullptr)
{
  std::vector<std::pair<ChipSatApp::SectorHead, uint32_t>> heads;
  for (uint32_t sector = 0; sector < ChipSatApp::kLogSectorCount; ++sector) {
    ChipSatApp::SectorHead head;
    ChipSatApp::readSectorHead(flash, sector, head);
    if (!head.erased) {
      heads.push_back({head, sector});
    }
  }
  std::stable_sort(heads.begin(), heads.end(),
                   [](const auto &a, const auto &b) { return ChipSatApp::headOlder(a.first, b.first); });
  Walk result;
  uint64_t last = 0;
  uint32_t count = 0;
  for (const auto &h : heads) {
    for (uint32_t slot = 0; slot < kLogRecordsPerSector; ++slot) {
      const uint8_t *bytes = &memory[(h.second * kLogRecordsPerSector + slot) * kLogRecordSize];
      if (ChipSatApp::recordErased(bytes)) {
        break;
      }
      LogRecord record;
      if (!ChipSatApp::decodeRecord(bytes, record)) {
        ++result.bad;
        continue;
      }
      ++result.good;
      const uint64_t order = static_cast<uint64_t>(record.bootCount) << 32 | record.uptimeMs;
      if (order < last) {
        result.inOrder = false;
      }
      last = order;
      count += record.type == ChipSatApp::kLogTypePacket ? 1 : 0;
    }
  }
  if (packets != nullptr) {
    *packets = count;
  }
  return result;
}

// Whether a packet with this boot and counter is still somewhere on the chip
static bool onChip(uint16_t boot, uint16_t counter)
{
  for (uint32_t index = 0; index < kLogRecordCount; ++index) {
    LogRecord record;
    if (ChipSatApp::decodeRecord(&memory[index * kLogRecordSize], record) &&
        record.type == ChipSatApp::kLogTypePacket && record.bootCount == boot &&
        record.packet.packetCounter == counter) {
      return true;
    }
  }
  return false;
}

static void testRecordLayout()
{
  LogRecord record;
  record.type = ChipSatApp::kLogTypePacket;
  record.bootCount = 0x1234;
  record.uptimeMs = 0xA1B2C3D4;
  record.packet = packetNumbered(777);
  uint8_t bytes[kLogRecordSize];
  ChipSatApp::encodeRecord(record, bytes);

  check(bytes[0] == 0xA5, "layout: type byte first");
  check(bytes[1] == 0x34 && bytes[2] == 0x12, "layout: boot count in 1-2, little endian");
  check(bytes[3] == 0xD4 && bytes[4] == 0xC3 && bytes[5] == 0xB2 && bytes[6] == 0xA1, "layout: uptime in 3-6, little endian");
  check(memcmp(&bytes[7], &record.packet, 55) == 0, "layout: the 55 byte packet in 7-61, as sent");
  const uint16_t crc = ChipSatTelemetry::calculateCRC16(bytes, 62);
  check(bytes[62] == (crc & 0xFF) && bytes[63] == (crc >> 8), "layout: calculateCRC16 over 0-61 in 62-63, little endian");

  LogRecord back;
  check(ChipSatApp::decodeRecord(bytes, back) && back.bootCount == 0x1234 && back.uptimeMs == 0xA1B2C3D4 &&
            back.packet.packetCounter == 777 && memcmp(&back.packet, &record.packet, 55) == 0,
        "layout: decode gives back what went in");

  bool everyByteCaught = true;
  for (uint8_t i = 0; i < kLogRecordSize; ++i) {
    for (uint8_t bit = 0; bit < 8; ++bit) {
      bytes[i] ^= static_cast<uint8_t>(1 << bit);
      if (ChipSatApp::decodeRecord(bytes, back)) {
        everyByteCaught = false;
      }
      bytes[i] ^= static_cast<uint8_t>(1 << bit);
    }
  }
  check(everyByteCaught, "crc: any single flipped bit in the 64 bytes is caught");

  uint8_t wrongType[kLogRecordSize];
  record.type = 0x5A;
  ChipSatApp::encodeRecord(record, wrongType);
  check(!ChipSatApp::decodeRecord(wrongType, back), "crc: a good CRC with an unknown type is still refused");

  uint8_t blank[kLogRecordSize];
  memset(blank, 0xFF, sizeof blank);
  check(ChipSatApp::recordErased(blank), "erased: all FF");
  blank[40] = 0xFE;
  check(!ChipSatApp::recordErased(blank), "erased: one bit cleared is not erased");
}

static void testEmptyChip()
{
  wipe();
  FlashLog log(flash, chipSystem);
  const std::vector<uint8_t> before = memory;
  check(log.begin() == Status::Ok && log.records() == 0 && log.bootCount() == 0 && !log.stopped(),
        "empty chip: end at record 0, boot 0");
  check(memory == before && erased.empty() && programs == 0, "empty chip: begin wrote and erased nothing");

  check(log.append(packetNumbered(0), 0) == Status::Ok && log.holding() && log.records() == 0,
        "empty chip: first record waits in RAM, sector 0 isn't erased yet");
  check(log.ensureSpace() == Status::Ok && log.busy() && erased.size() == 1 && erased[0] == 0,
        "empty chip: ensureSpace starts erasing sector 0 and returns");
  check(log.ensureSpace() == Status::Ok && erased.size() == 1, "empty chip: no second erase while one runs");
  fakeNow += 40;
  check(!log.busy() && !log.holding() && log.records() == 1, "empty chip: the waiting record goes in once the erase is done");
  log.ensureSpace();
  check(erased.size() == 2 && erased[1] == 4096, "empty chip: then sector 1 gets erased ahead");
}

static void testFlightRun()
{
  wipe();
  FlashLog log(flash, chipSystem);
  log.begin();
  // the boot wait: two erases before the first record
  for (int i = 0; i < 2; ++i) {
    log.ensureSpace();
    fakeNow += 40;
    log.busy();
  }
  run(log, 1000, 0, 50);
  const Walk result = walk();
  check(log.records() == 1000 && log.dropped() == 0, "flight run, 50 ms apart: 1000 records, none dropped");
  check(result.good == 1000 && result.bad == 0 && result.inOrder, "flight run: all 1000 read back good and in order");
  check(erased.size() == 1000 / kLogRecordsPerSector + 2, "flight run: one erase per sector, one sector ahead");
  bool neverAhead = true;
  for (size_t i = 0; i < erased.size(); ++i) {
    if (erased[i] != i * 4096) {
      neverAhead = false;
    }
  }
  check(neverAhead, "flight run: sectors erased in order, never a used one");

  // after a reset
  const std::vector<uint8_t> before = memory;
  const size_t erases = erased.size();
  FlashLog again(flash, chipSystem);
  check(again.begin() == Status::Ok && again.records() == 1000 && again.bootCount() == 1,
        "reboot: end found at 1000, boot count 1");
  check(memory == before && erased.size() == erases, "reboot: begin wrote and erased nothing");
  run(again, 100, 0, 50);
  const Walk after = walk();
  check(again.records() == 1100 && after.good == 1100 && after.inOrder, "reboot: 100 more go after the old ones");
  LogRecord last;
  ChipSatApp::decodeRecord(&memory[1099 * kLogRecordSize], last);
  check(last.bootCount == 1, "reboot: new records carry boot 1");
}

// Every 50 ms against a slow chip, the ring waits through every erase so none is lost
static void testSlowErase()
{
  wipe();
  eraseMs = 120;   // a slow chip, datasheet max is 400
  FlashLog log(flash, chipSystem);
  log.begin();
  run(log, 300, 0, 50);
  uint32_t packets = 0;
  const Walk result = ringWalk(&packets);
  check(log.dropped() == 0 && packets == 300 && !log.holding(),
        "slow erase: the ring waits through every erase, all 300 written, none dropped");
  check(result.good == log.records() && result.good == packets && result.inOrder,
        "slow erase: what's written is good and in order");
}

static void testBoundary()
{
  wipe();
  for (uint32_t i = 0; i < 2 * kLogRecordsPerSector; ++i) {
    putRecord(i, 3, static_cast<uint16_t>(i));
  }
  FlashLog log(flash, chipSystem);
  check(log.begin() == Status::Ok && log.records() == 128 && log.bootCount() == 4, "boundary: end right at sector 2");
  log.ensureSpace();
  check(erased.size() == 1 && erased[0] == 2 * 4096,
        "boundary: sector 2 is erased again before use, a reset may have cut its erase");

  wipe();
  for (uint32_t i = 0; i < 70; ++i) {
    putRecord(i, 3, static_cast<uint16_t>(i));
  }
  for (uint32_t i = 66; i < 70; ++i) {
    putRecord(i, 4, static_cast<uint16_t>(i));   // a later boot that started partway into sector 1
  }
  FlashLog mid(flash, chipSystem);
  mid.begin();
  check(mid.records() == 70, "mid sector: end at 70");
  check(mid.bootCount() == 5, "mid sector: boot count from the last record, not the sector's first one");
  check(mid.append(packetNumbered(70), 0) == Status::Ok && mid.records() == 71 && erased.empty(),
        "mid sector: the rest of sector 1 is used without erasing it");
  mid.ensureSpace();
  check(erased.size() == 1 && erased[0] == 2 * 4096, "mid sector: sector 2 erased ahead");
}

static void testTornRecord()
{
  wipe();
  for (uint32_t i = 0; i < 10; ++i) {
    putRecord(i, 2, static_cast<uint16_t>(i));
  }
  // a reset halfway through a program: part of record 10 is there
  memset(&memory[10 * kLogRecordSize], 0x00, 20);
  FlashLog log(flash, chipSystem);
  check(log.begin() == Status::Ok && log.records() == 11 && log.bootCount() == 3,
        "torn record: counted as used, end after it, boot count from the last good one");
  run(log, 5, 0, 50);
  const Walk result = walk();
  check(result.good == 15 && result.bad == 1, "torn record: the reader skips it and finds all 15 good ones");
}

static void testStaleLastSector()
{
  wipe();
  // what flash_test leaves in the last sector
  for (uint32_t i = 0; i < 512; ++i) {
    memory[0x1FF000 + i] = static_cast<uint8_t>(i ^ 0x5A);
  }
  for (uint32_t i = 0; i < 300; ++i) {
    putRecord(i, 5, static_cast<uint16_t>(i));
  }
  FlashLog log(flash, chipSystem);
  check(log.begin() == Status::Ok && log.records() == 300 && log.bootCount() == 6,
        "stale last sector: end still found at 300");
}

// A full chip overwrites its oldest sector (decided 2026-10-19, was stop-when-full)
static void testFull()
{
  wipe();
  for (uint32_t i = 0; i < kLogRecordCount; ++i) {
    putRecord(i, 7, static_cast<uint16_t>(i));
  }
  const std::vector<uint8_t> before = memory;
  FlashLog log(flash, chipSystem);
  check(log.begin() == Status::Ok && log.records() == kLogRecordCount && !log.wrapped(),
        "full chip: all 32768 records found");
  check(log.bootCount() == 8, "full chip: boot count still from the last record");
  check(memory == before && erased.empty(), "full chip: begin wrote and erased nothing");
  check(log.append(packetNumbered(1), 0) == Status::Ok && log.holding(), "full chip: append waits for a sector");
  check(log.ensureSpace() == Status::Ok && erased.size() == 1 && erased[0] == 0,
        "full chip: the oldest sector, 0, is erased for it");
  run(log, 99, 2, 50);
  uint32_t fullPackets = 0;
  const Walk full = ringWalk(&fullPackets);
  check(log.wrapped() && full.good == kLogRecordCount - erased.size() * kLogRecordsPerSector + 100 - log.dropped() &&
        fullPackets == full.good && full.inOrder &&
        erased[1] == 0x1000,
        "full chip: 100 new records go round into sectors 0 and 1, read back in order after the old ones");

  // filling up during a run
  wipe();
  for (uint32_t i = 0; i < kLogRecordCount - 100; ++i) {
    putRecord(i, 1, static_cast<uint16_t>(i));
  }
  FlashLog nearly(flash, chipSystem);
  nearly.begin();
  run(nearly, 200, 0, 50);
  check(erased.size() >= 3 && erased[0] == 0x1FF000 && erased[1] == 0 && erased[2] == 0x1000,
        "filling up: the last sector, then round to sector 0 and on");
  const Walk filling = ringWalk();
  check(filling.inOrder && filling.good == nearly.records() && nearly.records() <= kLogRecordCount,
        "filling up: every record still on the chip reads back in order");
}

// Round the seam and a reset in the middle of a sector: the end is where the newest records are
static void testWrapSeam()
{
  wipe();
  for (uint32_t i = 0; i < kLogRecordCount; ++i) {
    putRecord(i, 3, static_cast<uint16_t>(i));
  }
  FlashLog log(flash, chipSystem);
  log.begin();
  run(log, 70, 0, 50);   // sector 0 full and 6 into sector 1, boot 4
  const std::vector<uint8_t> before = memory;
  const size_t erases = erased.size();
  FlashLog again(flash, chipSystem);
  check(again.begin() == Status::Ok && again.bootCount() == 5 && again.wrapped(),
        "seam: after a reset the boot count is the highest on the chip plus one");
  check(memory == before && erased.size() == erases, "seam: begin wrote and erased nothing");
  check(again.append(packetNumbered(100), fakeNow) == Status::Ok && !again.holding() && erased.size() == erases,
        "seam: the next record goes straight in after the 70th, mid sector 1");
  fakeNow += 50;
  run(again, 9, 101, 50);
  uint32_t packets = 0;
  const Walk result = ringWalk(&packets);
  check(result.inOrder && result.bad == 0, "seam: boots 3, 4 and 5 read back in order across the wrap");
  check(onChip(4, 0) && onChip(4, 69) && onChip(5, 100) && onChip(5, 109) && !onChip(3, 0) && onChip(3, 32767),
        "seam: sector 0's old records went, the newest old ones stayed");
}

// A reset while the sector ahead was being erased, or while the first record of a new sector was
// being written. Neither may be taken for the end, and both get erased before anything goes in
static void testPowerCuts()
{
  wipe();
  for (uint32_t i = 0; i < kLogRecordCount; ++i) {
    putRecord(i, 2, static_cast<uint16_t>(i));
  }
  // the ring has gone round to sector 10: sectors 0 to 9 hold boot 3, sector 10 is half erased
  for (uint32_t i = 0; i < 10 * kLogRecordsPerSector; ++i) {
    putRecord(i, 3, static_cast<uint16_t>(i));
  }
  uint8_t *half = &memory[10 * 4096];
  for (uint32_t i = 0; i < 4096; ++i) {
    half[i] = i < 64 ? 0xFF : static_cast<uint8_t>(half[i] | 0x0F);   // first record FF, the rest part erased
  }
  FlashLog log(flash, chipSystem);
  check(log.begin() == Status::Ok && log.bootCount() == 4, "cut mid-erase: the end is still found, boot 4 next");
  check(log.append(packetNumbered(1), 1000) == Status::Ok && log.holding() && log.ensureSpace() == Status::Ok &&
        erased.size() == 1 && erased[0] == 10 * 4096,
        "cut mid-erase: the half erased sector is erased again before anything goes in");
  fakeNow = 2000;
  log.busy();
  check(!log.holding() && ringWalk().inOrder, "cut mid-erase: the record went into it, in order");

  // the first record of a fresh sector cut off
  wipe();
  for (uint32_t i = 0; i < 3 * kLogRecordsPerSector; ++i) {
    putRecord(i, 6, static_cast<uint16_t>(i));
  }
  memset(&memory[3 * 4096], 0x00, 20);
  FlashLog torn(flash, chipSystem);
  check(torn.begin() == Status::Ok && torn.bootCount() == 7 && torn.records() == 3 * kLogRecordsPerSector,
        "cut first record: the sector before is the end, the torn one isn't counted");
  torn.append(packetNumbered(1), 0);
  torn.ensureSpace();
  check(erased.size() == 1 && erased[0] == 3 * 4096, "cut first record: its sector is erased before it is used");
}

// Events: a boot event and the dropped count, readable like any record
static void testEvents()
{
  wipe();
  FlashLog log(flash, chipSystem);
  log.begin();
  const uint8_t csr[4] = {0x00, 0x00, 0x00, 0x0C};
  check(log.appendEvent(ChipSatApp::kLogEventBoot, 5, csr, sizeof csr) == Status::Ok, "events: boot event appended");
  log.ensureSpace();
  fakeNow = 1000;
  log.busy();
  LogRecord record;
  check(ChipSatApp::decodeRecord(&memory[0], record) && record.type == ChipSatApp::kLogTypeEvent &&
        record.eventKind == ChipSatApp::kLogEventBoot && record.uptimeMs == 5 && record.eventData[3] == 0x0C,
        "events: it reads back as an event, kind and data as written");
  // The ring only loses records when it goes all the way round before the chip takes them: 40 s
  // with the chip paused is 800 records into a ring of 600
  flash.pause();
  for (uint16_t i = 0; i < 800; ++i) {
    log.append(packetNumbered(i), fakeNow);
    fakeNow += 50;
  }
  flash.resume();
  check(log.dropped() == 800 - ChipSatConfig::kLogRingRecords, "events: 40 s paused drops what the ring cannot hold, counted");
  for (int i = 0; i < 400; ++i) {
    log.ensureSpace();
    fakeNow += 50;
    log.busy();
  }
  bool said = false;
  for (uint32_t index = 0; index < 2 * kLogRecordsPerSector * 8; ++index) {
    if (ChipSatApp::decodeRecord(&memory[index * kLogRecordSize], record) &&
        record.type == ChipSatApp::kLogTypeEvent && record.eventKind == ChipSatApp::kLogEventDropped) {
      said = record.eventData[0] | record.eventData[1] << 8;
    }
  }
  check(log.dropped() > 0 && said, "events: a dropped event follows the drops");
}

// Packet records in reader order, and whether each is strictly newer than the one before
struct Packets
{
  std::vector<uint32_t> ms;
  bool strict = true;
};

static Packets packetsRead(uint16_t boot)
{
  std::vector<std::pair<ChipSatApp::SectorHead, uint32_t>> heads;
  for (uint32_t sector = 0; sector < ChipSatApp::kLogSectorCount; ++sector) {
    ChipSatApp::SectorHead head;
    ChipSatApp::readSectorHead(flash, sector, head);
    if (!head.erased) {
      heads.push_back({head, sector});
    }
  }
  std::stable_sort(heads.begin(), heads.end(),
                   [](const auto &a, const auto &b) { return ChipSatApp::headOlder(a.first, b.first); });
  Packets result;
  uint64_t last = 0;
  bool any = false;
  for (const auto &h : heads) {
    for (uint32_t slot = 0; slot < kLogRecordsPerSector; ++slot) {
      const uint8_t *bytes = &memory[(h.second * kLogRecordsPerSector + slot) * kLogRecordSize];
      if (ChipSatApp::recordErased(bytes)) {
        break;
      }
      LogRecord record;
      if (!ChipSatApp::decodeRecord(bytes, record) || record.type != ChipSatApp::kLogTypePacket) {
        continue;
      }
      const uint64_t order = static_cast<uint64_t>(record.bootCount) << 32 | record.uptimeMs;
      if (any && order <= last) {
        result.strict = false;
      }
      any = true;
      last = order;
      if (record.bootCount == boot) {
        result.ms.push_back(record.uptimeMs);
      }
    }
  }
  return result;
}

struct OnChip
{
  uint32_t packets = 0;
  uint32_t times = 0;
  uint32_t boot = 0;
  uint8_t bootCsr3 = 0;
  uint32_t other = 0;
};

static OnChip onChipByType()
{
  OnChip seen;
  for (uint32_t index = 0; index < kLogRecordCount; ++index) {
    LogRecord record;
    if (!ChipSatApp::decodeRecord(&memory[index * kLogRecordSize], record)) {
      continue;
    }
    if (record.type == ChipSatApp::kLogTypePacket) {
      ++seen.packets;
    } else if (record.type == ChipSatApp::kLogTypeTime) {
      ++seen.times;
    } else if (record.eventKind == ChipSatApp::kLogEventBoot) {
      ++seen.boot;
      seen.bootCsr3 = record.eventData[3];
    } else {
      ++seen.other;
    }
  }
  return seen;
}

// The flight loop on its own clock: a record every 50 ms, the writer called between them as the
// controller's loop would, a few times per record
static void fly(FlashLog &log, uint32_t forMs, uint16_t &counter)
{
  const uint32_t until = fakeNow + forMs;
  while (fakeNow < until) {
    log.append(packetNumbered(counter++), fakeNow);
    for (int i = 0; i < 5; ++i) {
      log.ensureSpace();
      fakeNow += 10;
      log.busy();
    }
  }
}

static FlashLog *booted(FlashLog &log)
{
  log.begin();
  for (int i = 0; i < 2; ++i) {   // the boot wait, two erases
    log.ensureSpace();
    fakeNow += 40;
    log.busy();
  }
  return &log;
}

// The ground keeps one record per gap and the ring the last 30 s; the take-off writes all of the
// ring after what is on the chip, so the reader sees every record once and in order
static void testRingFlush()
{
  wipe();
  FlashLog log(flash, chipSystem);
  booted(log);
  log.setGroundGapMs(10000);
  log.setAir(false);
  uint16_t counter = 0;
  fly(log, 120000, counter);   // two minutes on the ground
  const uint32_t takeoffMs = fakeNow;
  const uint32_t groundOnChip = log.records();
  check(groundOnChip >= 8 && groundOnChip <= 10 && log.waiting() >= ChipSatConfig::kLogRingRecords - ChipSatConfig::kLogRingSlackRecords - 1,
        "ring: two minutes on the ground put a record per 10 s on the chip, the last 30 s wait in the ring");
  log.setAir(true);
  fly(log, 60000, counter);    // a minute in the air
  const Packets read = packetsRead(0);
  check(read.strict, "ring flush: every record once, each newer than the one before, across the take-off");
  uint32_t before = 0;
  bool every = true;
  for (size_t i = 1; i < read.ms.size(); ++i) {
    if (read.ms[i] >= takeoffMs - 28000 && read.ms[i] - read.ms[i - 1] != 50) {
      every = false;
    }
    before += read.ms[i] < takeoffMs && read.ms[i] >= takeoffMs - 28000 ? 1 : 0;
  }
  check(every && before >= 28000 / 50 - 1, "ring flush: every 50 ms from 28 s before the take-off on");
  check(log.dropped() == 0 && log.waiting() < 10, "ring flush: nothing dropped and the cursor caught up in the air");

  // A reset after it: the end is after the flush, and the next boot goes on from there
  const std::vector<uint8_t> chip = memory;
  FlashLog again(flash, chipSystem);
  check(again.begin() == Status::Ok && again.bootCount() == 1 && again.records() == log.records() && memory == chip,
        "ring flush: after a reset the end is found after the flush");
}

// The trickle: one record per ground gap, its own uptime deciding. Gap 0 writes nothing at all
static void testGroundTrickle()
{
  wipe();
  FlashLog log(flash, chipSystem);
  booted(log);
  log.setGroundGapMs(10000);
  log.setAir(false);
  uint16_t counter = 0;
  fly(log, 600000, counter);
  const Packets read = packetsRead(0);
  bool spaced = read.ms.size() > 1;
  for (size_t i = 1; i < read.ms.size(); ++i) {
    spaced = spaced && read.ms[i] - read.ms[i - 1] == 10000;
  }
  check(spaced && read.ms.size() >= 56 && read.ms.size() <= 58 && read.strict,
        "trickle: ten minutes on the ground is a record every 10 s, the last 30 s still in the ring");

  // Gap 0: only the boot event reaches the chip (decided 2026-10-22), no time anchors
  wipe();
  FlashLog none(flash, chipSystem);
  booted(none);
  none.setGroundGapMs(0);
  none.setAir(false);
  const uint8_t csr[4] = {1, 2, 3, 4};
  none.appendEvent(ChipSatApp::kLogEventBoot, fakeNow, csr, sizeof csr);
  counter = 0;
  for (int i = 0; i < 60; ++i) {
    none.appendTime(fakeNow, 1790000000u + i * 10, 0, 0x37, 20);
    fly(none, 10000, counter);
  }
  const OnChip seen = onChipByType();
  check(seen.boot == 1 && seen.bootCsr3 == 4 && seen.packets == 0 && seen.times == 0 && seen.other == 0 &&
        none.records() == 1 && none.dropped() == 0 && !none.holding(),
        "gap 0: ten minutes on the ground write the boot event and nothing else, no time anchor, nothing dropped");
}

// boot event, then 30 s of boot-air, then ground with gap 0 or 10 s
static void testBootEvent()
{
  for (uint32_t gap : {0u, 10000u}) {
    wipe();
    FlashLog log(flash, chipSystem);
    log.begin();
    log.setGroundGapMs(gap);
    log.setAir(true);   // the boot-air
    const uint8_t csr[4] = {0, 0, 0, 0x24};   // PIN + IWDG
    check(log.appendEvent(ChipSatApp::kLogEventBoot, fakeNow, csr, sizeof csr) == Status::Ok,
          gap == 0 ? "boot event, gap 0: appended before the first erase is done" : "boot event, gap 10 s: appended before the first erase is done");
    uint16_t counter = 0;
    log.appendTime(fakeNow, 1790000000u, 0, 0x37, 20);
    fly(log, 30000, counter);
    log.setAir(false);
    for (int i = 0; i < 30; ++i) {
      log.appendTime(fakeNow, 1790000100u + i * 10, 0, 0x37, 20);
      fly(log, 10000, counter);
    }
    const OnChip seen = onChipByType();
    LogRecord first;
    ChipSatApp::decodeRecord(&memory[0], first);
    const bool firstIsBoot = first.type == ChipSatApp::kLogTypeEvent && first.eventKind == ChipSatApp::kLogEventBoot &&
                             first.eventData[3] == 0x24;
    if (gap == 0) {
      check(firstIsBoot && seen.boot == 1 && seen.times == 1 && seen.packets >= 590 && seen.packets <= 601,
            "boot event, gap 0: first on the chip, then the boot-air's 30 s and its one anchor, nothing from the ground");
    } else {
      // the anchors at +280 and +290 s are still in the ring
      check(firstIsBoot && seen.boot == 1 && seen.times == 1 + 28 && seen.packets >= 590 + 27 && seen.packets <= 601 + 28,
            "boot event, gap 10 s: first on the chip; the ground keeps its trickle and its anchors, the last 30 s still in the ring");
    }
    check(log.dropped() == 0, gap == 0 ? "boot event, gap 0: nothing dropped" : "boot event, gap 10 s: nothing dropped");
  }

  // slow first erase
  wipe();
  FlashLog slow(flash, chipSystem);
  eraseMs = 900;
  slow.begin();
  slow.setGroundGapMs(0);
  slow.setAir(false);
  slow.ensureSpace();
  const uint8_t csr[4] = {0, 0, 0, 0x08};
  slow.appendEvent(ChipSatApp::kLogEventBoot, fakeNow, csr, sizeof csr);
  uint16_t counter = 0;
  fly(slow, 120000, counter);
  const OnChip seen = onChipByType();
  check(seen.boot == 1 && seen.packets == 0 && slow.dropped() == 0,
        "boot event behind a slow erase, gap 0 on the ground: it still reaches the chip, alone");
}

// Back on the ground: what the air left in the ring is written first, then the trickle again
static void testCatchUp()
{
  wipe();
  eraseMs = 400;   // the datasheet's slowest
  FlashLog log(flash, chipSystem);
  booted(log);
  log.setGroundGapMs(10000);
  log.setAir(false);
  uint16_t counter = 0;
  fly(log, 40000, counter);
  // the take-off, with the chip as slow as it gets: the 30 s flush crosses ten sector erases
  log.setAir(true);
  const size_t erasesBefore = erased.size();
  fly(log, 2000, counter);
  check(log.waiting() > 0, "catch up: two seconds after the take-off the flush is still going");
  fly(log, 30000, counter);
  check(erased.size() - erasesBefore >= 10 && log.dropped() == 0 && log.waiting() < 10,
        "catch up: through ten 400 ms erases the ring caught up, nothing dropped");
  const uint32_t landMs = fakeNow;
  // landed with records still waiting: they are all written before the ground lets any go
  flash.pause();
  fly(log, 2000, counter);
  flash.resume();
  log.setAir(false);
  fly(log, 60000, counter);
  const Packets read = packetsRead(0);
  bool tail = true;
  uint32_t trickle = 0;
  for (size_t i = 1; i < read.ms.size(); ++i) {
    if (read.ms[i] <= landMs + 2000 && read.ms[i] > landMs - 30000) {
      tail = tail && read.ms[i] - read.ms[i - 1] == 50;
    }
    trickle += read.ms[i] > landMs + 2000 ? 1 : 0;
  }
  check(read.strict && tail && log.dropped() == 0, "catch up: the air's last records, paused ones too, are all written");
  check(trickle >= 2 && trickle <= 5, "catch up: then the ground goes back to a record per gap");
}

// The ring going round before the chip takes anything: the oldest go, counted, the newest stay
static void testRingOverrun()
{
  wipe();
  FlashLog log(flash, chipSystem);
  booted(log);
  flash.pause();
  for (uint16_t i = 0; i < 1000; ++i) {
    log.append(packetNumbered(i), fakeNow);
    fakeNow += 50;
  }
  flash.resume();
  check(log.dropped() == 1000 - ChipSatConfig::kLogRingRecords, "overrun: 1000 into a ring of 600 in the air drops 400, counted");
  uint16_t counter = 1000;
  fly(log, 5000, counter);
  check(!onChip(0, 399) && onChip(0, 400) && onChip(0, 999) && onChip(0, 1099) && packetsRead(0).strict,
        "overrun: the newest 600 are written, in order, after the gap");
}

// A chip full of older flights: the take-off's flush goes over the oldest sectors, and after a
// reset the boot count is above every boot still on the chip, so the readers' order holds
static void testFullChipFlush()
{
  wipe();
  for (uint32_t i = 0; i < kLogRecordCount; ++i) {
    putRecord(i, static_cast<uint16_t>(3 + i / (kLogRecordCount / 4)), static_cast<uint16_t>(i));   // boots 3 to 6
  }
  FlashLog log(flash, chipSystem);
  booted(log);
  check(log.bootCount() == 7, "full chip: the next boot is 7");
  log.setGroundGapMs(10000);
  log.setAir(false);
  uint16_t counter = 0;
  fly(log, 60000, counter);
  log.setAir(true);
  fly(log, 120000, counter);
  log.setAir(false);
  fly(log, 60000, counter);
  const Packets read = packetsRead(7);
  check(read.strict && log.dropped() == 0 && log.wrapped() && !onChip(3, 0) && onChip(6, kLogRecordCount - 1),
        "full chip: the flight goes over the oldest sectors, every record once and in order after the old ones");

  FlashLog again(flash, chipSystem);
  again.begin();
  uint16_t highest = 0;
  for (uint32_t sector = 0; sector < ChipSatApp::kLogSectorCount; ++sector) {
    ChipSatApp::SectorHead head;
    ChipSatApp::readSectorHead(flash, sector, head);
    if (head.dated && head.boot > highest) {
      highest = head.boot;
    }
  }
  check(highest == 7 && again.bootCount() == 8 && again.wrapped(),
        "full chip reboot: boot 8, higher than every sector still on the chip");
  counter = 0;
  again.setAir(true);
  fly(again, 10000, counter);
  check(packetsRead(8).strict && packetsRead(8).ms.size() == 200, "full chip reboot: boot 8 reads after boot 7, in order");
}

static void testPaused()
{
  wipe();
  FlashLog log(flash, chipSystem);
  log.begin();
  run(log, 10, 0, 50);
  flash.pause();
  check(log.append(packetNumbered(10), fakeNow) == Status::Ok && log.holding() && log.records() == 10,
        "paused: the record waits in RAM");
  flash.resume();
  log.ensureSpace();
  check(!log.holding() && log.records() == 11 && walk().good == 11, "paused: written after resume");
}

static void testWriteFailures()
{
  wipe();
  FlashLog log(flash, chipSystem);
  log.begin();
  run(log, 10, 0, 50);
  programsToFail = 1;
  check(log.append(packetNumbered(10), fakeNow) != Status::Ok && log.records() == 10,
        "failed write: nothing written, the slot is used again");
  // The failed record stays in the ring and goes first next time
  check(log.append(packetNumbered(11), fakeNow) == Status::Ok && log.records() == 12 && walk().good == 12 &&
        walk().inOrder && log.dropped() == 0,
        "failed write: tried again on the next call, into the same slot, no gap of FF and nothing lost");

  programsToFail = 3;
  for (int i = 0; i < 3; ++i) {
    log.append(packetNumbered(12), fakeNow);
  }
  check(!log.ready() && log.append(packetNumbered(13), fakeNow) == Status::NotReady,
        "three failed writes in a row: logging stops for this boot");

  wipe();
  FlashLog stuck(flash, chipSystem);
  stuck.begin();
  stuck.ensureSpace();
  eraseStuck = true;
  fakeNow += 500;
  check(stuck.ensureSpace() == Status::Ok && stuck.busy(), "stuck erase: still waiting at 500 ms");
  fakeNow += 600;
  check(stuck.ensureSpace() == Status::Timeout && !stuck.ready(), "stuck erase: gives up past 1 s, logging stops");
}

int main()
{
  testRecordLayout();
  testEmptyChip();
  testFlightRun();
  testSlowErase();
  testBoundary();
  testTornRecord();
  testStaleLastSector();
  testFull();
  testWrapSeam();
  testPowerCuts();
  testEvents();
  testPaused();
  testWriteFailures();
  testRingFlush();
  testGroundTrickle();
  testBootEvent();
  testCatchUp();
  testRingOverrun();
  testFullChipFlush();

  printf("%s: %d checks, %d failed\n", failures ? "FAILED" : "PASSED", checks, failures);
  return failures ? 1 : 0;
}
