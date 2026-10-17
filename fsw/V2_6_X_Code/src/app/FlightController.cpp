#include <Arduino.h>
#include <math.h>
#include "FlightController.h"
#include "ImuValidity.h"
#include "TxInterval.h"
#include "../Settings.h"
#include "../config.h"
#include "../constants.h"
#include "../log/Log.h"

namespace ChipSatApp
{

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

[[maybe_unused]] static uint8_t imuValidBits(const ChipSatSensors::IMUData &imu)
{
  return (imu.linearAccelerationValid ? 1U : 0U) | (imu.gyroscopeValid ? 2U : 0U) |
         (imu.magnetometerValid ? 4U : 0U) | (imu.orientationValid ? 8U : 0U);
}

FlightController::FlightController(ChipSatPlatform::System &system,
                                   ChipSatPlatform::I2cBus &i2c,
                                   ChipSatPlatform::Uart &console,
                                   ChipSatPlatform::Uart &gpsPort,
                                   ChipSatDevices::Imu &imu,
                                   ChipSatDevices::Gps &gps,
                                   ChipSatDevices::EnvSensor &env,
                                   ChipSatDevices::FuelGauge &gauge,
                                   ChipSatDevices::Radio &radio,
                                   ChipSatDevices::Led &led,
                                   ChipSatDevices::Flash &flash,
                                   FlashLog &flashLog)
  : system_(system), i2c_(i2c), console_(console), gpsPort_(gpsPort), imu_(imu), gps_(gps), env_(env),
    gauge_(gauge), radio_(radio), led_(led), flash_(flash), flashLog_(flashLog),
    txIntervalMs_(kTxIntervalMs)
{
}

// The gap between transmissions is measured from the end of one to the start of the next: the cycle
// stamps its clock after the radio is done, so an interval is time off air and nothing needs
// clamping to the time on air. 0 therefore means back to back

void FlightController::setup()
{
  led_.begin();

  console_.begin(kConsoleBaud);
  ChipSatLog::begin(console_, system_);

  LOG_BOOT(Boot, "start") { line.field("log", CHIPSAT_LOG_LEVEL); }

  // Before anything is configured, because the radio and the packets both need to know which board
  // this is. Reads the settings block if a tool wrote one, and stands on the compiled defaults if not
  ChipSatSettings::begin();
  LOG_BOOT(Boot, "settings") {
    const ChipSatSettings::Settings& s = ChipSatSettings::active();
    line.field("from", ChipSatSettings::source() == ChipSatSettings::Source::kBlock ? "block" : "build");
    line.field("csid", s.csid);
    line.field("module", s.module == ChipSatSettings::kModuleHp ? "hp" : "le");
    line.field("sf", s.spreadingFactor);
    line.field("dbm", s.targetDbm);
    // one gap per phase: the fixed one, or 'soc' for one that follows the battery
    uint32_t fixed[ChipSatSettings::kPhases];
    uint32_t soc[ChipSatSettings::kPhases];
    for (size_t p = 0; p < ChipSatSettings::kPhases; p++) {
      fixed[p] = s.rate[p].fixedMs;
      soc[p] = s.rate[p].fromSoc ? 1 : 0;
    }
    line.fieldList("fixedms", fixed, ChipSatSettings::kPhases);
    line.fieldList("soc", soc, ChipSatSettings::kPhases);
    // only when something was wrong with a block that was actually written, so a plain image is quiet
    if (ChipSatSettings::rejectReason() != nullptr) {
      line.field("refused", ChipSatSettings::rejectReason());
    }
  }
  logResetCause();
  system_.startWatchdog(kWatchdogTimeoutMs);

  // Radio before the sensors, so a sensor that hangs at boot still leaves one packet on air.
  // Never halts, a missing sensor just goes out invalid
  radioReady_ = configRadio();
  txIntervalMs_ = ChipSatSettings::active().rate[0].fixedMs;   // it starts waiting
  sendStatusPacket();

  beginSensors();

  LOG_BOOT(Boot, "setup") {
    line.field("done", true);
    line.field("txms", txIntervalMs_);                          // what it will actually use
    line.field("radiostate", radio_.lastCode());
  }
}

void FlightController::logResetCause()
{
  const ChipSatPlatform::ResetCause cause = system_.readAndClearResetCause();

  LOG_BOOT(Rst, "cause") {
    line.fieldHex("csr", cause.csr, 8);
    line.field("lpwr", cause.lowPower);
    line.field("wwdg", cause.windowWatchdog);
    line.field("iwdg", cause.independentWatchdog);
    line.field("sft", cause.software);
    line.field("bor", cause.brownOut);
    line.field("pin", cause.pin);
    line.field("obl", cause.optionByteLoad);
    line.field("rfilar", cause.radioIllegalAccess);
  }
}

void FlightController::beginSensors()
{
  i2c_.begin(kI2cSdaPin, kI2cSclPin);
  gpsPort_.begin(kGpsBaud);   // before the IMU

  if (flash_.begin() == Status::Ok) {
    flashLog_.begin();
    // two erases at boot, so the first records aren't held: the end sector, then the next
    for (uint8_t i = 0; i < 2; ++i) {
      flashLog_.ensureSpace();
      const uint32_t startMs = system_.nowMs();
      while (flashLog_.busy() && system_.nowMs() - startMs < kFlashEraseTimeoutMs) {
        system_.waitMs(1);
      }
    }
    LOG_I(Flash, "log") {
      line.field("records", flashLog_.records());
      line.field("boot", flashLog_.bootCount());
      line.field("full", flashLog_.full());
    }
  }

  imu_.begin(kImuReportIntervalMs);
  keepAlive();
  gps_.begin();
  keepAlive();
  gauge_.begin();
  keepAlive();
  env_.begin();
}

// Whatever is known so far, all fresh bits clear
void FlightController::sendStatusPacket()
{
  ChipSatTelemetry::encodePacket(data_, packetCounter_, ChipSatSettings::active().csid, false, packet_);
  sentCounter_ = packetCounter_;
  packetCounter_++;
  LOG_PACKET(reinterpret_cast<const uint8_t *>(&packet_), sizeof(packet_));
  transmit();
}

// A slow sensor start after a reboot mid-flight shouldn't leave the air quiet
void FlightController::keepAlive()
{
  if (system_.nowMs() - lastTxMs_ >= kBootKeepAliveMs) {
    sendStatusPacket();
  }
}

bool FlightController::configRadio()
{
  LOG_I(Radio, "start");
  Status status = radio_.begin();
  LOG_D(Radio, "ldro") { line.field("code", radio_.ldroCode()); }
  if (status != Status::Ok) {
    LOG_E(Radio, "failed") {
      line.field("reason", "radiobegin");
      line.field("code", radio_.lastCode());
    }
    return false;
  }
  LOG_I(Radio, "init") {
    line.field("ok", true);
    line.field("code", radio_.lastCode());
  }

  status = radio_.setCurrentLimit();
  if (status != Status::Ok) {
    LOG_E(Radio, "failed") {
      line.field("reason", "radioocp");
      line.field("code", radio_.lastCode());
    }
    return false;
  }
  status = radio_.setOutputPower();
  if (status != Status::Ok) {
    LOG_E(Radio, "failed") {
      line.field("reason", "radiopower");
      line.field("code", radio_.lastCode());
    }
    return false;
  }
  LOG_I(Radio, "power") { line.field("ok", true); }

  status = radio_.applyPaConfig();
  if (status != Status::Ok) {
    LOG_E(Radio, "failed") {
      line.field("reason", "radiopa");
      line.field("code", radio_.lastCode());
    }
    return false;
  }
  LOG_I(Radio, "pa") {
    line.field("ok", true);
    const ChipSatSettings::Settings& s = ChipSatSettings::active();
    const PaSettings pa = s.module == ChipSatSettings::kModuleHp ? paSettingsHighPower(s.targetDbm)
                                                                 : paSettingsLowPower(s.targetDbm);
    line.field("module", s.module == ChipSatSettings::kModuleHp ? "hp" : "le");
    line.field("want", s.targetDbm);  // what was asked for
    line.field("dbm", pa.txParamsDbm);   // what SetTxParams gets, the PA row decides the real output
    line.field("code", radio_.lastCode());
  }
  // SPI read, only compiled in with this log line. After the PA step, which resets it
  LOG_I(Radio, "ocp") { line.field("ma", radio_.currentLimitMa(), 2); }
  return true;
}

void FlightController::loop()
{
  system_.feedWatchdog();
  imu_.service();
  gps_.service();
  sampleSensors();
  logIfDue();

  if (transmitDue()) {
    runCycle();
  }
}

// Each sensor at its own pace. Nothing here waits for the radio, and nothing waits for a reading it
// hasn't got yet
void FlightController::sampleSensors()
{
  const uint32_t now = system_.nowMs();

  if (!envMeasuring_) {
    if (now - envStartedMs_ >= kEnvIntervalMs) {
      envMeasuring_ = env_.startMeasurement() == Status::Ok;
      envStartedMs_ = now;
    }
  } else if (now - envStartedMs_ >= kEnvConversionMs) {
    env_.read();   // the conversion is over, so this doesn't wait
    envMeasuring_ = false;
  }

  if (now - gpsReadMs_ >= kGpsIntervalMs) {
    gps_.read();
    gpsReadMs_ = now;
  }

  if (now - gaugeReadMs_ >= kGaugeIntervalMs) {
    gauge_.read();
    gaugeReadMs_ = now;
  }
  followPhase();
}

// Every fresh reading goes to the phase as it arrives, IMU at 50 ms and barometer at 200 ms, so a
// release is felt on the reading that shows it and not on the next log tick. A phase change
// makes the next packet due at once: the first thing a dropped sat should do is say so
void FlightController::followPhase()
{
  const uint32_t now = system_.nowMs();
  const ChipSatSensors::IMUData imu = imu_.data();
  if (imu.linearAccelerationValid && imu.gyroscopeValid && imu.linearAccelerationUpdatedMs != imuSeenMs_) {
    imuSeenMs_ = imu.linearAccelerationUpdatedMs;
    const ChipSatSensors::Vector3f &a = imu.linearAccelerationMps2;
    const ChipSatSensors::Vector3f &g = imu.gyroscopeRadPerSec;
    phase_.imuSample(now, sqrtf(a.x * a.x + a.y * a.y + a.z * a.z), sqrtf(g.x * g.x + g.y * g.y + g.z * g.z));
  } else if (now - imuSeenMs_ > kImuMaxAgeMs) {
    phase_.imuLost(now);
  }
  const ChipSatSensors::EnvironmentalData env = env_.data();
  if (env.valid && env.updatedMs != envSeenMs_) {
    envSeenMs_ = env.updatedMs;
    phase_.envSample(now, env.altitudeM);
  }
  // GPS altitude stands in for a dead barometer. The driver's valid is a trusted 3D fix, fresh
  const ChipSatSensors::GPSData gps = gps_.data();
  if (gps.valid && gps.updatedMs != gpsSeenMs_) {
    gpsSeenMs_ = gps.updatedMs;
    phase_.gpsSample(now, static_cast<float>(gps.altitudeMSLmm) / 1000.0f);
  }
  if (phase_.changed()) {
    LOG_I(Phase, "now") {
      line.field("phase", static_cast<unsigned int>(phase_.phase()));
      line.field("source", static_cast<unsigned int>(phase_.altitudeSource()));
      line.field("rate", phase_.verticalRateMps(), 2);
      line.field("above", phase_.aboveGroundM(), 1);
    }
    txIntervalMs_ = phaseTxIntervalMs(gauge_.data().valid);
    previousReadMs_ = now - txIntervalMs_;   // due now
  }
}

// The gap after a packet in this phase: its fixed gap, or its battery table when it follows the
// battery and the gauge reads
uint32_t FlightController::phaseTxIntervalMs(bool socOkay)
{
  const ChipSatSettings::Rate &rate = ChipSatSettings::active().rate[static_cast<size_t>(phase_.phase())];
  if (rate.fromSoc && socOkay) {
    return txIntervalFromSoc(rate, gauge_.data().cellPercentage);
  }
  return rate.fixedMs;
}

// The newest reading each sensor has, with anything too old marked invalid
void FlightController::buildPacket(ChipSatTelemetry::TelemetryPacket &into, bool &allFresh,
                                   uint16_t counter)
{
  const uint32_t now = system_.nowMs();

  data_.imu = imu_.data();
  data_.gps = gps_.data();
  data_.stateOfCharge = gauge_.data();
  data_.environmental = env_.data();

  clearStaleImuReports(data_.imu, now, kImuMaxAgeMs);
  if (now - data_.environmental.updatedMs > kEnvMaxAgeMs) {
    data_.environmental.valid = false;
  }
  if (now - data_.stateOfCharge.updatedMs > kGaugeMaxAgeMs) {
    data_.stateOfCharge.valid = false;
  }
  // the GPS driver already refuses anything older than kGpsPvtMaxAgeMs

  allFresh = data_.imu.linearAccelerationValid && data_.imu.gyroscopeValid &&
             data_.imu.magnetometerValid && data_.imu.orientationValid &&
             data_.gps.valid && data_.stateOfCharge.valid && data_.environmental.valid;

  ChipSatTelemetry::encodePacket(data_, counter, ChipSatSettings::active().csid, allFresh, into);
}

// One record per log gap: fast in the air, slow on the ground either side
void FlightController::logIfDue()
{
  // A full chip is the end of the mission's logging, not a fault. Stop building records for it and
  // leave the cycle to the radio, which is what matters once there's nowhere left to write
  if (flashLog_.full()) {
    if (!logFull_) {
      logFull_ = true;
      LOG_W(Flash, "full") {
        line.field("records", flashLog_.records());
        line.field("dropped", flashLog_.dropped());
      }
    }
    return;
  }

  // The gap this phase was given, or the blind one while nothing says what the phase is: the IMU
  // alone cannot tell a hover from the ground, and a chip full on the drone is a flight lost
  const uint32_t now = system_.nowMs();
  const ChipSatSettings::Settings &settings = ChipSatSettings::active();
  const uint32_t interval = phase_.altitudeSource() == AltitudeSource::None
    ? settings.logBlindMs
    : settings.logMs[static_cast<size_t>(phase_.phase())];
  if (interval == 0) {
    // every reading: a record each time the IMU delivers a new one, or at its rate with it dead
    const uint32_t imuMs = imu_.data().linearAccelerationUpdatedMs;
    const bool fresh = imu_.data().linearAccelerationValid && imuMs != logImuMs_;
    if (!fresh && now - logWrittenMs_ < kImuReportIntervalMs) {
      return;
    }
    logImuMs_ = imuMs;
  } else if (now - logWrittenMs_ < interval) {
    return;
  }
  logWrittenMs_ = now;

  // the counter of the packet that last went out, not the one being built next: the counter only
  // moves on a transmission, and a record carrying the next one can't be lined up against a radio log
  bool allFresh = false;
  buildPacket(logPacket_, allFresh, sentCounter_);

  anchorUtc(now);

  if (flashLog_.ready()) {
    flashLog_.append(logPacket_, now);
    if (!flash_.paused()) {
      flashLog_.ensureSpace();
    }
  }
}

// Ties this boot's uptime to UTC, in the log and on the console. One line is enough to put a clock on
// every record of the boot, the earlier ones included
void FlightController::anchorUtc(uint32_t nowMs)
{
  if (!gps_.utcValid()) {
    return;
  }
  if (utcAnchored_ && nowMs - utcAnchorMs_ < kUtcAnchorIntervalMs) {
    return;
  }
  utcAnchorMs_ = nowMs;
  utcAnchored_ = true;

  if (flashLog_.ready()) {
    flashLog_.appendTime(gps_.utcUptimeMs(), gps_.utcEpoch(), gps_.utcNano(), gps_.utcBits(),
                         gps_.utcAccNs());
  }
  LOG_I(Gps, "utc") {
    line.field("epoch", gps_.utcEpoch());
    line.field("uptime", gps_.utcUptimeMs());
    line.field("nano", gps_.utcNano());
    line.fieldHex("bits", gps_.utcBits(), 2);
    line.field("accns", gps_.utcAccNs());
  }
}

bool FlightController::transmitDue() const
{
  return system_.nowMs() - previousReadMs_ >= txIntervalMs_;
}

void FlightController::runCycle()
{
  ++cycle_;
  ChipSatLog::setCycle(cycle_);

  const uint32_t now = system_.nowMs();
  CycleReads reads;
  reads.gateMs = now;

  bool allDataFresh = false;
  buildPacket(packet_, allDataFresh, packetCounter_);

  // how old each reading in this packet is
  reads.imuOkay = data_.imu.linearAccelerationValid;
  reads.gpsOkay = data_.gps.valid;
  reads.socOkay = data_.stateOfCharge.valid;
  reads.envOkay = data_.environmental.valid;
  reads.imuMs = now - data_.imu.linearAccelerationUpdatedMs;
  reads.gpsMs = now - data_.gps.updatedMs;
  reads.socMs = now - data_.stateOfCharge.updatedMs;
  reads.envMs = now - data_.environmental.updatedMs;

  logReads(reads);
  system_.waitMs(0);   // the console blocks while its buffer drains, read the GPS port

  const uint16_t counterUsed = packetCounter_;
  sentCounter_ = counterUsed;
  packetCounter_++;

  LOG_PACKET(reinterpret_cast<const uint8_t *>(&packet_), sizeof(packet_));
  system_.waitMs(0);
  LOG_I(Cyc, "packet") {
    line.field("ctr", counterUsed);
    line.fieldHex("valid", packet_.sensorValidity, 2);
    line.field("fresh", allDataFresh);
    line.field("gate", reads.gateMs);
    line.field("imuready", imu_.ready());
    line.field("imusleep", imu_.sleeping());
    line.field("phase", static_cast<unsigned int>(phase_.phase()));
    line.field("rec", flashLog_.records());
    line.field("drop", flashLog_.dropped());
  }

  // HP module only, the IMU sleeps for the transmission. The packet goes out either way
  // Whether the IMU sleeps through a transmission follows the module, and the module is now a
  // runtime setting - so this cannot stay a compile-time constant or a board configured as HP from
  // an LE build would keep its IMU awake through TX, which is the current draw the sleep exists to
  // avoid. Costs half the IMU samples on HP, which is visible in a pull as validity alternating
  const bool moduleSleepsImu = ChipSatSettings::active().module == ChipSatSettings::kModuleHp;
  const bool imuSleeps = moduleSleepsImu && imu_.ready();
  if (imuSleeps) {
    imu_.sleep();
  }
  // same rule for the flash: on the HP module nothing is logged during a transmission anyway
  const bool flashSleeps = moduleSleepsImu && flash_.ready() && !flashLog_.busy();
  if (flashSleeps) {
    flash_.pause();
  }
  transmit();
  if (flashSleeps) {
    flash_.resume();
  }
  if (imuSleeps) {
    imu_.wake();
  }

  // Soft reset the IMU if a report stays silent for a few cycles
  const uint8_t silent = silentImuReports(imu_.eventCounts(), imuCountsLastCycle_);
  for (uint8_t i = 0; i < 4; ++i) {
    imuCountsLastCycle_[i] = imu_.eventCounts()[i];
  }
  if (imu_.ready() && !imu_.sleeping() && silent != 0) {
    ++imuStuckCycles_;
  } else {
    imuStuckCycles_ = 0;
  }
  if (imuStuckCycles_ >= kImuStuckCycles) {
    LOG_W(Imu, "stuck") {
      line.fieldBits("silent", silent, 4);
      line.field("cycles", imuStuckCycles_);
      line.field("resets", imuSoftResets_);
    }
    if (imuSoftResets_ < kImuMaxSoftResets) {
      imu_.resetHub();
      ++imuSoftResets_;
    }
    imuStuckCycles_ = 0;
  }

  txIntervalMs_ = phaseTxIntervalMs(reads.socOkay);

  previousReadMs_ = system_.nowMs();

  LOG_I(Cyc, "end") {
    line.field("nextms", txIntervalMs_);
    line.field("phase", static_cast<unsigned int>(phase_.phase()));
    line.field("battery", ChipSatSettings::active().rate[static_cast<size_t>(phase_.phase())].fromSoc && reads.socOkay);
    line.field("durms", previousReadMs_ - reads.gateMs);
  }
}

void FlightController::transmit()
{
  bool freshSetup = false;
  if (!radioReady_) {
    freshSetup = true;
    radioReady_ = configRadio();
    if (!radioReady_) {
      LOG_E(Tx, "skipped") { line.field("reason", "radio"); }
      return;
    }
  }

  LOG_D(Tx, "send") { line.field("step", "start"); }

  const uint32_t startMs = system_.nowMs();
  led_.transmitStarted();
  Status status = radio_.startTransmit(reinterpret_cast<const uint8_t *>(&packet_), sizeof(packet_));
  if (status != Status::Ok && !freshSetup) {
    // a radio that reset itself fails here. Set it up again and try once more, so it costs no packet
    radioReady_ = configRadio();
    if (radioReady_) {
      status = radio_.startTransmit(reinterpret_cast<const uint8_t *>(&packet_), sizeof(packet_));
    }
  }
  if (status == Status::Ok) {
    // time on air is exact, 100 ms covers TCXO start and ramp. waitMs keeps the GPS port read meanwhile
    const uint32_t timeoutMs = radio_.timeOnAirMs(sizeof(packet_)) + 100;
    while (!radio_.transmitDone() && system_.nowMs() - startMs <= timeoutMs) {
      system_.waitMs(1);
      imu_.service();   // asleep on the HP module, where service() returns at once
      sampleSensors();
      logIfDue();
    }
    const bool done = radio_.transmitDone();
    status = radio_.finishTransmit();
    if (!done) {
      status = Status::Timeout;
    }
  }
  led_.transmitEnded();
  lastTxMs_ = system_.nowMs();
  const uint32_t txMs = lastTxMs_ - startMs;

  if (status == Status::Ok) {
    LOG_I(Tx, "done") {
      line.field("ok", true);
      line.field("code", radio_.lastCode());
      line.field("ms", txMs);
    }
  } else {
    radioReady_ = false;   // set the radio up again before the next packet
    LOG_E(Tx, "done") {
      line.field("ok", false);
      line.field("code", radio_.lastCode());
      line.field("ms", txMs);
    }
  }
}

void FlightController::logReads(const CycleReads &reads) const
{
  LOG_I(Imu, "read") {
    line.field("ok", reads.imuOkay);
    line.field("age", reads.imuMs);
    line.fieldBits("valid", imuValidBits(data_.imu), 4);
    const uint32_t age[4] = {
      line.ms() - data_.imu.linearAccelerationUpdatedMs,
      line.ms() - data_.imu.gyroscopeUpdatedMs,
      line.ms() - data_.imu.magnetometerUpdatedMs,
      line.ms() - data_.imu.orientationUpdatedMs,
    };
    line.fieldList("age", age, 4);
  }
  LOG_I(Gps, "read") {
    line.field("ok", reads.gpsOkay);
    line.field("fix", gps_.fixType());
    if (gps_.llhChecked()) {
      line.field("llhbad", gps_.invalidLlh());
    }
    line.field("valid", data_.gps.valid);
    line.field("age", reads.gpsMs);
  }
  LOG_I(Soc, "read") {
    line.field("ok", reads.socOkay);
    line.field("valid", data_.stateOfCharge.valid);
    line.field("age", reads.socMs);
  }
  LOG_I(Env, "read") {
    line.field("ok", reads.envOkay);
    line.field("valid", data_.environmental.valid);
    line.field("age", reads.envMs);
  }

  LOG_D(Imu, "values") {
    line.field("ax", data_.imu.linearAccelerationMps2.x, 3);
    line.field("ay", data_.imu.linearAccelerationMps2.y, 3);
    line.field("az", data_.imu.linearAccelerationMps2.z, 3);
    line.field("gx", data_.imu.gyroscopeRadPerSec.x, 4);
    line.field("gy", data_.imu.gyroscopeRadPerSec.y, 4);
    line.field("gz", data_.imu.gyroscopeRadPerSec.z, 4);
    line.field("mx", data_.imu.magnetometerMicroTesla.x, 2);
    line.field("my", data_.imu.magnetometerMicroTesla.y, 2);
    line.field("mz", data_.imu.magnetometerMicroTesla.z, 2);
  }
  LOG_D(Imu, "quat") {
    line.field("qi", data_.imu.orientation.i, 5);
    line.field("qj", data_.imu.orientation.j, 5);
    line.field("qk", data_.imu.orientation.k, 5);
    line.field("qr", data_.imu.orientation.real, 5);
    const uint32_t updated[4] = {
      data_.imu.linearAccelerationUpdatedMs,
      data_.imu.gyroscopeUpdatedMs,
      data_.imu.magnetometerUpdatedMs,
      data_.imu.orientationUpdatedMs,
    };
    line.fieldList("upd", updated, 4);
    line.fieldList("events", imu_.eventCounts(), 4);
  }
  LOG_D(Gps, "values") {
    line.fieldFixed("lat", data_.gps.latitudeE7, 7);
    line.fieldFixed("lon", data_.gps.longitudeE7, 7);
    line.field("altmm", data_.gps.altitudeMSLmm);
    line.field("upd", data_.gps.updatedMs);
  }
  LOG_D(Soc, "values") {
    line.field("pct", data_.stateOfCharge.cellPercentage, 2);
    line.field("v", data_.stateOfCharge.cellVoltage, 3);
    line.field("raw", gauge_.rawPercent(), 2);
    line.field("upd", data_.stateOfCharge.updatedMs);
  }
  LOG_D(Env, "values") {
    line.field("temp", data_.environmental.temperatureC, 2);
    line.field("phpa", data_.environmental.pressurePa / 100.0f, 2);
    line.field("rh", data_.environmental.humidityPercent, 2);
    line.field("altm", data_.environmental.altitudeM, 2);
    line.field("upd", data_.environmental.updatedMs);
  }
}

} // namespace ChipSatApp
