#include "Health.h"
#include "../config.h"
#include "../log/Log.h"
#include "../log/SystemEvent.h"

namespace ChipSatApp
{

using namespace ChipSatConfig;
using namespace ChipSatLog;

// CHIPSAT_LOG_AT with a run-time tag
#define LOG_RECOVER(minLevel, letter, tagValue) \
  if constexpr (!(CHIPSAT_LOG_LEVEL >= (minLevel))) {} else \
    switch (ChipSatLog::Line line{letter, tagValue, "recover"}; 0) default:

static Tag tagOf(uint8_t part)
{
  switch (part) {
    case 0: return Tag::Imu;
    case 1: return Tag::Env;
    case 2: return Tag::Soc;
    default: return Tag::Gps;
  }
}

static uint8_t eventPart(uint8_t part)
{
  switch (part) {
    case 0: return kPartImu;
    case 1: return kPartEnv;
    case 2: return kPartGauge;
    default: return kPartGps;
  }
}

static const char *reasonWord(uint8_t why)
{
  switch (why) {
    case kWhyAsleep: return "asleep";
    case kWhySilent: return "silent";
    case kWhyPartial: return "partial";
    case kWhyStale: return "stale";
    case kWhyNotReady: return "notready";
    case kWhyNoPvt: return "nopvt";
    case kWhyNoBytes: return "nobytes";
  }
  return "?";
}

Health::Health(ChipSatPlatform::System &system, ChipSatPlatform::I2cBus &bus, ChipSatDevices::Imu &imu,
               ChipSatDevices::Gps &gps, ChipSatDevices::EnvSensor &env, ChipSatDevices::FuelGauge &gauge)
  : system_(system), bus_(bus), imuPart_(imu), gpsPart_(gps), envPart_(env), gaugePart_(gauge),
    imu_(WatchRules{kImuWatchRetryMs, kImuWatchSettleMs, 2}),
    env_(WatchRules{kEnvWatchRetryMs, kEnvWatchSettleMs, 1}),
    gauge_(WatchRules{kGaugeWatchRetryMs, kGaugeWatchSettleMs, 1}),
    gps_(WatchRules{kGpsWatchRetryMs, kGpsWatchSettleMs, 2})
{
}

void Health::begin()
{
  beganMs_ = system_.nowMs();
  readsSinceMs_ = beganMs_;
  gpsSinceMs_ = beganMs_;
  imuUp_ = false;
}

void Health::expect(uint32_t readGapMs, bool gpsOn)
{
  const uint32_t now = system_.nowMs();
  if (readGapMs != readGapMs_) {
    readGapMs_ = readGapMs;
    readsSinceMs_ = now;
  }
  if (gpsOn != gpsOn_) {
    gpsOn_ = gpsOn;
    gpsSinceMs_ = now;
  }
}

SensorWatch &Health::watch(Part part)
{
  switch (part) {
    case Part::Imu: return imu_;
    case Part::Env: return env_;
    case Part::Gauge: return gauge_;
    default: return gps_;
  }
}

// Not ready: restartStep() has it. Nothing puts the IMU to sleep in flight, so asleep is a fault
Seen Health::judgeImu(uint32_t nowMs, bool &partial)
{
  partial = false;
  if (!imuPart_.ready()) {
    imuUp_ = false;
    return Seen::Unknown;
  }
  if (imuPart_.sleeping()) {
    imuUp_ = false;
    imuSilent_ = 0x0F;
    return Seen::Bad;
  }
  const uint32_t *counts = imuPart_.eventCounts();
  if (imuPart_.reportIntervalMs() != imuIntervalMs_) {
    imuIntervalMs_ = imuPart_.reportIntervalMs();
    imuUp_ = false;   // a new pace gets the grace again
  }
  if (!imuUp_) {
    imuUp_ = true;
    imuUpMs_ = nowMs;
    imuMoved_ = 0;
    for (uint8_t i = 0; i < 4; ++i) {
      imuCounts_[i] = counts[i];
      imuMovedMs_[i] = nowMs;
    }
  }
  for (uint8_t i = 0; i < 4; ++i) {
    if (counts[i] != imuCounts_[i]) {
      imuCounts_[i] = counts[i];
      imuMovedMs_[i] = nowMs;
      imuMoved_ |= static_cast<uint8_t>(1U << i);
    }
  }
  if (nowMs - imuUpMs_ < kImuGraceMs) {
    return imuMoved_ == 0x0F ? Seen::Good : Seen::Unknown;
  }
  // as many reports missed at 1 Hz as at 20 Hz
  const uint32_t silentMs = kImuSilentMs * imuIntervalMs_ / kImuReportIntervalMs;
  uint8_t silent = 0;
  for (uint8_t i = 0; i < 4; ++i) {
    if (nowMs - imuMovedMs_[i] > silentMs) {
      silent |= static_cast<uint8_t>(1U << i);
    }
  }
  imuSilent_ = silent;
  if (silent == 0) {
    return Seen::Good;
  }
  partial = silent != 0x0F;
  return Seen::Bad;
}

// age of the last good reading only, valid drops on a single failed read
Seen Health::judgeEnv(uint32_t nowMs) const
{
  const ChipSatSensors::EnvironmentalData &d = envPart_.data();
  const uint32_t limitMs = kEnvStaleMs + readGapMs_;
  if (d.updatedMs != 0 && nowMs - d.updatedMs <= limitMs) {
    return Seen::Good;
  }
  return nowMs - readsSinceMs_ <= limitMs ? Seen::Unknown : Seen::Bad;
}

Seen Health::judgeGauge(uint32_t nowMs) const
{
  const ChipSatSensors::StateOfChargeData &d = gaugePart_.data();
  const uint32_t limitMs = kGaugeStaleMs + readGapMs_;
  if (d.updatedMs != 0 && nowMs - d.updatedMs <= limitMs) {
    return Seen::Good;
  }
  return nowMs - readsSinceMs_ <= limitMs ? Seen::Unknown : Seen::Bad;
}

// any NAV-PVT counts, fix or not. No fix must never reset the receiver, and in standby it says nothing
Seen Health::judgeGps(uint32_t nowMs) const
{
  if (!gpsOn_) {
    return Seen::Unknown;
  }
  if (gpsPart_.pvtSeen() && nowMs - gpsPart_.lastPvtMs() <= kGpsSilentMs) {
    return Seen::Good;
  }
  return nowMs - gpsSinceMs_ <= kGpsSilentMs ? Seen::Unknown : Seen::Bad;
}

uint8_t Health::reason(Part part, uint32_t nowMs) const
{
  switch (part) {
    case Part::Imu:
      return imuPart_.sleeping() ? kWhyAsleep : imuSilent_ == 0x0F ? kWhySilent : kWhyPartial;
    case Part::Env:
      return envPart_.ready() ? kWhyStale : kWhyNotReady;
    case Part::Gauge:
      return gaugePart_.ready() ? kWhyStale : kWhyNotReady;
    case Part::Gps:
      return gpsPart_.byteSeen() && nowMs - gpsPart_.lastByteMs() <= kGpsSilentMs ? kWhyNoPvt : kWhyNoBytes;
  }
  return 0;
}

// The flash gets the event whatever the log level, so it's noted outside the console line
void Health::report(Part part, WatchChange change, uint32_t nowMs)
{
  const SensorWatch &w = watch(part);
  const Tag tag = tagOf(static_cast<uint8_t>(part));
  const uint8_t where = eventPart(static_cast<uint8_t>(part));
  const uint8_t why = reason(part, nowMs);
  switch (change) {
    case WatchChange::None:
      return;
    case WatchChange::WentBad:
      noteSystem(kSourceSensor, kSensorRetry, where, why, part == Part::Imu ? imuSilent_ : 0, nowMs);
      LOG_RECOVER(2, 'W', tag) {
        line.field("step", "retry");
        line.field("reason", reasonWord(why));
        if (part == Part::Imu) {
          line.fieldBits("silent", imuSilent_, 4);
        }
      }
      return;
    case WatchChange::Deferred:
      noteSystem(kSourceSensor, kSensorDefer, where, kWhyPartial, imuSilent_, nowMs);
      LOG_RECOVER(2, 'W', tag) {
        line.field("step", "defer");
        line.field("reason", "partial");
        line.fieldBits("silent", imuSilent_, 4);
      }
      return;
    case WatchChange::Aside:
      noteSystem(kSourceSensor, kSensorAside, where, why, w.backoffMs(), nowMs);
      LOG_RECOVER(2, 'W', tag) {
        line.field("step", "aside");
        line.field("reason", reasonWord(why));
        line.field("backoffms", w.backoffMs());
        line.field("attempts", static_cast<unsigned int>(w.attempts()));
      }
      return;
    case WatchChange::Recovered:
      noteSystem(kSourceSensor, kSensorOk, where, 0, nowMs - w.badSinceMs(), nowMs);
      LOG_RECOVER(3, 'I', tag) {
        line.field("step", "ok");
        line.field("downms", nowMs - w.badSinceMs());
        line.field("attempts", static_cast<unsigned int>(w.attempts()));
      }
      return;
  }
}

bool Health::act(Part part, uint32_t nowMs)
{
  SensorWatch &w = watch(part);
  const uint8_t which = w.resetIndex();
  const uint8_t why = reason(part, nowMs);
  const char *how = "start";
  bool ok = true;
  switch (part) {
    case Part::Imu:
      if (which == 0) {
        how = "softreset";
        ok = imuPart_.resetHub() == ChipSatPlatform::Status::Ok;
      } else {
        // the background restart only sends the soft reset again, the pin reaches a hub that ignores it
        how = "pinreset";
        imuPart_.pinReset(why);
      }
      imuUp_ = false;
      break;
    case Part::Env:
      ok = envPart_.restart();
      break;
    case Part::Gauge:
      ok = gaugePart_.restart();
      break;
    case Part::Gps:
      if (which == 0) {
        how = "port";
        gpsPart_.restartPort();
      } else {
        how = "hotstart";
        gpsPart_.hotStart();
      }
      break;
  }
  w.didReset(nowMs);
  noteSystem(kSourceSensor, which == 0 ? kSensorReset : kSensorRestart, eventPart(static_cast<uint8_t>(part)), why,
             ok ? 1 : 0, nowMs);
  LOG_RECOVER(2, 'W', tagOf(static_cast<uint8_t>(part))) {
    line.field("step", "reset");
    line.field("how", how);
    line.field("attempt", static_cast<unsigned int>(w.attempts()));
    line.field("reason", reasonWord(why));
    line.field("ok", ok);
    line.field("ms", system_.nowMs() - nowMs);
  }
  return ok;
}

void Health::service(bool air)
{
  const uint32_t now = system_.nowMs();

  bool partial = false;
  const Seen imuSeen = judgeImu(now, partial);
  report(Part::Imu, imu_.observe(now, imuSeen, partial, air), now);
  report(Part::Env, env_.observe(now, judgeEnv(now), false, air), now);
  report(Part::Gauge, gauge_.observe(now, judgeGauge(now), false, air), now);
  report(Part::Gps, gps_.observe(now, judgeGps(now), false, air), now);

  if (actedAny_ && now - lastActionMs_ < kHealthGapMs) {
    return;
  }
  if (bus_.imuResetDue(now)) {
    imuPart_.pinReset(kWhyBusStuck);
  }
  if (bus_.recover(now)) {
    lastActionMs_ = now;
    actedAny_ = true;
    return;
  }
  if (bus_.faulted(now)) {
    return;
  }

  static const Part kAir[4] = {Part::Imu, Part::Env, Part::Gps, Part::Gauge};
  static const Part kGround[4] = {Part::Gps, Part::Imu, Part::Env, Part::Gauge};
  const Part *order = air ? kAir : kGround;
  for (uint8_t i = 0; i < 4; ++i) {
    if (watch(order[i]).due()) {
      act(order[i], now);
      lastActionMs_ = now;
      actedAny_ = true;
      return;
    }
  }
}

} // namespace ChipSatApp
