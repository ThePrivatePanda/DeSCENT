// FlashLog.cpp on the laptop, against a chip kept in RAM that behaves like NOR flash: erase sets FF, a program can
// only clear bits, reads and programs are refused while an erase runs
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src/app test/test_flash_log.cpp ../V2_6_X_Code/src/app/{FlashLog,Telemetry}.cpp -o /tmp/flashlog && /tmp/flashlog
#include <cstdio>
#include <cstring>
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
  check(log.begin() == Status::Ok && log.records() == 0 && log.bootCount() == 0 && !log.full(),
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

static void testSlowErase()
{
  wipe();
  eraseMs = 120;   // a slow chip, datasheet max is 400
  FlashLog log(flash, chipSystem);
  log.begin();
  run(log, 300, 0, 50);
  const Walk result = walk();
  check(log.dropped() > 0 && log.records() + log.dropped() == 300,
        "slow erase: every record is either written or counted as dropped");
  check(result.good == log.records() && result.inOrder, "slow erase: what's written is good and in order");
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

static void testFull()
{
  wipe();
  for (uint32_t i = 0; i < kLogRecordCount; ++i) {
    putRecord(i, 7, static_cast<uint16_t>(i));
  }
  const std::vector<uint8_t> before = memory;
  FlashLog log(flash, chipSystem);
  check(log.begin() == Status::Ok && log.full() && log.records() == kLogRecordCount,
        "full chip: all 32768 records found");
  check(log.bootCount() == 8, "full chip: boot count still from the last record");
  check(log.append(packetNumbered(1), 0) == Status::Failed && log.ensureSpace() == Status::Ok,
        "full chip: append refused, ensureSpace has nothing to do");
  check(memory == before && erased.empty(), "full chip: nothing erased, no wrap");

  // filling up during a run
  wipe();
  for (uint32_t i = 0; i < kLogRecordCount - 100; ++i) {
    putRecord(i, 1, static_cast<uint16_t>(i));
  }
  FlashLog nearly(flash, chipSystem);
  nearly.begin();
  run(nearly, 200, 0, 50);
  check(nearly.full() && nearly.records() == kLogRecordCount, "filling up: stops at exactly 32768");
  check(erased.size() == 1 && erased[0] == 0x1FF000, "filling up: only the last sector erased, never sector 0");
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
  check(log.append(packetNumbered(11), fakeNow) == Status::Ok && log.records() == 11 && walk().good == 11,
        "failed write: next one fills the slot, no gap of FF in the log");

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
  testPaused();
  testWriteFailures();

  printf("%s: %d checks, %d failed\n", failures ? "FAILED" : "PASSED", checks, failures);
  return failures ? 1 : 0;
}
