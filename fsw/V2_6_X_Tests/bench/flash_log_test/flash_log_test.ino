// Our flash driver and log on the board: finds the end of the log, adds 300 records 10 ms apart the way the flight
// loop would (erase started ahead, a record waits in RAM while it runs), then finds the end again and reads the new
// records back. Cut the power whenever you like, the next boot carries on from the end. flash_dump prints the
// whole log. Console PB7/PB6 at 115200

#include "../../../V2_6_X_Code/src/platform/arduino/SystemArduino.cpp"
#include "../../../V2_6_X_Code/src/platform/arduino/UartArduino.cpp"
#include "../../../V2_6_X_Code/src/log/Log.cpp"
#include "../../../V2_6_X_Code/src/app/Telemetry.cpp"
#include "../../../V2_6_X_Code/src/app/FlashLog.cpp"
#include "../../../V2_6_X_Code/src/devices/own/FlashW25Q.cpp"

using ChipSatPlatform::Status;
using ChipSatPlatform::statusName;

ChipSatPlatform::System chipSystem;
ChipSatPlatform::Uart console(ChipSatPlatform::UartPort::Console);
ChipSatDevices::Flash flash(chipSystem);
ChipSatApp::FlashLog flashLog(flash, chipSystem);

constexpr uint16_t kRecords = 300;
constexpr uint32_t kSpacingMs = 10;   // faster than flight, so some records have to wait for an erase
constexpr uint16_t kPauseEvery = 50;
constexpr uint32_t kPauseMs = 100;

void mount(const char *when)
{
  const uint32_t startMs = chipSystem.nowMs();
  const Status status = flashLog.begin();
  LOG_I(Flash, "mount") {
    line.field("when", when);
    line.field("ok", status == Status::Ok);
    line.field("status", statusName(status));
    line.field("records", flashLog.records());
    line.field("boot", static_cast<unsigned int>(flashLog.bootCount()));
    line.field("stopped", flashLog.stopped());
    line.field("ms", chipSystem.nowMs() - startMs);
  }
}

void setup()
{
  console.begin(ChipSatConfig::kConsoleBaud);
  ChipSatLog::begin(console, chipSystem);
  LOG_I(Boot, "start") { line.field("test", "flashlog"); }

  if (flash.begin() != Status::Ok) {
    return;   // the init line says why
  }
  mount("boot");
  if (!flashLog.ready()) {
    return;
  }
  const uint32_t firstRecord = flashLog.records();
  const uint16_t boot = flashLog.bootCount();

  uint16_t failed = 0;
  uint32_t slowestAppendMs = 0;
  bool erasing = false;
  uint32_t eraseStartMs = 0;
  for (uint16_t i = 0; i < kRecords && !flashLog.stopped(); ++i) {
    ChipSatSensors::SensorData nothing;
    ChipSatTelemetry::TelemetryPacket packet;
    ChipSatTelemetry::encodePacket(nothing, i, ChipSatConfig::kChipSatId, false, packet);

    const uint32_t appendStartMs = chipSystem.nowMs();
    Status status = flashLog.append(packet, appendStartMs);
    const uint32_t appendMs = chipSystem.nowMs() - appendStartMs;
    if (appendMs > slowestAppendMs) {
      slowestAppendMs = appendMs;
    }
    if (status != Status::Ok) {
      ++failed;
      LOG_W(Flash, "append") {
        line.field("ok", false);
        line.field("i", static_cast<unsigned int>(i));
        line.field("status", statusName(status));
      }
    }

    // Only as exact as the 10 ms between records
    if (erasing && !flashLog.busy()) {
      erasing = false;
      LOG_I(Flash, "erased") {
        line.field("at", flashLog.records());
        line.field("ms", chipSystem.nowMs() - eraseStartMs);
      }
    }
    status = flashLog.ensureSpace();
    if (status != Status::Ok) {
      LOG_W(Flash, "space") {
        line.field("ok", false);
        line.field("status", statusName(status));
      }
    }
    if (!erasing && flashLog.busy()) {
      erasing = true;
      eraseStartMs = chipSystem.nowMs();
    }

    // The way the flight loop powers it down around a transmit. Refused while an erase runs
    if (i % kPauseEvery == kPauseEvery - 1) {
      const Status paused = flash.pause();
      chipSystem.waitMs(kPauseMs);
      const uint32_t resumeStartMs = chipSystem.nowMs();
      const Status resumed = flash.resume();
      LOG_I(Flash, "pause") {
        line.field("pause", statusName(paused));
        line.field("resume", statusName(resumed));
        line.field("resumems", chipSystem.nowMs() - resumeStartMs);
      }
    }
    chipSystem.waitMs(kSpacingMs);
  }

  // Let the last erase finish so a waiting record goes in
  const uint32_t waitStartMs = chipSystem.nowMs();
  while (flashLog.busy() && chipSystem.nowMs() - waitStartMs < ChipSatConstants::kFlashEraseTimeoutMs) {
    chipSystem.waitMs(1);
  }
  LOG_I(Flash, "written") {
    line.field("records", flashLog.records());
    line.field("new", flashLog.records() - firstRecord);
    line.field("dropped", flashLog.dropped());
    line.field("failed", static_cast<unsigned int>(failed));
    line.field("waiting", flashLog.holding());
    line.field("slowestms", slowestAppendMs);
  }

  const uint32_t endBefore = flashLog.records();
  mount("again");
  LOG_I(Flash, "end") {
    line.field("same", flashLog.records() == endBefore);
    line.field("records", flashLog.records());
    line.field("address", flashLog.records() * ChipSatApp::kLogRecordSize);
  }

  // This run's records: CRC good, this boot's count, counters going up
  uint32_t good = 0;
  uint32_t bad = 0;
  uint32_t outOfOrder = 0;
  int32_t lastCounter = -1;
  for (uint32_t index = firstRecord; index < flashLog.records(); ++index) {
    uint8_t bytes[ChipSatApp::kLogRecordSize];
    ChipSatApp::LogRecord record;
    if (flash.read(index * ChipSatApp::kLogRecordSize, bytes, sizeof bytes) != Status::Ok ||
        !ChipSatApp::decodeRecord(bytes, record) || record.bootCount != boot) {
      ++bad;
      continue;
    }
    ++good;
    if (static_cast<int32_t>(record.packet.packetCounter) <= lastCounter) {
      ++outOfOrder;
    }
    lastCounter = record.packet.packetCounter;
  }
  LOG_I(Flash, "check") {
    line.field("good", good);
    line.field("bad", bad);
    line.field("order", outOfOrder == 0);
  }
}

void loop()
{
}
