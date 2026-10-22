#pragma once

#include <stdint.h>
#include "../config.h"
#include "../constants.h"
#include "../devices/Flash.h"
#include "../platform/Status.h"
#include "../platform/System.h"
#include "Telemetry.h"

namespace ChipSatApp
{

// One 64 byte record per packet, little endian:
//   0      type: kLogTypePacket, kLogTypeTime for a time anchor, kLogTypeEvent for an event. Never
//          FF, so a written record is never all FF. An unknown type fails decodeRecord and reads as
//          a damaged slot, so a new type needs every reader updated
//   1-2    boot count
//   3-6    ms since boot
//   7-61   packet record: a 55 byte packet built the same way as one that goes out, carrying
//          the counter of the last one that did, with its own CRC
//          time record: utc seconds since 1970 (u32), the fraction of that second in ns (i32), the
//          receiver's own validity bits (u8), its time accuracy estimate in ns (u32), then zeros
//          event record: its kind (u8), then 54 bytes that kind lays out, zeros where unused
//   62-63  ChipSatTelemetry::calculateCRC16 over bytes 0-61
// The first 7 bytes and the CRC are in the same place whatever the type, so one reader handles all
constexpr uint8_t kLogRecordSize = 64;
constexpr uint8_t kLogTypePacket = 0xA5;
constexpr uint8_t kLogTypeTime = 0xA6;
constexpr uint8_t kLogTypeEvent = 0xA7;
constexpr uint8_t kLogBootOffset = 1;
constexpr uint8_t kLogUptimeOffset = 3;
constexpr uint8_t kLogPacketOffset = 7;
constexpr uint8_t kLogCrcOffset = 62;
constexpr uint8_t kLogUtcEpochOffset = 7;
constexpr uint8_t kLogUtcNanoOffset = 11;
constexpr uint8_t kLogUtcValidOffset = 15;
constexpr uint8_t kLogUtcAccOffset = 16;
constexpr uint8_t kLogEventKindOffset = 7;
constexpr uint8_t kLogEventDataOffset = 8;
constexpr uint8_t kLogEventDataBytes = kLogCrcOffset - kLogEventDataOffset;   // 54
constexpr uint32_t kLogRecordsPerSector = ChipSatConstants::kFlashSectorSize / kLogRecordSize;
constexpr uint32_t kLogSectorCount = ChipSatConstants::kFlashSize / ChipSatConstants::kFlashSectorSize;
constexpr uint32_t kLogRecordCount = ChipSatConstants::kFlashSize / kLogRecordSize;
constexpr uint8_t kLogMaxFailures = 3;   // failed writes in a row before logging stops for this boot
constexpr uint16_t kLogNoSector = 0xFFFF;

// Event kinds, byte 7 of an event record. The data after it, little endian:
//   boot     0-3 RCC CSR before it was cleared: bit 24 RFILAR, 25 OBL, 26 PIN, 27 BOR, 28 SFT,
//            29 IWDG, 30 WWDG, 31 LPWR. Once a boot, kept even with a ground gap of 0
//   2        was the flight lock, not written any more. Readers skip it
//   dropped  0-3 records lost this boot so far (ring overran in the air, or a kept ground record
//            couldn't be written in time). Written when it grows, at most every kLogDroppedEventMs
// 0x10 to 0x3F are for the sensor and bus recovery events
constexpr uint8_t kLogEventBoot = 1;
constexpr uint8_t kLogEventDropped = 3;
constexpr uint32_t kLogDroppedEventMs = 10000;

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
  uint8_t eventKind = 0;                      // kLogTypeEvent
  uint8_t eventData[kLogEventDataBytes] = {};
};

void encodeRecord(const LogRecord &record, uint8_t *bytes);
bool decodeRecord(const uint8_t *bytes, LogRecord &record);   // false on a bad CRC or unknown type
bool recordErased(const uint8_t *bytes);

// A sector's age is that of its first record that decodes, so a cut off first write doesn't lose it
struct SectorHead
{
  bool erased = true;      // its first record is all FF
  bool dated = false;      // a record in it decodes, and boot and ms are the first such one's
  uint16_t boot = 0;
  uint32_t ms = 0;
};

ChipSatPlatform::Status readSectorHead(ChipSatDevices::Flash &flash, uint32_t sector, SectorHead &head);
bool headOlder(const SectorHead &a, const SectorHead &b);   // by boot, then ms. Undated is oldest

// Ring over the whole chip, erasing one sector ahead of the one being written (the oldest once the
// chip is full). The end after a reset is wherever the newest records are, so readers, flash_dump
// too, order sectors by the boot and uptime of their first record. Records have to go onto the chip
// in time order for that to work. Nothing in here erases the whole chip, the ground sketch does that
//
// A RAM ring of the last kLogRingRecords records sits in front of the chip. In the air it's written
// whenever the chip is free, so an erase or the HP pause during TX only loses records if the ring
// laps. On the ground only one packet per ground gap gets written, so the ring still holds the
// last 30 s when the take-off comes and all of it goes onto the chip then
class FlashLog
{
public:
  FlashLog(ChipSatDevices::Flash &flash, ChipSatPlatform::System &system);

