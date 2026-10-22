#include "FlashLog.h"
#include <string.h>

namespace ChipSatApp
{

using ChipSatPlatform::Status;
using ChipSatTelemetry::calculateCRC16;
using namespace ChipSatConstants;
using namespace ChipSatConfig;

static_assert(sizeof(ChipSatTelemetry::TelemetryPacket) == kLogCrcOffset - kLogPacketOffset, "the packet fills bytes 7-61");
static_assert(kFlashPageSize % kLogRecordSize == 0, "a record can't cross a page, the chip would wrap it");
static_assert(kLogSectorCount < kLogNoSector, "a sector number has to fit beside kLogNoSector");

static void put16(uint8_t *at, uint16_t v)
{
  at[0] = static_cast<uint8_t>(v);
  at[1] = static_cast<uint8_t>(v >> 8);
}

static void put32(uint8_t *at, uint32_t v)
{
  for (uint8_t i = 0; i < 4; ++i) {
    at[i] = static_cast<uint8_t>(v >> (8 * i));
  }
}

static uint16_t get16(const uint8_t *at)
{
  return static_cast<uint16_t>(at[0] | at[1] << 8);
}

static uint32_t get32(const uint8_t *at)
{
  uint32_t v = 0;
  for (uint8_t i = 0; i < 4; ++i) {
    v |= static_cast<uint32_t>(at[i]) << (8 * i);
  }
  return v;
}

void encodeRecord(const LogRecord &record, uint8_t *bytes)
{
  bytes[0] = record.type;
  put16(&bytes[kLogBootOffset], record.bootCount);
  put32(&bytes[kLogUptimeOffset], record.uptimeMs);
  if (record.type == kLogTypeEvent) {
    bytes[kLogEventKindOffset] = record.eventKind;
    memcpy(&bytes[kLogEventDataOffset], record.eventData, kLogEventDataBytes);
  } else {
    memcpy(&bytes[kLogPacketOffset], &record.packet, sizeof record.packet);
  }
  const uint16_t crc = calculateCRC16(bytes, kLogCrcOffset);
  put16(&bytes[kLogCrcOffset], crc);
}

bool decodeRecord(const uint8_t *bytes, LogRecord &record)
{
  const uint16_t crc = get16(&bytes[kLogCrcOffset]);
  if ((bytes[0] != kLogTypePacket && bytes[0] != kLogTypeTime && bytes[0] != kLogTypeEvent) ||
      crc != calculateCRC16(bytes, kLogCrcOffset)) {
    return false;
  }
  record.type = bytes[0];
  record.bootCount = get16(&bytes[kLogBootOffset]);
  record.uptimeMs = get32(&bytes[kLogUptimeOffset]);
  if (record.type == kLogTypeTime) {
    record.utcEpoch = get32(&bytes[kLogUtcEpochOffset]);
    record.utcNano = static_cast<int32_t>(get32(&bytes[kLogUtcNanoOffset]));
    record.utcAccNs = get32(&bytes[kLogUtcAccOffset]);
    record.utcBits = bytes[kLogUtcValidOffset];
    return true;
  }
  if (record.type == kLogTypeEvent) {
    record.eventKind = bytes[kLogEventKindOffset];
    memcpy(record.eventData, &bytes[kLogEventDataOffset], kLogEventDataBytes);
    return true;
  }
  memcpy(&record.packet, &bytes[kLogPacketOffset], sizeof record.packet);
  return true;
}

bool recordErased(const uint8_t *bytes)
{
  for (uint8_t i = 0; i < kLogRecordSize; ++i) {
    if (bytes[i] != 0xFF) {
      return false;
    }
  }
  return true;
}

Status readSectorHead(ChipSatDevices::Flash &flash, uint32_t sector, SectorHead &head)
{
  head = SectorHead();
  uint8_t bytes[kLogRecordSize];
  LogRecord record;
  for (uint32_t slot = 0; slot < kLogRecordsPerSector; ++slot) {
    const Status status = flash.read((sector * kLogRecordsPerSector + slot) * kLogRecordSize, bytes, sizeof bytes);
    if (status != Status::Ok) {
      return status;
    }
    if (recordErased(bytes)) {
      return Status::Ok;   // the rest of the sector is erased
    }
    head.erased = false;
    if (!decodeRecord(bytes, record)) {
      continue;
    }
    head.dated = true;
    head.boot = record.bootCount;
    head.ms = record.uptimeMs;
    return Status::Ok;
  }
  return Status::Ok;
}

bool headOlder(const SectorHead &a, const SectorHead &b)
{
  if (a.dated != b.dated) {
    return !a.dated;
  }
  return a.boot != b.boot ? a.boot < b.boot : a.ms < b.ms;
}

FlashLog::FlashLog(ChipSatDevices::Flash &flash, ChipSatPlatform::System &system)
  : flash_(flash), system_(system)
{
}

Status FlashLog::begin()
{
  ready_ = false;
  began_ = false;
  erasing_ = false;
  failures_ = 0;
  current_ = kLogNoSector;
  prepared_ = kLogNoSector;
  erasingSector_ = kLogNoSector;
  used_ = 0;
  wrapped_ = false;
  ringNext_ = 0;
  ringPending_ = 0;
  dragging_ = !air_;
  keptAny_ = false;
  memset(hasData_, 0, sizeof hasData_);
  if (!flash_.ready()) {
    return Status::NotReady;
  }

  // The log goes on from the newest sector. A chip that filled up under the old firmware has no
  // erased sector, and a wrapped one has older sectors after the newest
  SectorHead head;
  SectorHead newest;
  uint32_t newestSector = kLogNoSector;
  uint32_t dated = 0;
  bool found = false;
  uint16_t lastBoot = 0;
  for (uint32_t sector = 0; sector < kLogSectorCount; ++sector) {
    const Status status = readSectorHead(flash_, sector, head);
    if (status != Status::Ok) {
      return status;
    }
    if (!head.dated) {
      continue;   // erased, or nothing decodes (flash_test's pattern)
    }
    ++dated;
    hasData_[sector / 8] |= static_cast<uint8_t>(1U << (sector % 8));
    if (!found || head.boot > lastBoot) {
      lastBoot = head.boot;
    }
    found = true;
    if (newestSector == kLogNoSector || headOlder(newest, head)) {
      newest = head;
      newestSector = sector;
    }
  }

  // A record that isn't all FF but fails its CRC was cut off by a reset, still counts as used
  if (newestSector != kLogNoSector) {
    current_ = newestSector;
    next_ = (newestSector + 1) * kLogRecordsPerSector;
    uint8_t bytes[kLogRecordSize];
    LogRecord record;
    for (uint32_t index = newestSector * kLogRecordsPerSector; index < next_; ++index) {
      const Status status = flash_.read(index * kLogRecordSize, bytes, sizeof bytes);
      if (status != Status::Ok) {
        return status;
      }
      if (recordErased(bytes)) {
        next_ = index;
        break;
      }
      if (decodeRecord(bytes, record) && record.bootCount > lastBoot) {
        lastBoot = record.bootCount;
      }
    }
    used_ = (dated - 1) * kLogRecordsPerSector + (next_ - newestSector * kLogRecordsPerSector);
    // Anything after the newest sector is older, the ring went round
    for (uint32_t sector = newestSector + 1; sector < kLogSectorCount && !wrapped_; ++sector) {
      wrapped_ = (hasData_[sector / 8] >> (sector % 8)) & 1;
    }
  }
  // Highest boot anywhere, so the count keeps rising across the wrap
  bootCount_ = found ? static_cast<uint16_t>(lastBoot + 1) : 0;

  // The next sector always gets erased again before use, in case a reset caught it mid-erase
  ready_ = true;
  began_ = true;
  return Status::Ok;
}

// Once the chip is full this is the oldest
uint32_t FlashLog::pickSector() const
{
  return current_ == kLogNoSector ? 0 : (current_ + 1) % kLogSectorCount;
}

// Called just before the sector is erased
void FlashLog::forget(uint32_t sector)
{
  if ((hasData_[sector / 8] >> (sector % 8)) & 1) {
    hasData_[sector / 8] &= static_cast<uint8_t>(~(1U << (sector % 8)));
    used_ = used_ >= kLogRecordsPerSector ? used_ - kLogRecordsPerSector : 0;
    wrapped_ = true;
  }
}

// Collects a finished erase. One running past kFlashEraseTimeoutMs means the chip is stuck, and
// logging stops for this boot
bool FlashLog::eraseRunning()
{
  if (!erasing_) {
    return false;
  }
  if (flash_.busy()) {
    if (system_.nowMs() - eraseStartMs_ <= kFlashEraseTimeoutMs) {
      return true;
    }
    ready_ = false;
  } else {
    prepared_ = erasingSector_;
  }
  erasingSector_ = kLogNoSector;
  erasing_ = false;
  return false;
}

void FlashLog::startSector()
{
  current_ = prepared_;
  prepared_ = kLogNoSector;
  next_ = current_ * kLogRecordsPerSector;
}

bool FlashLog::slotReady()
{
  if (!ready_ || erasing_ || flash_.paused()) {
    return false;
  }
  if (current_ != kLogNoSector && next_ < (current_ + 1) * kLogRecordsPerSector) {
    return true;
  }
  if (prepared_ == kLogNoSector) {
    return false;
  }
  startSector();
  return true;
}

// A record only counts as written once it reads back as something other than all FF. A gap of FF
// would end the log early for the next boot and for the ground sketch. A slot left half written
// is used up, its CRC gives it away. Ok only when it reads back right
Status FlashLog::writeRecord(const uint8_t *bytes)
{
  const uint32_t address = next_ * kLogRecordSize;
  Status status = flash_.program(address, bytes, kLogRecordSize);
  uint8_t check[kLogRecordSize];
  const Status readBack = flash_.read(address, check, sizeof check);
  if (readBack == Status::Ok && !recordErased(check)) {
    ++next_;
    ++used_;
    hasData_[current_ / 8] |= static_cast<uint8_t>(1U << (current_ % 8));
    status = memcmp(check, bytes, kLogRecordSize) == 0 ? Status::Ok : Status::BadData;
  } else if (status == Status::Ok) {
    status = readBack == Status::Ok ? Status::Failed : readBack;
  }

  if (status == Status::Ok) {
    failures_ = 0;
  } else if (status != Status::NotReady && ++failures_ >= kLogMaxFailures) {
    ready_ = false;
  }
  return status;
}

bool FlashLog::holding() const
{
  for (uint16_t i = 0; i < ringPending_; ++i) {
    if (!dragging_ || kept(static_cast<uint16_t>((oldestSlot() + i) % kRing))) {
      return true;
    }
  }
  return false;
}

// A full ring drops its oldest, counted unless the ground had let it go anyway. On the ground only
// one packet per gap and the time and event records are kept, none with a gap of 0 except the boot
// event
void FlashLog::push(const uint8_t *bytes)
{
  if (ringPending_ == kRing) {
    const uint16_t oldest = oldestSlot();
    if (!dragging_ || kept(oldest)) {
      ++dropped_;
    }
    --ringPending_;
  }
  const uint32_t uptimeMs = get32(&bytes[kLogUptimeOffset]);
  bool keep = air_ || groundGapMs_ != 0;
  if (bytes[0] == kLogTypeEvent && bytes[kLogEventKindOffset] == kLogEventBoot) {
    keep = true;
  }
  if (bytes[0] == kLogTypePacket) {
    keep = air_ || (groundGapMs_ != 0 && (!keptAny_ || uptimeMs - keptMs_ >= groundGapMs_));
    if (keep) {
      keptAny_ = true;
      keptMs_ = uptimeMs;
    }
  }
  memcpy(ring_[ringNext_], bytes, kLogRecordSize);
  const uint8_t mask = static_cast<uint8_t>(1U << (ringNext_ % 8));
  keep_[ringNext_ / 8] = static_cast<uint8_t>(keep ? keep_[ringNext_ / 8] | mask : keep_[ringNext_ / 8] & ~mask);
  ringNext_ = static_cast<uint16_t>((ringNext_ + 1) % kRing);
  ++ringPending_;
}

// At most kLogDrainPerCall records a call. On the ground it waits until a record is about to leave
// the ring, kLogRingSlackRecords early so an erase doesn't cost a kept one
Status FlashLog::drain()
{
  Status result = Status::Ok;
  uint8_t written = 0;
  while (ringPending_ > 0 && written < kLogDrainPerCall) {
    const uint16_t at = oldestSlot();
    if (dragging_) {
      if (ringPending_ < kRing - kLogRingSlackRecords) {
        break;
      }
      if (!kept(at)) {
        --ringPending_;   // not kept
        continue;
      }
    }
    if (!slotReady()) {
      break;
    }
    result = writeRecord(ring_[at]);
    ++written;
    if (result != Status::Ok) {
      break;
    }
    --ringPending_;
  }
  if (!air_ && ringPending_ == 0) {
    dragging_ = true;   // caught up after the air
  }
  return result;
}

void FlashLog::setAir(bool air)
{
  air_ = air;
  if (air) {
    dragging_ = false;
  } else if (ringPending_ == 0) {
    dragging_ = true;
  }
}

Status FlashLog::append(const ChipSatTelemetry::TelemetryPacket &packet, uint32_t uptimeMs)
{
  if (!ready_) {
    return Status::NotReady;
  }
  LogRecord record;
  record.type = kLogTypePacket;
  record.bootCount = bootCount_;
  record.uptimeMs = uptimeMs;
  record.packet = packet;
  uint8_t bytes[kLogRecordSize];
  encodeRecord(record, bytes);
  return appendBytes(bytes);
}

// The same UTC the receiver reported, with its own validity bits kept as they came
Status FlashLog::appendTime(uint32_t uptimeMs, uint32_t utcEpoch, int32_t utcNano,
                            uint8_t utcValidBits, uint32_t utcAccNs)
{
  if (!ready_) {
    return Status::NotReady;
  }

  uint8_t bytes[kLogRecordSize] = {0};
  bytes[0] = kLogTypeTime;
  put16(&bytes[kLogBootOffset], bootCount_);
  put32(&bytes[kLogUptimeOffset], uptimeMs);
  put32(&bytes[kLogUtcEpochOffset], utcEpoch);
  put32(&bytes[kLogUtcNanoOffset], static_cast<uint32_t>(utcNano));
  put32(&bytes[kLogUtcAccOffset], utcAccNs);
  bytes[kLogUtcValidOffset] = utcValidBits;
  put16(&bytes[kLogCrcOffset], calculateCRC16(bytes, kLogCrcOffset));
  return appendBytes(bytes);
}

void FlashLog::eventBytes(uint8_t kind, uint32_t uptimeMs, const uint8_t *data, uint8_t length,
                          uint8_t *bytes) const
{
  LogRecord record;
  record.type = kLogTypeEvent;
  record.bootCount = bootCount_;
  record.uptimeMs = uptimeMs;
  record.eventKind = kind;
  memcpy(record.eventData, data, length < kLogEventDataBytes ? length : kLogEventDataBytes);
  encodeRecord(record, bytes);
}

Status FlashLog::appendEvent(uint8_t kind, uint32_t uptimeMs, const uint8_t *data, uint8_t length)
{
  if (!ready_) {
    return Status::NotReady;
  }
  uint8_t bytes[kLogRecordSize];
  eventBytes(kind, uptimeMs, data, length, bytes);
  return appendBytes(bytes);
}

Status FlashLog::appendBytes(const uint8_t *bytes)
{
  eraseRunning();
  if (!ready_) {
    return Status::Timeout;
  }
  // Waiting records first, so a ring that filled during an erase doesn't lose more
  const Status before = drain();
  if (!ready_) {
    return before;
  }
  push(bytes);
  const Status after = drain();
  return before != Status::Ok ? before : after;
}

Status FlashLog::ensureSpace()
{
  if (!ready_) {
    return Status::NotReady;
  }
  if (eraseRunning()) {
    return Status::Ok;
  }
  if (!ready_) {
    return Status::Timeout;
  }
  const Status written = drain();
  if (written != Status::Ok) {
    return written;
  }

  // Dropped count goes in the log too
  const uint32_t now = system_.nowMs();
  if (dropped_ != droppedLogged_ && now - droppedEventMs_ >= kLogDroppedEventMs) {
    uint8_t data[4];
    put32(data, dropped_);
    uint8_t bytes[kLogRecordSize];
    eventBytes(kLogEventDropped, now, data, sizeof data, bytes);
    droppedLogged_ = dropped_;
    droppedEventMs_ = now;
    push(bytes);
    drain();
  }

  if (prepared_ != kLogNoSector) {
    return Status::Ok;
  }
  const uint32_t sector = pickSector();
  const Status status = flash_.startSectorErase(sector * kFlashSectorSize);
  if (status == Status::Ok) {
    forget(sector);
    erasing_ = true;
    erasingSector_ = sector;
    eraseStartMs_ = system_.nowMs();
  }
  return status;
}

bool FlashLog::busy()
{
  if (!ready_) {
    return false;
  }
  if (eraseRunning()) {
    return true;
  }
  drain();
  return false;
}

} // namespace ChipSatApp
