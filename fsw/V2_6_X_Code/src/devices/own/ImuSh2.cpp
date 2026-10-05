#include <Arduino.h>
#include <string.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_IMU_DRIVER == CHIPSAT_DRIVER_OWN

#include "../Imu.h"
#include "../../log/Log.h"

namespace ChipSatDevices
{

using ChipSatPlatform::I2cBus;
using ChipSatPlatform::Status;
using ChipSatPlatform::System;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

// In the order of the ImuReport bits in Imu.h
static const uint8_t kReportIds[4] = {kSh2LinearAcceleration, kSh2Gyroscope, kSh2Magnetometer,
                                      kSh2RotationVector};
static const uint8_t kReportQ[4] = {kSh2LinearAccelerationQ, kSh2GyroscopeQ, kSh2MagnetometerQ,
                                    kSh2RotationVectorQ};

struct ReportSlot
{
  bool fresh = false;
  float v[4] = {0.0f, 0.0f, 0.0f, 0.0f};
  uint32_t count = 0;
};

static ReportSlot slots[4];
static bool reportThisPass = false;

// What the IMU told us, set while packets are handled
static bool resetSeen = false;
static bool sh2InitSeen = false;
static bool headerWarned = false;   // one warning per bad header, not one per loop pass
static bool unknownWarned = false;   // one per boot is enough
static bool lengthsChecked = false;
static uint8_t featureReplies = 0;   // one bit per report, set by its 0xFC reply
static uint8_t wantedIndex = 0;
static uint32_t wantedCountBefore = 0;
static uint32_t grantedUs[4] = {0, 0, 0, 0};
static bool productIdSeen = false;
static uint8_t productId[kSh2ProductIdResponseLength];

// Our side's sequence number per channel. The IMU doesn't seem to check them, sent anyway like shtp.c
static uint8_t outSeq[kShtpChannelReports + 1] = {0, 0, 0, 0};

// The advertisement after a reset is the biggest thing the IMU sends. It fitted SparkFun's 384 bytes
static uint8_t packet[kShtpPacketMax];
static uint16_t packetLength = 0;

static uint32_t u32(const uint8_t *p)
{
  return static_cast<uint32_t>(p[0]) | static_cast<uint32_t>(p[1]) << 8 | static_cast<uint32_t>(p[2]) << 16 |
         static_cast<uint32_t>(p[3]) << 24;
}

// value / 2^Q, the same float as the library's read16() * SCALE_Q()
static float fixedPoint(const uint8_t *p, uint8_t q)
{
  return static_cast<int16_t>(p[0] | p[1] << 8) / static_cast<float>(1UL << q);
}

// The 4-byte header first for the length, then the whole packet again from the start in 32-byte
// chunks. Each chunk after the first repeats the header, which is skipped. Same as SparkFun's
// i2chal_read. Ok with packetLength 0 means the IMU had nothing
static Status readPacket(I2cBus &bus)
{
  packetLength = 0;
  uint8_t header[kShtpHeaderLength];
  Status status = bus.read(kImuAddress, header, sizeof header);
  if (status != Status::Ok) {
    return status;
  }

  const uint16_t raw = static_cast<uint16_t>(header[0] | header[1] << 8);
  const uint16_t length = raw & ~kShtpContinuation;
  if (length == 0) {
    return Status::Ok;
  }
  if (length > kShtpPacketMax) {
    // A garbled header (0xFFFF from a dead bus lands here too), or more than the IMU should ever send.
    // Not read out, reading 32 kB would take seconds. If the IMU keeps offering it, the stuck check soft resets it
    if (!headerWarned) {
      LOG_W(Imu, "packet") {
        line.field("dropped", static_cast<unsigned int>(length));
        line.field("reason", "header");
      }
      headerWarned = true;
    }
    return Status::BadData;
  }
  headerWarned = false;

  // A continuation here is the rest of a packet we gave up on. It still gets read out, so the IMU
  // doesn't keep offering it, but it isn't used
  const bool continuation = (raw & kShtpContinuation) != 0;
  const bool keep = !continuation && length >= kShtpHeaderLength;

  uint8_t chunk[kShtpChunkLength];
  uint16_t remaining = length;
  uint16_t stored = 0;
  bool first = true;
  while (remaining > 0) {
    const uint16_t wanted = first ? remaining : static_cast<uint16_t>(remaining + kShtpHeaderLength);
    const uint8_t size = wanted < kShtpChunkLength ? static_cast<uint8_t>(wanted) : kShtpChunkLength;
    status = bus.read(kImuAddress, chunk, size);
    if (status != Status::Ok) {
      return status;
    }
    const uint8_t skip = first ? 0 : kShtpHeaderLength;
    const uint8_t cargo = static_cast<uint8_t>(size - skip);
    if (keep) {
      memcpy(&packet[stored], &chunk[skip], cargo);
    }
    stored = static_cast<uint16_t>(stored + cargo);
    remaining = static_cast<uint16_t>(remaining - cargo);
    first = false;
  }

  if (!keep) {
    LOG_W(Imu, "packet") {
      line.field("dropped", static_cast<unsigned int>(length));
      line.field("reason", continuation ? "partial" : "short");
    }
    return Status::BadData;
  }
  packetLength = length;
  return Status::Ok;
}

// One attempt, the caller decides what to do about a failure
static Status sendPacket(I2cBus &bus, uint8_t channel, const uint8_t *payload, uint8_t length)
{
  uint8_t out[kShtpHeaderLength + kSh2SetFeatureLength];
  if (channel > kShtpChannelReports || length > sizeof out - kShtpHeaderLength) {
    return Status::BadData;
  }
  const uint8_t total = static_cast<uint8_t>(kShtpHeaderLength + length);
  out[0] = total;
  out[1] = 0;
  out[2] = channel;
  out[3] = outSeq[channel];
  memcpy(&out[kShtpHeaderLength], payload, length);

  const Status status = bus.write(kImuAddress, out, total);
  if (status == Status::Ok) {
    ++outSeq[channel];
  }
  return status;
}

static uint8_t reportLength(uint8_t id)
{
  switch (id) {
    case kSh2Timebase:
    case kSh2TimestampRebase:
      return kSh2TimeReportLength;
    case kSh2LinearAcceleration:
    case kSh2Gyroscope:
    case kSh2Magnetometer:
      return kSh2ReportLength;
    case kSh2RotationVector:
      return kSh2RotationVectorLength;
    default:
      return 0;
  }
}

static void takeReport(const uint8_t *report)
{
  for (uint8_t i = 0; i < 4; ++i) {
    if (report[0] != kReportIds[i]) {
      continue;
    }
    ReportSlot &slot = slots[i];
    slot.v[0] = fixedPoint(&report[4], kReportQ[i]);
    slot.v[1] = fixedPoint(&report[6], kReportQ[i]);
    slot.v[2] = fixedPoint(&report[8], kReportQ[i]);
    if (report[0] == kSh2RotationVector) {
      slot.v[3] = fixedPoint(&report[10], kReportQ[i]);
    }
    slot.fresh = true;
    ++slot.count;
    reportThisPass = true;
    return;
  }
}

// Reports one after another. We can't know where the one after an unknown id starts, so the rest
// of the packet goes
static void walkReports(const uint8_t *cargo, uint16_t length)
{
  uint16_t at = 0;
  while (at < length) {
    const uint8_t size = reportLength(cargo[at]);
    if (size == 0 || at + size > length) {
      if (!unknownWarned) {
        LOG_W(Imu, "report") {
          line.fieldHex("unknown", cargo[at], 2);
          line.field("left", static_cast<unsigned int>(length - at));
        }
        unknownWarned = true;
      }
      return;
    }
    takeReport(&cargo[at]);
    at = static_cast<uint16_t>(at + size);
  }
}

// Only the enable replies and the product id matter here, the rest is skipped
static void walkControl(const uint8_t *cargo, uint16_t length)
{
  uint16_t at = 0;
  while (at < length) {
    const uint8_t id = cargo[at];
    uint8_t size = 0;
    if (id == kSh2GetFeatureResponse) {
      size = kSh2GetFeatureResponseLength;
    } else if (id == kSh2ProductIdResponse) {
      size = kSh2ProductIdResponseLength;
    } else if (id == kSh2CommandResponse) {
      size = kSh2CommandResponseLength;
    }
    if (size == 0 || at + size > length) {
      return;
    }

    const uint8_t *reply = &cargo[at];
    if (id == kSh2GetFeatureResponse) {
      for (uint8_t i = 0; i < 4; ++i) {
        if (reply[1] == kReportIds[i]) {
          featureReplies |= static_cast<uint8_t>(1U << i);
          grantedUs[i] = u32(&reply[5]);
        }
      }
    } else if (id == kSh2CommandResponse && reply[2] == kSh2UnsolicitedInit && reply[6] == kSh2InitSystem) {
      sh2InitSeen = true;   // the sensor hub part is up, after reset complete
    } else if (id == kSh2ProductIdResponse && !productIdSeen) {
      memcpy(productId, reply, sizeof productId);   // the first of several, one per firmware part
      productIdSeen = true;
    }
    at = static_cast<uint16_t>(at + size);
  }
}

// The advertisement's own report length table, checked once against reportLength(). Only logged,
// a wrong one would show as unknown reports
static void checkLengths(const uint8_t *cargo, uint16_t length)
{
  uint16_t at = 1;   // after the advertisement's response id
  while (at + 2 <= length) {
    const uint8_t tag = cargo[at];
    const uint8_t size = cargo[at + 1];
    if (at + 2 + size > length) {
      return;
    }
    if (tag == kShtpTagReportLengths) {
      uint8_t checked = 0;
      uint8_t wrong = 0;
      for (uint8_t i = 0; i + 1 < size; i = static_cast<uint8_t>(i + 2)) {
        const uint8_t id = cargo[at + 2 + i];
        const uint8_t imuLength = cargo[at + 3 + i];
        const uint8_t ours = reportLength(id);
        if (ours == 0) {
          continue;
        }
        ++checked;
        if (ours != imuLength) {
          ++wrong;
          LOG_W(Imu, "length") {
            line.fieldHex("id", id, 2);
            line.field("imu", static_cast<unsigned int>(imuLength));
            line.field("ours", static_cast<unsigned int>(ours));
          }
        }
      }
      LOG_I(Imu, "lengths") {
        line.field("checked", static_cast<unsigned int>(checked));
        line.field("wrong", static_cast<unsigned int>(wrong));
      }
      lengthsChecked = true;
      return;
    }
    at = static_cast<uint16_t>(at + 2 + size);
  }
}

static void handlePacket()
{
  const uint8_t channel = packet[2];
  const uint8_t *cargo = &packet[kShtpHeaderLength];
  const uint16_t length = static_cast<uint16_t>(packetLength - kShtpHeaderLength);

  switch (channel) {
    case kShtpChannelCommand:
      // The advertisement, sent after every reset. Our sequence numbers start over, as in shtp.c
      memset(outSeq, 0, sizeof outSeq);
      if (!lengthsChecked) {
        checkLengths(cargo, length);
      }
      break;
    case kShtpChannelExecutable:
      if (length == 1 && cargo[0] == kImuResetComplete) {
        resetSeen = true;
        sh2InitSeen = false;   // the sensor hub's own init comes after this
      }
      break;
    case kShtpChannelControl:
      walkControl(cargo, length);
      break;
    case kShtpChannelReports:
      walkReports(cargo, length);
      break;
    default:
      break;   // wake and gyro rotation vector channels, never turned on
  }
}

// Reads and handles packets until done() or the time is up. A read can be held by the IMU until it
// has something to send (clock stretching, BNO085 datasheet 1.2.2), Wire gives up after 100 ms
static bool pollUntil(I2cBus &bus, System &system, uint16_t timeoutMs, bool (*done)())
{
  const uint32_t startMs = system.nowMs();
  while (true) {
    const Status status = readPacket(bus);
    const bool gotOne = status == Status::Ok && packetLength > 0;
    if (gotOne) {
      handlePacket();
    }
    if (done()) {
      return true;
    }
    if (static_cast<uint32_t>(system.nowMs() - startMs) >= timeoutMs) {
      return false;
    }
    // waitMs(0) runs onWait once. The GPS UART's 64 bytes fill in about 66 ms
    system.waitMs(gotOne ? 0 : kImuPollMs);
  }
}

enum class Enable : uint8_t { NotSent, Sent, Confirmed };

// Set Feature (BNO085 datasheet Figure 1-33)
static Status sendSetFeature(I2cBus &bus, uint8_t index, uint32_t intervalUs)
{
  uint8_t command[kSh2SetFeatureLength] = {0};
  command[0] = kSh2SetFeature;
  command[1] = kReportIds[index];
  command[5] = static_cast<uint8_t>(intervalUs);
  command[6] = static_cast<uint8_t>(intervalUs >> 8);
  command[7] = static_cast<uint8_t>(intervalUs >> 16);
  command[8] = static_cast<uint8_t>(intervalUs >> 24);
  return sendPacket(bus, kShtpChannelControl, command, sizeof command);
}

// Set Feature, then wait for its 0xFC reply or the report itself. The library never waited for
// either, so no answer isn't a failure on its own
static Enable enableReport(I2cBus &bus, System &system, uint8_t index, uint32_t intervalUs)
{
  wantedIndex = index;
  wantedCountBefore = slots[index].count;
  featureReplies &= static_cast<uint8_t>(~(1U << index));
  if (sendSetFeature(bus, index, intervalUs) != Status::Ok) {
    return Enable::NotSent;
  }
  const bool answered = pollUntil(bus, system, kImuReplyWaitMs, [] {
    return (featureReplies & (1U << wantedIndex)) != 0 || slots[wantedIndex].count != wantedCountBefore;
  });
  if (!answered) {
    return Enable::Sent;
  }

  if ((featureReplies & (1U << index)) != 0 && grantedUs[index] != intervalUs) {
    LOG_W(Imu, "interval") {
      line.field("report", static_cast<unsigned int>(index));
      line.field("us", grantedUs[index]);
    }
  }
  return Enable::Confirmed;
}

// Logged for reference, begin() doesn't depend on it. The library waited forever for this one
static void readProductId(I2cBus &bus, System &system)
{
  productIdSeen = false;
  const uint8_t request[2] = {kSh2ProductIdRequest, 0};
  const bool okay = sendPacket(bus, kShtpChannelControl, request, sizeof request) == Status::Ok &&
                    pollUntil(bus, system, kImuReplyWaitMs, [] { return productIdSeen; });
  if (!okay) {
    LOG_W(Imu, "productid") { line.field("ok", false); }
    return;
  }
  LOG_I(Imu, "productid") {
    line.field("part", u32(&productId[4]));
    line.field("ver", static_cast<unsigned int>(productId[2]));
    line.field("minor", static_cast<unsigned int>(productId[3]));
    line.field("patch", static_cast<unsigned int>(productId[12] | productId[13] << 8));
    line.field("build", u32(&productId[8]));
    line.field("cause", static_cast<unsigned int>(productId[1]));   // 1 power on, 2 internal, 3 watchdog, 4 external
  }
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

// Each report's valid flag and time in IMUData, in ImuReport bit order
static bool *validFlag(ChipSatSensors::IMUData &d, uint8_t i)
{
  bool *const flags[4] = {&d.linearAccelerationValid, &d.gyroscopeValid, &d.magnetometerValid, &d.orientationValid};
  return flags[i];
}

static uint32_t *updatedTime(ChipSatSensors::IMUData &d, uint8_t i)
{
  uint32_t *const times[4] = {&d.linearAccelerationUpdatedMs, &d.gyroscopeUpdatedMs, &d.magnetometerUpdatedMs,
                              &d.orientationUpdatedMs};
  return times[i];
}

// An IMU that never started, or was lost, gets started again in the background by restartStep().
// One short step per call and never a wait: probe and reset, up to kImuRestartReads packets (the
// advertisement is the long one, about 30 ms at 100 kHz), or one Set Feature
enum class Restart : uint8_t { Idle, ResetSent, Enabling };
static Restart restart = Restart::Idle;
static uint32_t restartMs = 0;   // when this step started, or the last try
static uint8_t restartIndex = 0;

Imu::Imu(ChipSatPlatform::I2cBus &bus, ChipSatPlatform::System &system)
  : bus_(bus), system_(system)
{
}

Status Imu::begin(uint16_t reportIntervalMs)
{
  reportIntervalMs_ = reportIntervalMs;
  sleeping_ = false;
  ready_ = false;
  LOG_I(Imu, "start");
  const uint32_t startMs = system_.nowMs();
  const char *reason = "noack";
  Status failure = Status::NoAck;

  // Every attempt starts from nothing, so a retry really is one
  for (uint8_t attempt = 0; attempt < kImuInitAttempts; ++attempt) {
    if (attempt > 0) {
      system_.waitMs(kInitRetryDelayMs);
    }
    invalidate();
    memset(outSeq, 0, sizeof outSeq);
    resetSeen = false;
    sh2InitSeen = false;

    if (bus_.probe(kImuAddress) != Status::Ok) {
      reason = "noack";
      failure = Status::NoAck;
      continue;
    }

    // 05 00 01 00 01, the same reset SparkFun sends at boot. The IMU answers with its advertisement,
    // then reset complete on the executable channel
    const uint8_t reset = kImuCommandReset;
    if (sendPacket(bus_, kShtpChannelExecutable, &reset, 1) != Status::Ok) {
      reason = "write";
      failure = Status::Failed;
      continue;
    }
    if (!pollUntil(bus_, system_, kImuResetWaitMs, [] { return resetSeen; })) {
      reason = "reset";
      failure = Status::Timeout;
      continue;
    }
    resetSeen = false;
    firstResetSeen_ = true;

    // Reset complete is the executable. The sensor hub sends its own init after it (BNO085 datasheet 5.2.1),
    // and nothing should be asked of it before that. The library had at least 300 ms here. Goes on
    // without it at the old 1000 ms settle, since it's not known that every firmware sends it
    [[maybe_unused]] const bool sh2Up =
      sh2InitSeen || pollUntil(bus_, system_, kImuBootSettleMs, [] { return sh2InitSeen; });
    invalidate();   // anything read before the reset is from the last run

    readProductId(bus_, system_);

    if (configureReports(0)) {
      ready_ = true;
      restart = Restart::Idle;
      emptyWaits_ = 0;
      LOG_I(Imu, "init") {
        line.field("ok", true);
        line.field("sh2", sh2Up);
        line.field("attempts", attempt + 1);
        line.field("ms", system_.nowMs() - startMs);
      }
      return Status::Ok;
    }
    reason = "reports";
    failure = Status::Failed;
  }

  LOG_E(Imu, "init") {
    line.field("ok", false);
    line.field("reason", reason);
    line.field("attempts", kImuInitAttempts);
    line.field("ms", system_.nowMs() - startMs);
  }
  lose(false);   // restartStep() tries again from the next service()
  return failure;
}

void Imu::service()
{
  if (!ready_) {
    restartStep();
  } else if (!sleeping_) {
    update();
  }
}

void Imu::restartStep()
{
  const uint32_t now = system_.nowMs();
  const uint32_t elapsed = now - restartMs;

  switch (restart) {
    case Restart::Idle: {
      if (elapsed < sensorRetryMs(now, kImuRetryMs)) {
        return;
      }
      restartMs = now;
      if (bus_.probe(kImuAddress) != Status::Ok) {
        return;
      }
      invalidate();
      memset(outSeq, 0, sizeof outSeq);
      resetSeen = false;
      sh2InitSeen = false;
      const uint8_t reset = kImuCommandReset;
      if (sendPacket(bus_, kShtpChannelExecutable, &reset, 1) == Status::Ok) {
        restart = Restart::ResetSent;
        LOG_I(Imu, "restart") { line.field("step", "reset"); }
      }
      return;
    }

    case Restart::ResetSent:
      // All that's waiting, not one per call. service() runs once per cycle, about 650 ms, and the
      // advertisement and reset complete both have to come in before the time limit below
      for (uint8_t read = 0; read < kImuRestartReads && !sh2InitSeen; ++read) {
        if (readPacket(bus_) != Status::Ok || packetLength == 0) {
          break;
        }
        handlePacket();
        system_.waitMs(0);
      }
      // same as begin, the sensor hub's init or the old settle after reset complete
      if (sh2InitSeen || (resetSeen && elapsed >= kImuBootSettleMs)) {
        firstResetSeen_ = true;
        invalidate();
        restart = Restart::Enabling;
        restartIndex = 0;
        restartMs = now;
      } else if (elapsed >= kImuResetWaitMs + kImuBootSettleMs) {
        restart = Restart::Idle;
        restartMs = now;
        LOG_W(Imu, "restart") {
          line.field("ok", false);
          line.field("step", resetSeen ? "sh2" : "reset");
        }
      }
      return;

    case Restart::Enabling:
      // one enable per call, not confirmed. The reports showing up in update() are the check
      if (sendSetFeature(bus_, restartIndex, static_cast<uint32_t>(reportIntervalMs_) * 1000UL) != Status::Ok) {
        restart = Restart::Idle;
        restartMs = now;
        LOG_W(Imu, "restart") {
          line.field("ok", false);
          line.field("step", "enable");
        }
        return;
      }
      if (++restartIndex < 4) {
        return;
      }
      restart = Restart::Idle;
      resetSeen = false;
      sleeping_ = false;
      ready_ = true;
      LOG_I(Imu, "restart") { line.field("ok", true); }
      return;
  }
}

bool Imu::configureReports(uint16_t startupDelayMs)
{
  if (startupDelayMs > 0) {
    system_.waitMs(startupDelayMs);
  }

  // okay once sent. Confirmed is the extra check the library didn't have, it only retries and logs
  bool okay[4] = {false, false, false, false};
  uint8_t unconfirmed = 0;
  const uint32_t intervalUs = static_cast<uint32_t>(reportIntervalMs_) * 1000UL;
  for (uint8_t i = 0; i < 4; ++i) {
    Enable result = Enable::NotSent;
    for (uint8_t attempt = 0; attempt < kImuEnableAttempts && result != Enable::Confirmed; ++attempt) {
      const Enable tried = enableReport(bus_, system_, i, intervalUs);
      if (tried != Enable::NotSent) {
        result = tried;
      }
      tries_[i] = attempt + 1;
      system_.waitMs(tried == Enable::Confirmed ? kImuEnableSpacingMs : kImuEnableRetryDelayMs);
    }
    okay[i] = result != Enable::NotSent;
    if (result == Enable::Sent) {
      unconfirmed |= static_cast<uint8_t>(1U << i);
    }
  }

  if (unconfirmed != 0) {
    LOG_W(Imu, "reports") { line.fieldBits("unconfirmed", unconfirmed, 4); }
  }
  const bool allOkay = okay[0] && okay[1] && okay[2] && okay[3];
  if (allOkay) {
    LOG_I(Imu, "reports") { writeReportResults(line, okay[0], okay[1], okay[2], okay[3], tries_); }
  } else {
    LOG_W(Imu, "reports") { writeReportResults(line, okay[0], okay[1], okay[2], okay[3], tries_); }
  }
  return allOkay;
}

Status Imu::waitForFresh(uint16_t timeoutMs)
{
  if (!ready_ || sleeping_) {
    missing_ = kImuAllReports;
    if (!refusedLogged_) {   // once, not every cycle until the restart
      LOG_W(Imu, "refused") {
        line.field("call", "fresh");
        line.field("ready", ready_);
        line.field("sleeping", sleeping_);
      }
      refusedLogged_ = true;
    }
    return Status::NotReady;
  }
  refusedLogged_ = false;

  // Drain what's already queued, those don't count
  update();

  // The valid flags could be from before the last TX, so each timestamp has to move
  uint32_t previousMs[4];
  for (uint8_t i = 0; i < 4; ++i) {
    previousMs[i] = *updatedTime(data_, i);
  }

  uint8_t freshReports = 0;
  const uint32_t startMs = system_.nowMs();

  do {
    update();
    if (!ready_) {
      missing_ = kImuAllReports;
      return Status::NotReady;   // it reset, restartStep() has it now
    }

    for (uint8_t i = 0; i < 4; ++i) {
      if (*validFlag(data_, i) && *updatedTime(data_, i) != previousMs[i]) {
        freshReports |= static_cast<uint8_t>(1U << i);
      }
    }

    if (freshReports == kImuAllReports) {
      missing_ = 0;
      emptyWaits_ = 0;
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

  // A hub that takes our writes but sends nothing would cost this whole wait every cycle, and the
  // stuck check's soft resets run out. After a few waits with no report at all it's restarted
  emptyWaits_ = freshReports == 0 ? static_cast<uint8_t>(emptyWaits_ + 1) : 0;
  if (emptyWaits_ >= kImuLostAfterEmptyWaits) {
    LOG_W(Imu, "lost") { line.field("emptywaits", emptyWaits_); }
    lose(false);
  }
  return Status::Timeout;
}

bool Imu::update()
{
  if (!ready_ || sleeping_) {
    return false;
  }

  // One packet per read, keep going while reports come in
  for (uint8_t pass = 0; pass < kImuMaxServicePasses && !resetSeen; ++pass) {
    reportThisPass = false;
    if (readPacket(bus_) != Status::Ok || packetLength == 0) {
      break;
    }
    handlePacket();
    if (!reportThisPass) {
      break;
    }
    system_.waitMs(0);   // GPS UART and watchdog, 16 passes can pass 40 ms
  }

  if (resetSeen) {
    // The IMU reset by itself (begin() and restartStep() take the ones they asked for). The reports
    // go back on in the background, blocking here took up to 4.5 s. resetSeen stays set for it
    LOG_W(Imu, "reset") { line.field("latched", !firstResetSeen_); }
    firstResetSeen_ = true;
    lose(true);
    return false;
  }

  const uint32_t now = system_.nowMs();
  bool updated = false;

  ChipSatSensors::Vector3f *const vectors[3] = {&data_.linearAccelerationMps2, &data_.gyroscopeRadPerSec,
                                                 &data_.magnetometerMicroTesla};

  for (uint8_t i = 0; i < 4; ++i) {
    events_[i] = slots[i].count;
    if (!slots[i].fresh) {
      continue;
    }
    const float *v = slots[i].v;
    if (i < 3) {
      vectors[i]->x = v[0];
      vectors[i]->y = v[1];
      vectors[i]->z = v[2];
    } else {
      data_.orientation.i = v[0];
      data_.orientation.j = v[1];
      data_.orientation.k = v[2];
      data_.orientation.real = v[3];
    }
    *validFlag(data_, i) = true;
    *updatedTime(data_, i) = now;
    slots[i].fresh = false;
    updated = true;
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
  const uint8_t command = kImuCommandSleep;
  const Status status = sendPacket(bus_, kShtpChannelExecutable, &command, 1);
  if (status != Status::Ok) {
    LOG_W(Imu, "sleep") {
      line.field("ok", false);
      line.field("status", ChipSatPlatform::statusName(status));
    }
    return status;   // the library would have retried this write forever
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
  const uint8_t command = kImuCommandOn;
  const Status status = sendPacket(bus_, kShtpChannelExecutable, &command, 1);
  if (status != Status::Ok) {
    LOG_W(Imu, "wake") {
      line.field("ok", false);
      line.field("step", "modeon");
    }
    return status;   // still flagged asleep
  }

  // Clear first, waitForFresh below needs update() to run
  sleeping_ = false;

  // The BNO085 datasheet says on brings the reports back by itself (1.3.1). Not checked yet, so they're
  // turned on again like before. Unlike the reset path, a failure here doesn't mark the IMU lost
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
    return Status::NotReady;   // restartStep() already has it
  }

  resetSeen = false;
  sh2InitSeen = false;
  const uint8_t command = kImuCommandReset;
  const Status status = sendPacket(bus_, kShtpChannelExecutable, &command, 1);
  if (status == Status::Ok) {
    lose(true);   // restartStep() waits for reset complete and turns the reports back on
  }

  LOG_W(Imu, "softreset") { line.field("ok", status == Status::Ok); }
  return status;
}

// Hands the IMU to restartStep(), starting on the next service(). resetSent: a reset is already on
// its way (resetHub or the IMU resetting by itself), so the restart waits for it instead of sending
// another
void Imu::lose(bool resetSent)
{
  invalidate();
  ready_ = false;
  sleeping_ = false;
  emptyWaits_ = 0;
  restart = resetSent ? Restart::ResetSent : Restart::Idle;
  restartMs = resetSent ? system_.nowMs() : system_.nowMs() - kImuRetryMs;
}

void Imu::invalidate()
{
  for (uint8_t i = 0; i < 4; ++i) {
    *validFlag(data_, i) = false;
    slots[i].fresh = false;
  }
  data_.valid = false;
}

void Imu::updateOverallValidity()
{
  data_.valid = data_.linearAccelerationValid && data_.gyroscopeValid && data_.magnetometerValid &&
                data_.orientationValid;
}

} // namespace ChipSatDevices

#endif