  // Only reads, about 580 reads of 64 bytes. Empties the ring
  ChipSatPlatform::Status begin();

  // Also Ok when it only went into the ring (erasing, paused, on the ground). A failed write is
  // tried again on the next call
  ChipSatPlatform::Status append(const ChipSatTelemetry::TelemetryPacket &packet, uint32_t uptimeMs);

  // Ties this boot's uptime to UTC. Written when the GPS first has usable time and every so often
  // after, so the ground can put a clock on every record of the boot, including the earlier ones
  ChipSatPlatform::Status appendTime(uint32_t uptimeMs, uint32_t utcEpoch, int32_t utcNano,
                                     uint8_t utcValidBits, uint32_t utcAccNs);

  // Rest of the data zeroed
  ChipSatPlatform::Status appendEvent(uint8_t kind, uint32_t uptimeMs, const uint8_t *data, uint8_t length);

  // Going to air writes the whole ring, oldest first. Starts in the air so nothing is dropped
  // before the controller says otherwise
  void setAir(bool air);

  // One packet record per this many ms is kept on the ground, plus time and event records. 0 keeps
  // only the boot event. Goes by the record's own uptime
  void setGroundGapMs(uint32_t ms) { groundGapMs_ = ms; }

  // Starts erasing the next sector if the log needs it and returns straight away. Keeps one erased
  // sector ahead of the end. Also writes from the ring once the chip is free. Call it every cycle
  ChipSatPlatform::Status ensureSpace();
  bool busy();   // an erase is still running. Writes from the ring if it's done

  bool ready() const { return ready_; }
  bool stopped() const { return began_ && !ready_; }   // stuck chip or failed writes
  bool full() const { return stopped(); }   // old name, a ring is never full. Until FlightController uses stopped()
  bool air() const { return air_; }
  bool holding() const;                                // records in the ring still to be written
  uint32_t waiting() const { return ringPending_; }    // records from the cursor on, kept or not
  bool wrapped() const { return wrapped_; }            // a sector with records in it has been reused
  uint32_t records() const { return used_; }           // slots holding something, bad ones included
  uint16_t bootCount() const { return bootCount_; }    // counts boots that logged something, 0 on an empty chip
  uint32_t dropped() const { return dropped_; }

private:
  static_assert(ChipSatConfig::kLogRingRecords > ChipSatConfig::kLogRingSlackRecords &&
                ChipSatConfig::kLogRingRecords < 0xFFFF, "the ring needs room past its slack, and fits a uint16_t");
  static constexpr uint16_t kRing = static_cast<uint16_t>(ChipSatConfig::kLogRingRecords);

  bool eraseRunning();
  bool slotReady();
  void startSector();
  void forget(uint32_t sector);
  uint32_t pickSector() const;
  uint16_t oldestSlot() const { return static_cast<uint16_t>((ringNext_ + kRing - ringPending_) % kRing); }
  bool kept(uint16_t slot) const { return (keep_[slot / 8] >> (slot % 8)) & 1; }
  void push(const uint8_t *bytes);
  ChipSatPlatform::Status drain();
  ChipSatPlatform::Status writeRecord(const uint8_t *bytes);
  ChipSatPlatform::Status appendBytes(const uint8_t *bytes);
  void eventBytes(uint8_t kind, uint32_t uptimeMs, const uint8_t *data, uint8_t length, uint8_t *bytes) const;

  ChipSatDevices::Flash &flash_;
  ChipSatPlatform::System &system_;
  bool ready_ = false;
  bool began_ = false;
  uint32_t current_ = kLogNoSector;   // the sector being written, kLogNoSector before the first
  uint32_t next_ = 0;                 // index of the next record, inside current_ or just past it
  uint32_t prepared_ = kLogNoSector;  // erased and waiting to be the next current_
  uint32_t erasingSector_ = kLogNoSector;
  bool erasing_ = false;
  uint32_t eraseStartMs_ = 0;
  uint16_t bootCount_ = 0;
  uint32_t dropped_ = 0;
  uint32_t droppedLogged_ = 0;
  uint32_t droppedEventMs_ = 0;
  uint8_t failures_ = 0;
  uint32_t used_ = 0;
  bool wrapped_ = false;
  uint8_t hasData_[kLogSectorCount / 8] = {};

  // 38.4 KB, most of the RAM, so never on the stack
  uint8_t ring_[kRing][kLogRecordSize];
  uint8_t keep_[(kRing + 7) / 8] = {};   // per slot: kept on the ground
  uint16_t ringNext_ = 0;                // where the next record goes
  uint16_t ringPending_ = 0;             // from the cursor to the newest
  bool air_ = true;
  bool dragging_ = false;                // on the ground and caught up, only kept records get written
  uint32_t groundGapMs_ = 0;
  bool keptAny_ = false;
  uint32_t keptMs_ = 0;                  // the uptime of the last packet record kept
};

} // namespace ChipSatApp
