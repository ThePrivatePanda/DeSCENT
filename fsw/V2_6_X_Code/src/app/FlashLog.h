#pragma once

#include <stdint.h>
#include "../constants.h"
#include "../devices/Flash.h"
#include "../platform/Status.h"
#include "../platform/System.h"
#include "Telemetry.h"

namespace ChipSatApp
{

// One 64 byte record per packet, little endian:
//   0      type: kLogTypePacket, or kLogTypeTime for a time anchor. Never FF, so a written record
//          is never all FF. A reader that doesn't know a type skips 64 bytes and carries on
//   1-2    boot count
//   3-6    ms since boot
//   7-61   packet record: a 55 byte packet built the same way as one that goes out, carrying
//          the counter of the last one that did, with its own CRC
//          time record: utc seconds since 1970 (u32), the fraction of that second in ns (i32), the
//          receiver's own validity bits (u8), its time accuracy estimate in ns (u32), then zeros
//   62-63  ChipSatTelemetry::calculateCRC16 over bytes 0-61
// The first 7 bytes and the CRC are in the same place whatever the type, so one reader handles both
constexpr uint8_t kLogRecordSize = 64;
constexpr uint8_t kLogTypePacket = 0xA5;
constexpr uint8_t kLogTypeTime = 0xA6;
constexpr uint8_t kLogBootOffset = 1;
constexpr uint8_t kLogUptimeOffset = 3;
constexpr uint8_t kLogPacketOffset = 7;
constexpr uint8_t kLogCrcOffset = 62;
constexpr uint8_t kLogUtcEpochOffset = 7;
constexpr uint8_t kLogUtcNanoOffset = 11;
constexpr uint8_t kLogUtcValidOffset = 15;
constexpr uint8_t kLogUtcAccOffset = 16;
constexpr uint32_t kLogRecordsPerSector = ChipSatConstants::kFlashSectorSize / kLogRecordSize;
constexpr uint32_t kLogSectorCount = ChipSatConstants::kFlashSize / ChipSatConstants::kFlashSectorSize;
constexpr uint32_t kLogRecordCount = ChipSatConstants::kFlashSize / kLogRecordSize;
constexpr uint8_t kLogMaxFailures = 3;   // failed writes in a row before logging stops for this boot

struct LogRecord
{
  uint8_t type = 0;
  uint16_t bootCount = 0;
  uint32_t uptimeMs = 0;
  ChipSatTelemetry::TelemetryPacket packet;   // kLogTypePacket
  uint32_t utcEpoch = 0;                      // kLogTypeTime
  int32_t utcNano = 0;
  uint8_t utcBits = 0;
  uint32_t utcAccNs = 0;
};

void encodeRecord(const LogRecord &record, uint8_t *bytes);
bool decodeRecord(const uint8_t *bytes, LogRecord &record);   // false on a bad CRC or unknown type
bool recordErased(const uint8_t *bytes);

// Append-only log from the start of the chip. Stops when full, never wraps. Nothing in here erases
// the chip, the ground sketch does that before a flight
class FlashLog
{
public:
  FlashLog(ChipSatDevices::Flash &flash, ChipSatPlatform::System &system);

  // Only reads. The end is the first all FF record: the first record of each sector, then every
  // record of the last used one. About 580 reads of 64 bytes
  ChipSatPlatform::Status begin();

  // Also Ok when the record has to wait in RAM: during an erase, before the first sector is erased
  // or while the chip is paused. There's room for one, a newer one replaces it and counts as dropped
  ChipSatPlatform::Status append(const ChipSatTelemetry::TelemetryPacket &packet, uint32_t uptimeMs);

  // Ties this boot's uptime to UTC. Written when the GPS first has usable time and every so often
  // after, so the ground can put a clock on every record of the boot, including the earlier ones
  ChipSatPlatform::Status appendTime(uint32_t uptimeMs, uint32_t utcEpoch, int32_t utcNano,
                                     uint8_t utcValidBits, uint32_t utcAccNs);

  // Starts erasing the next sector if the log needs it and returns straight away. Keeps one erased
  // sector ahead of the end. Also writes a waiting record once the chip is free. Call it every cycle
  ChipSatPlatform::Status ensureSpace();
  bool busy();   // an erase is still running. Writes a waiting record if it's done

  bool ready() const { return ready_; }
  bool full() const { return next_ >= kLogRecordCount; }
  bool holding() const { return holding_; }
  uint32_t records() const { return next_; }   // bad ones included
  uint16_t bootCount() const { return bootCount_; }   // counts boots that logged something, 0 on an empty chip
  uint32_t dropped() const { return dropped_; }

private:
  bool eraseRunning();
  ChipSatPlatform::Status writeHeld();
  ChipSatPlatform::Status writeRecord(const uint8_t *bytes);
  ChipSatPlatform::Status appendBytes(const uint8_t *bytes);

  ChipSatDevices::Flash &flash_;
  ChipSatPlatform::System &system_;
  bool ready_ = false;
  uint32_t next_ = 0;         // index of the next record
  uint32_t erasedUpTo_ = 0;   // records from next_ up to here are erased
  bool erasing_ = false;
  uint32_t eraseStartMs_ = 0;
  uint16_t bootCount_ = 0;
  bool holding_ = false;
  uint8_t held_[kLogRecordSize];
  uint32_t dropped_ = 0;
  uint8_t failures_ = 0;
};

} // namespace ChipSatApp
