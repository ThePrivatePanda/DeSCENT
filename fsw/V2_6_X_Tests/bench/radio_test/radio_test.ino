// Our radio driver sending a flight-format packet a second (or back to back, kBackToBack) with a counter, compare with radio_test_library

#define CHIPSAT_RADIO_DRIVER 2   // ours
#ifndef CHIPSAT_RADIO_MODULE
#define CHIPSAT_RADIO_MODULE 1   // 1 LE, 2 HP, from the module label
#endif

#include "../../../V2_6_X_Code/src/platform/arduino/SystemArduino.cpp"
#include "../../../V2_6_X_Code/src/platform/arduino/UartArduino.cpp"
#include "../../../V2_6_X_Code/src/log/Log.cpp"
#include "../../../V2_6_X_Code/src/app/Telemetry.cpp"
#include "../../../V2_6_X_Code/src/devices/own/RadioSubGhz.cpp"

using ChipSatPlatform::Status;

ChipSatPlatform::System chipSystem;
ChipSatPlatform::Uart console(ChipSatPlatform::UartPort::Console);
ChipSatDevices::Radio radio(chipSystem);

// true sends packets back to back like flight (kTxIntervalMs 0), for the flight current
constexpr bool kBackToBack = true;

bool radioReady = false;
uint16_t counter = 0;

// The registers that decide what goes on air, same line as radio_test_library prints.
// Expected after the first packet: sync 1424, iq bit 2 set, txmod 05, clamp DE on HP / C8 on LE
void dumpRegisters(const char *when)
{
  using namespace ChipSatConstants;
  Status status = Status::Ok;
  uint8_t sync[2] = {0, 0};
  uint8_t iq = 0, txmod = 0, clamp = 0, ocp = 0;
  ChipSatDevices::readRegisters(status, "dump", kRadioRegSyncWord, sync, 2);
  ChipSatDevices::readRegisters(status, "dump", kRadioRegIqPolarity, &iq, 1);
  ChipSatDevices::readRegisters(status, "dump", kRadioRegTxModulation, &txmod, 1);
  ChipSatDevices::readRegisters(status, "dump", kRadioRegTxClamp, &clamp, 1);
  ChipSatDevices::readRegisters(status, "dump", kRadioRegOcp, &ocp, 1);
  LOG_I(Radio, "regs") {
    line.field("when", when);
    line.field("ok", status == Status::Ok);
    line.fieldHex("sync", static_cast<uint32_t>(sync[0] << 8 | sync[1]), 4);
    line.fieldHex("iq", iq, 2);
    line.fieldHex("txmod", txmod, 2);
    line.fieldHex("clamp", clamp, 2);
    line.fieldHex("ocp", ocp, 2);
  }
}

// The same four steps and log lines as FlightController::configRadio
bool setUpRadio()
{
  Status status = radio.begin();
  LOG_I(Radio, "init") {
    line.field("ok", status == Status::Ok);
    line.field("code", radio.lastCode());
  }
  if (status != Status::Ok) {
    return false;
  }
  status = radio.setCurrentLimit();
  LOG_I(Radio, "ocp") { line.field("ma", radio.currentLimitMa(), 2); }
  if (status == Status::Ok) {
    status = radio.setOutputPower();
  }
  if (status == Status::Ok) {
    status = radio.applyPaConfig();
  }
  LOG_I(Radio, "pa") {
    line.field("ok", status == Status::Ok);
    line.field("module", ChipSatConstants::kRadioModuleName);
    line.field("code", radio.lastCode());
    line.field("ocpafter", radio.currentLimitMa(), 2);
  }
  return status == Status::Ok;
}

void setup()
{
  console.begin(ChipSatConfig::kConsoleBaud);
  ChipSatLog::begin(console, chipSystem);
  LOG_I(Boot, "start") { line.field("test", "radio"); }
  radioReady = setUpRadio();
  if (radioReady) {
    dumpRegisters("setup");
  }
}

void loop()
{
  const uint32_t cycleStartMs = chipSystem.nowMs();

  if (!radioReady) {
    radioReady = setUpRadio();
  }

  if (radioReady) {
    ChipSatSensors::SensorData nothing;
    ChipSatTelemetry::TelemetryPacket packet;
    ChipSatTelemetry::encodePacket(nothing, counter, ChipSatConfig::kChipSatId, false, packet);

    // Same wait as FlightController::transmit
    const uint32_t startMs = chipSystem.nowMs();
    Status status = radio.startTransmit(reinterpret_cast<const uint8_t *>(&packet), sizeof packet);
    bool done = false;
    if (status == Status::Ok) {
      const uint32_t timeoutMs = 5 + 5 * radio.timeOnAirMs(sizeof packet);
      while (!radio.transmitDone() && chipSystem.nowMs() - startMs <= timeoutMs) {
        chipSystem.waitMs(1);
      }
      done = radio.transmitDone();
      status = radio.finishTransmit();
      if (!done) {
        status = Status::Timeout;
      }
    }

    LOG_I(Tx, "done") {
      line.field("ok", status == Status::Ok);
      line.field("ctr", static_cast<unsigned int>(counter));
      line.field("status", ChipSatPlatform::statusName(status));
      line.field("code", radio.lastCode());
      line.field("ms", chipSystem.nowMs() - startMs);
    }
    if (counter == 0 && status == Status::Ok) {
      dumpRegisters("firsttx");
    }
    if (status != Status::Ok) {
      radioReady = false;
    }
    counter++;
  }

  const uint32_t elapsedMs = chipSystem.nowMs() - cycleStartMs;
  if (!kBackToBack && elapsedMs < 1000) {
    chipSystem.waitMs(1000 - elapsedMs);
  }
}
