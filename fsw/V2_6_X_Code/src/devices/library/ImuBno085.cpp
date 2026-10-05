#include <Arduino.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_IMU_DRIVER == CHIPSAT_DRIVER_LIBRARY

#include <SparkFun_BNO08x_Arduino_Library.h>
#include "../Imu.h"
#include "../../log/Log.h"

namespace ChipSatDevices
{

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

// Only one of these, the library keeps its state in globals
static BNO08x bno;

// SparkFun's own report handler keeps only the last report of each I2C read, so we use ours
// (sensorHandler() decodes every event into the one _sensor_value)
struct ReportSlot
{
  bool fresh = false;
  float v[4] = {0.0f, 0.0f, 0.0f, 0.0f};
  uint32_t count = 0;
};

static ReportSlot slots[4];
static bool reportThisPass = false;

static void onSensorEvent(void *, sh2_SensorEvent_t *event)
{
  sh2_SensorValue_t value;
  if (sh2_decodeSensorEvent(&value, event) != SH2_OK) {
    return;
  }

  ReportSlot *slot = nullptr;
  switch (value.sensorId) {
    case SH2_LINEAR_ACCELERATION:
      slot = &slots[0];
      slot->v[0] = value.un.linearAcceleration.x;
      slot->v[1] = value.un.linearAcceleration.y;
      slot->v[2] = value.un.linearAcceleration.z;
      break;
    case SH2_GYROSCOPE_CALIBRATED:
      slot = &slots[1];
      slot->v[0] = value.un.gyroscope.x;
      slot->v[1] = value.un.gyroscope.y;
      slot->v[2] = value.un.gyroscope.z;
      break;
    case SH2_MAGNETIC_FIELD_CALIBRATED:
      slot = &slots[2];
      slot->v[0] = value.un.magneticField.x;
      slot->v[1] = value.un.magneticField.y;
      slot->v[2] = value.un.magneticField.z;
      break;
    case SH2_ROTATION_VECTOR:
      slot = &slots[3];
      slot->v[0] = value.un.rotationVector.i;
      slot->v[1] = value.un.rotationVector.j;
      slot->v[2] = value.un.rotationVector.k;
      slot->v[3] = value.un.rotationVector.real;
      break;
    default:
      return;
  }

  slot->fresh = true;
  ++slot->count;
  reportThisPass = true;
}

[[maybe_unused]] static void writeReportNames(ChipSatLog::Line &line, uint8_t reports)
{
  static const char *const kNames[4] = {"linaccel", "gyro", "mag", "quat"};
  bool first = true;
  for (uint8_t i = 0; i < 4; ++i) {
    if (reports & (1U << i)) {
      if (!first) {
        line.text(",");
      }
      line.text(kNames[i]);
      first = false;
    }
  }
}

[[maybe_unused]] static void writeReportResults(ChipSatLog::Line &line, bool accel, bool gyro, bool mag,
                                                bool quat, const uint8_t *tries)
{
  // Keep this line under 64 bytes so it doesn't block (SERIAL_TX_BUFFER_SIZE is 64 in STM32 core 2.12.0)
  const uint8_t okBits = (accel ? 1U : 0U) | (gyro ? 2U : 0U) | (mag ? 4U : 0U) | (quat ? 8U : 0U);
  line.fieldBits("ok", okBits, 4);
  const uint32_t t[4] = {tries[0], tries[1], tries[2], tries[3]};
  line.fieldList("tries", t, 4);
}

Imu::Imu(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system)
  : bus_(bus), system_(system)
{
}

Status Imu::begin(uint16_t reportIntervalMs)
{
  reportIntervalMs_ = reportIntervalMs;
  sleeping_ = false;
  LOG_I(Imu, "start");
  const uint32_t startMs = system_.nowMs();

  for (uint8_t attempt = 0; attempt < kInitAttempts; ++attempt) {
    // No INT/RST pins, so the library leaves PB3/PB4 alone
    if (bno.begin(kImuAddress, bus_.arduinoWire(), -1, -1)) {
      sh2_setSensorCallback(onSensorEvent, nullptr);   // begin() just registered SparkFun's handler
      if (configureReports(kImuBootSettleMs)) {
        ready_ = true;
        LOG_I(Imu, "init") {
          line.field("ok", true);
          line.field("attempts", attempt + 1);
          line.field("ms", system_.nowMs() - startMs);
        }
        return Status::Ok;
      }

      // Found it but the reports wouldn't turn on, give up
      ready_ = false;
      LOG_E(Imu, "init") {
        line.field("ok", false);
        line.field("reason", "reports");
        line.field("attempts", attempt + 1);
        line.field("ms", system_.nowMs() - startMs);
      }
      return Status::Failed;
    }

    system_.waitMs(kInitRetryDelayMs);
  }

  ready_ = false;
  LOG_E(Imu, "init") {
    line.field("ok", false);
    line.field("reason", "begin");
    line.field("attempts", kInitAttempts);
    line.field("ms", system_.nowMs() - startMs);
  }
  return Status::NoAck;
}

// The library retries a failed I2C write forever (shtp.c:309-314), so every call that can write
// checks the IMU still answers first. Narrows the window, doesn't close it
static bool answers(ChipSatPlatform::I2cBus &bus)
{
  return bus.probe(kImuAddress) == Status::Ok;
}

void Imu::service()
{
  if (!sleeping_) {
    update();
  }
}

bool Imu::configureReports(uint16_t startupDelayMs)
{
  // modeOn might ack before the IMU is ready for report commands, the settle is there to be safe
  if (startupDelayMs > 0) {
    system_.waitMs(startupDelayMs);
  }

  bool accelerationOkay = false;
  bool gyroOkay = false;
  bool magnetometerOkay = false;
  bool quaternionOkay = false;

  for (uint8_t attempt = 0; attempt < kImuEnableAttempts && !accelerationOkay; ++attempt) {
    accelerationOkay = bno.enableLinearAccelerometer(reportIntervalMs_);
    tries_[0] = attempt + 1;
    system_.waitMs(accelerationOkay ? kImuEnableSpacingMs : kImuEnableRetryDelayMs);
  }

  for (uint8_t attempt = 0; attempt < kImuEnableAttempts && !gyroOkay; ++attempt) {
    gyroOkay = bno.enableGyro(reportIntervalMs_);
    tries_[1] = attempt + 1;
    system_.waitMs(gyroOkay ? kImuEnableSpacingMs : kImuEnableRetryDelayMs);
  }

  for (uint8_t attempt = 0; attempt < kImuEnableAttempts && !magnetometerOkay; ++attempt) {
    magnetometerOkay = bno.enableMagnetometer(reportIntervalMs_);
    tries_[2] = attempt + 1;
    system_.waitMs(magnetometerOkay ? kImuEnableSpacingMs : kImuEnableRetryDelayMs);
  }

  for (uint8_t attempt = 0; attempt < kImuEnableAttempts && !quaternionOkay; ++attempt) {
    quaternionOkay = bno.enableRotationVector(reportIntervalMs_);
    tries_[3] = attempt + 1;
    system_.waitMs(quaternionOkay ? kImuEnableSpacingMs : kImuEnableRetryDelayMs);
  }

  const bool allOkay = accelerationOkay && gyroOkay && magnetometerOkay && quaternionOkay;
  if (allOkay) {
    LOG_I(Imu, "reports") {
      writeReportResults(line, accelerationOkay, gyroOkay, magnetometerOkay, quaternionOkay, tries_);
    }
  } else {
    LOG_W(Imu, "reports") {
      writeReportResults(line, accelerationOkay, gyroOkay, magnetometerOkay, quaternionOkay, tries_);
    }
  }
  return allOkay;
}

Status Imu::waitForFresh(uint16_t timeoutMs)
{
  if (!ready_ || sleeping_) {
    missing_ = kImuAllReports;
    LOG_W(Imu, "refused") {
      line.field("call", "fresh");
      line.field("ready", ready_);
      line.field("sleeping", sleeping_);
    }
    return Status::NotReady;
  }

  // Drain what's already queued, those don't count
  update();

  // The valid flags could be from before the last TX, so each timestamp has to move
  const uint32_t previousAccelerationMs = data_.linearAccelerationUpdatedMs;
  const uint32_t previousGyroscopeMs = data_.gyroscopeUpdatedMs;
  const uint32_t previousMagnetometerMs = data_.magnetometerUpdatedMs;
  const uint32_t previousOrientationMs = data_.orientationUpdatedMs;

  uint8_t freshReports = 0;
  const uint32_t startMs = system_.nowMs();

  do {
    update();

    if (data_.linearAccelerationValid && data_.linearAccelerationUpdatedMs != previousAccelerationMs) {
      freshReports |= kImuLinearAcceleration;
    }
    if (data_.gyroscopeValid && data_.gyroscopeUpdatedMs != previousGyroscopeMs) {
      freshReports |= kImuGyroscope;
    }
    if (data_.magnetometerValid && data_.magnetometerUpdatedMs != previousMagnetometerMs) {
      freshReports |= kImuMagnetometer;
    }
    if (data_.orientationValid && data_.orientationUpdatedMs != previousOrientationMs) {
      freshReports |= kImuOrientation;
    }

    if (freshReports == kImuAllReports) {
      missing_ = 0;
      return Status::Ok;
    }

    system_.waitMs(kImuFreshPollMs);
  } while (static_cast<uint32_t>(system_.nowMs() - startMs) < timeoutMs);

  missing_ = kImuAllReports & ~freshReports;
  LOG_W(Imu, "timeout") {
    line.field("limitms", timeoutMs);   // the cycle read or the wait after a wake
    line.key("missing");
    writeReportNames(line, missing_);
  }
  return Status::Timeout;
}

bool Imu::update()
{
  if (!ready_ || sleeping_ || !answers(bus_)) {
    return false;
  }

  if (bno.wasReset()) {
    // The library flags a reset during begin(), so this also runs once right after boot
    const bool latched = !firstResetSeen_;
    firstResetSeen_ = true;
    if (latched) {
      LOG_I(Imu, "reset") { line.field("latched", true); }
    } else {
      LOG_W(Imu, "reset") { line.field("latched", false); }
    }

    invalidate();
    ready_ = configureReports(kImuWakeSettleMs);   // once false, nothing sets it back

    if (!ready_) {
      LOG_E(Imu, "reset") {
        line.field("reenable", false);
        line.field("ready", false);
        line.field("retry", "never");
      }
      return false;
    }
  }

  // Each serviceBus() reads one I2C transfer, keep going while reports come in
  for (uint8_t pass = 0; pass < kImuMaxServicePasses; ++pass) {
    reportThisPass = false;
    bno.serviceBus();
    if (!reportThisPass) {
      break;
    }
  }

  const uint32_t now = system_.nowMs();
  bool updated = false;

  if (slots[0].fresh) {
    data_.linearAccelerationMps2.x = slots[0].v[0];
    data_.linearAccelerationMps2.y = slots[0].v[1];
    data_.linearAccelerationMps2.z = slots[0].v[2];
    data_.linearAccelerationValid = true;
    data_.linearAccelerationUpdatedMs = now;
    slots[0].fresh = false;
    updated = true;
  }
  if (slots[1].fresh) {
    data_.gyroscopeRadPerSec.x = slots[1].v[0];
    data_.gyroscopeRadPerSec.y = slots[1].v[1];
    data_.gyroscopeRadPerSec.z = slots[1].v[2];
    data_.gyroscopeValid = true;
    data_.gyroscopeUpdatedMs = now;
    slots[1].fresh = false;
    updated = true;
  }
  if (slots[2].fresh) {
    data_.magnetometerMicroTesla.x = slots[2].v[0];
    data_.magnetometerMicroTesla.y = slots[2].v[1];
    data_.magnetometerMicroTesla.z = slots[2].v[2];
    data_.magnetometerValid = true;
    data_.magnetometerUpdatedMs = now;
    slots[2].fresh = false;
    updated = true;
  }
  if (slots[3].fresh) {
    data_.orientation.i = slots[3].v[0];
    data_.orientation.j = slots[3].v[1];
    data_.orientation.k = slots[3].v[2];
    data_.orientation.real = slots[3].v[3];
    data_.orientationValid = true;
    data_.orientationUpdatedMs = now;
    slots[3].fresh = false;
    updated = true;
  }

  for (uint8_t i = 0; i < 4; ++i) {
    events_[i] = slots[i].count;
  }

  updateOverallValidity();
  return updated;
}

Status Imu::sleep()
{
  if (!ready_) {
    LOG_W(Imu, "refused") {
      line.field("call", "sleep");
      line.field("ready", false);
    }
    return Status::NotReady;
  }

  if (sleeping_) {
    return Status::Ok;   // lets TX go ahead after a failed wake
  }

  LOG_D(Imu, "sleep") { line.field("step", "start"); }
  if (!answers(bus_)) {
    LOG_W(Imu, "sleep") {
      line.field("ok", false);
      line.field("step", "probe");
    }
    return Status::Failed;
  }
  if (!bno.modeSleep()) {
    LOG_W(Imu, "sleep") { line.field("ok", false); }
    return Status::Failed;
  }

  sleeping_ = true;
  LOG_I(Imu, "sleep") { line.field("ok", true); }
  return Status::Ok;
}

Status Imu::wake()
{
  if (!ready_) {
    LOG_W(Imu, "refused") {
      line.field("call", "wake");
      line.field("ready", false);
    }
    return Status::NotReady;
  }

  if (!sleeping_) {
    LOG_D(Imu, "wake") {
      line.field("step", "skip");
      line.field("sleeping", false);
    }
    return Status::Ok;
  }

  const uint32_t startMs = system_.nowMs();
  LOG_D(Imu, "wake") { line.field("step", "start"); }
  if (!answers(bus_)) {
    LOG_W(Imu, "wake") {
      line.field("ok", false);
      line.field("step", "probe");
    }
    return Status::Failed;   // still flagged asleep, tried again next cycle
  }
  if (!bno.modeOn()) {
    LOG_W(Imu, "wake") {
      line.field("ok", false);
      line.field("step", "modeon");
    }
    return Status::Failed;   // still flagged asleep
  }

  // Clear first, waitForFresh below needs update() to run
  sleeping_ = false;

  // Unlike the reset path, a failure here doesn't mark the IMU lost
  if (!configureReports(kImuWakeSettleMs)) {
    LOG_W(Imu, "wake") {
      line.field("ok", false);
      line.field("step", "reports");
    }
    return Status::Failed;
  }

  if (waitForFresh(kImuFreshWaitWakeMs) != Status::Ok) {
    LOG_W(Imu, "wake") {
      line.field("ok", false);
      line.field("step", "fresh");
      line.field("ms", system_.nowMs() - startMs);
    }
    return Status::Timeout;
  }

  LOG_I(Imu, "wake") {
    line.field("ok", true);
    line.field("fresh", true);
    line.field("ms", system_.nowMs() - startMs);
  }
  return Status::Ok;
}

Status Imu::resetHub()
{
  if (!ready_) {
    return Status::NotReady;   // a lost IMU stays lost
  }

  // softReset() only sends the command (sh2_devReset -> shtp_send), it doesn't wait for a reply.
  // update() turns the reports back on once the IMU says it has reset
  if (!answers(bus_)) {
    LOG_W(Imu, "softreset") { line.field("ok", false); }
    return Status::Failed;
  }
  const bool ok = bno.softReset();
  system_.waitMs(kImuSoftResetSettleMs);

  LOG_W(Imu, "softreset") { line.field("ok", ok); }
  return ok ? Status::Ok : Status::Failed;
}

void Imu::invalidate()
{
  data_.linearAccelerationValid = false;
  data_.gyroscopeValid = false;
  data_.magnetometerValid = false;
  data_.orientationValid = false;
  data_.valid = false;
}

void Imu::updateOverallValidity()
{
  data_.valid = data_.linearAccelerationValid && data_.gyroscopeValid && data_.magnetometerValid &&
                data_.orientationValid;
}

} // namespace ChipSatDevices

#endif
