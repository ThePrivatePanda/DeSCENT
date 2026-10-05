#include "FlashLog.h"
#include <string.h>

namespace ChipSatApp
{

using ChipSatPlatform::Status;
using ChipSatTelemetry::calculateCRC16;
using namespace ChipSatConstants;

static_assert(sizeof(ChipSatTelemetry::TelemetryPacket) == kLogCrcOffset - kLogPacketOffset, "the packet fills bytes 7-61");
static_assert(kFlashPageSize % kLogRecordSize == 0, "a record can't cross a page, the chip would wrap it");

void encodeRecord(const LogRecord &record, uint8_t *bytes)
{
  bytes[0] = record.type;
  bytes[kLogBootOffset] = static_cast<uint8_t>(record.bootCount);
  bytes[kLogBootOffset + 1] = static_cast<uint8_t>(record.bootCount >> 8);
  for (uint8_t i = 0; i < 4; ++i) {
    bytes[kLogUptimeOffset + i] = static_cast<uint8_t>(record.uptimeMs >> (8 * i));
  }
  memcpy(&bytes[kLogPacketOffset], &record.packet, sizeof record.packet);
  const uint16_t crc = calculateCRC16(bytes, kLogCrcOffset);
  bytes[kLogCrcOffset] = static_cast<uint8_t>(crc);
  bytes[kLogCrcOffset + 1] = static_cast<uint8_t>(crc >> 8);
}

bool decodeRecord(const uint8_t *bytes, LogRecord &record)
{
  const uint16_t crc = static_cast<uint16_t>(bytes[kLogCrcOffset] | bytes[kLogCrcOffset + 1] << 8);
  if ((bytes[0] != kLogTypePacket && bytes[0] != kLogTypeTime) ||
      crc != calculateCRC16(bytes, kLogCrcOffset)) {
    return false;
  }
  record.type = bytes[0];
  record.bootCount = static_cast<uint16_t>(bytes[kLogBootOffset] | bytes[kLogBootOffset + 1] << 8);
  record.uptimeMs = 0;
  for (uint8_t i = 0; i < 4; ++i) {
    record.uptimeMs |= static_cast<uint32_t>(bytes[kLogUptimeOffset + i]) << (8 * i);
  }
  if (record.type == kLogTypeTime) {
    record.utcEpoch = 0;
    record.utcNano = 0;
    record.utcAccNs = 0;
    for (uint8_t i = 0; i < 4; ++i) {
      record.utcEpoch |= static_cast<uint32_t>(bytes[kLogUtcEpochOffset + i]) << (8 * i);
      record.utcNano |= static_cast<int32_t>(static_cast<uint32_t>(bytes[kLogUtcNanoOffset + i]) << (8 * i));
      record.utcAccNs |= static_cast<uint32_t>(bytes[kLogUtcAccOffset + i]) << (8 * i);
    }
    record.utcBits = bytes[kLogUtcValidOffset];
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

FlashLog::FlashLog(ChipSatDevices::Flash &flash, ChipSatPlatform::System &system)
  : flash_(flash), system_(system)
{
}

Status FlashLog::begin()
{
  ready_ = false;
  erasing_ = false;
  holding_ = false;
  failures_ = 0;
  if (!flash_.ready()) {
    return Status::NotReady;
  }

  uint8_t bytes[kLogRecordSize];
  LogRecord record;
  bool found = false;
  uint16_t lastBoot = 0;

  // The log grows up from sector 0 and a sector only gets written after it's erased, so the first
  // sector that starts with an erased record is past the end. Anything left above that, like
  // flash_test's pattern in the last sector, is never looked at
  uint32_t sector = 0;
  for (; sector < kLogSectorCount; ++sector) {
    const Status status = flash_.read(sector * kFlashSectorSize, bytes, sizeof bytes);
    if (status != Status::Ok) {
      return status;
    }
    if (recordErased(bytes)) {
      break;
    }
    if (decodeRecord(bytes, record)) {
      lastBoot = record.bootCount;
      found = true;
    }
  }

  // The end is somewhere in the sector before, or that sector is full. A record that isn't all FF
  // but fails its CRC was cut off by a reset, it counts as used
  next_ = sector * kLogRecordsPerSector;
  if (sector > 0) {
    for (uint32_t index = (sector - 1) * kLogRecordsPerSector; index < sector * kLogRecordsPerSector; ++index) {
      const Status status = flash_.read(index * kLogRecordSize, bytes, sizeof bytes);
      if (status != Status::Ok) {
        return status;
      }
      if (recordErased(bytes)) {
        next_ = index;
        break;
      }
      if (decodeRecord(bytes, record)) {
        lastBoot = record.bootCount;
        found = true;
      }
    }
  }
  bootCount_ = found ? static_cast<uint16_t>(lastBoot + 1) : 0;

  // The rest of a sector with records in it was erased before its first record. An end right on a
  // sector boundary gets that sector erased again, a reset during its erase can leave it reading
  // all FF without being properly erased
  erasedUpTo_ = (next_ + kLogRecordsPerSector - 1) / kLogRecordsPerSector * kLogRecordsPerSector;
  ready_ = true;
  return Status::Ok;
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
    erasedUpTo_ += kLogRecordsPerSector;
  }
  erasing_ = false;
  return false;
}

// A record only counts as written once it reads back as something other than all FF. A gap of FF
// would end the log early for the next boot and for the ground sketch. A slot left half written
// is used up, its CRC gives it away
Status FlashLog::writeRecord(const uint8_t *bytes)
{
  const uint32_t address = next_ * kLogRecordSize;
  Status status = flash_.program(address, bytes, kLogRecordSize);
  uint8_t check[kLogRecordSize];
  const Status readBack = flash_.read(address, check, sizeof check);
  if (readBack == Status::Ok && !recordErased(check)) {
    ++next_;
    if (status == Status::Ok && memcmp(check, bytes, kLogRecordSize) != 0) {
      status = Status::BadData;
    }
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

Status FlashLog::writeHeld()
{
  if (!holding_ || erasing_ || next_ >= erasedUpTo_ || flash_.paused()) {
    return Status::Ok;
  }
  holding_ = false;
  return writeRecord(held_);
}

Status FlashLog::append(const ChipSatTelemetry::TelemetryPacket &packet, uint32_t uptimeMs)
{
  if (!ready_) {
    return Status::NotReady;
  }
  if (full()) {
    return Status::Failed;
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
  if (full()) {
    return Status::Failed;
  }

  uint8_t bytes[kLogRecordSize] = {0};
  bytes[0] = kLogTypeTime;
  bytes[kLogBootOffset] = static_cast<uint8_t>(bootCount_);
  bytes[kLogBootOffset + 1] = static_cast<uint8_t>(bootCount_ >> 8);
  for (uint8_t i = 0; i < 4; ++i) {
    bytes[kLogUptimeOffset + i] = static_cast<uint8_t>(uptimeMs >> (8 * i));
    bytes[kLogUtcEpochOffset + i] = static_cast<uint8_t>(utcEpoch >> (8 * i));
    bytes[kLogUtcNanoOffset + i] = static_cast<uint8_t>(static_cast<uint32_t>(utcNano) >> (8 * i));
    bytes[kLogUtcAccOffset + i] = static_cast<uint8_t>(utcAccNs >> (8 * i));
  }
  bytes[kLogUtcValidOffset] = utcValidBits;
  const uint16_t crc = calculateCRC16(bytes, kLogCrcOffset);
  bytes[kLogCrcOffset] = static_cast<uint8_t>(crc);
  bytes[kLogCrcOffset + 1] = static_cast<uint8_t>(crc >> 8);
  return appendBytes(bytes);
}

Status FlashLog::appendBytes(const uint8_t *bytes)
{
  eraseRunning();
  if (!ready_) {
    return Status::Timeout;
  }
  // The older one first, so records stay in order
  const Status held = writeHeld();
  if (!ready_) {
    return held;
  }
  if (full()) {
    return Status::Failed;
  }

  if (erasing_ || next_ >= erasedUpTo_ || flash_.paused()) {
    if (holding_) {
      ++dropped_;
    }
    memcpy(held_, bytes, sizeof held_);
    holding_ = true;
    return Status::Ok;
  }
  return writeRecord(bytes);
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
  const Status held = writeHeld();
  if (held != Status::Ok) {
    return held;
  }
  if (full() && holding_) {
    holding_ = false;
    ++dropped_;
  }

  if (erasedUpTo_ >= kLogRecordCount || erasedUpTo_ - next_ > kLogRecordsPerSector) {
    return Status::Ok;
  }
  const Status status = flash_.startSectorErase(erasedUpTo_ * kLogRecordSize);
  if (status == Status::Ok) {
    erasing_ = true;
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
  writeHeld();
  return false;
}

} // namespace ChipSatApp
