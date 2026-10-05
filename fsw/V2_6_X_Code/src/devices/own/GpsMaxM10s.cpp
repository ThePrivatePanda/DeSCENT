#include <Arduino.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_GPS_DRIVER == CHIPSAT_DRIVER_OWN

#include "../Gps.h"
#include "../../log/Log.h"

namespace ChipSatDevices
{

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

static uint8_t frame[kNavPvtLength + 8];   // one NAV-PVT, framed
static uint16_t framePayloadLength = 0;
static uint8_t frameClass = 0;
static uint8_t frameId = 0;

// Parser state, kept between calls since a frame can arrive over several
enum class Step : uint8_t { Sync1, Sync2, Class, Id, LenLo, LenHi, Payload, CkA, CkB };
static Step step = Step::Sync1;
static uint16_t parseIndex = 0;
static uint16_t parseLength = 0;
static uint8_t ckA = 0;
static uint8_t sumA = 0, sumB = 0;

// What came in since the last read(), for its log line
static uint16_t bytesIn = 0;
static uint16_t framesIn = 0;
static uint16_t badSums = 0;
static uint16_t notUbx = 0;       // NMEA, if it's still on
static uint16_t mostQueued = 0;   // 63 means the port's 64 byte buffer filled up
static uint32_t frameStartMs = 0;   // when the frame being read now started arriving

// True when this byte completes a frame with a good checksum, the frame is then in frame[]
static bool feedUbx(uint8_t byte)
{
  switch (step) {
    case Step::Sync1:
      if (byte == kUbxSync1) {
        step = Step::Sync2;
      } else {
        ++notUbx;
      }
      break;
    case Step::Sync2:
      step = (byte == kUbxSync2) ? Step::Class : Step::Sync1;
      break;
    case Step::Class:
      frameClass = byte;
      sumA = byte;
      sumB = byte;
      step = Step::Id;
      break;
    case Step::Id:
      frameId = byte;
      sumA = static_cast<uint8_t>(sumA + byte);
      sumB = static_cast<uint8_t>(sumB + sumA);
      step = Step::LenLo;
      break;
    case Step::LenLo:
      parseLength = byte;
      sumA = static_cast<uint8_t>(sumA + byte);
      sumB = static_cast<uint8_t>(sumB + sumA);
      step = Step::LenHi;
      break;
    case Step::LenHi:
      parseLength |= static_cast<uint16_t>(byte) << 8;
      sumA = static_cast<uint8_t>(sumA + byte);
      sumB = static_cast<uint8_t>(sumB + sumA);
      parseIndex = 0;
      if (parseLength > sizeof frame) {
        step = Step::Sync1;   // false sync, nothing we read is this long
      } else {
        step = (parseLength == 0) ? Step::CkA : Step::Payload;
      }
      break;
    case Step::Payload:
      frame[parseIndex] = byte;
      sumA = static_cast<uint8_t>(sumA + byte);
      sumB = static_cast<uint8_t>(sumB + sumA);
      if (++parseIndex >= parseLength) {
        step = Step::CkA;
      }
      break;
    case Step::CkA:
      ckA = byte;
      step = Step::CkB;
      break;
    case Step::CkB:
      step = Step::Sync1;   // done either way, a bad frame just resynchronises
      if (ckA != sumA || byte != sumB) {
        ++badSums;
      } else {
        ++framesIn;
        framePayloadLength = parseLength;
        return true;
      }
      break;
  }
  return false;
}

static void clearCounts()
{
  bytesIn = 0;
  framesIn = 0;
  badSums = 0;
  notUbx = 0;
  mostQueued = 0;
}

static void dropQueued(ChipSatPlatform::Uart &port)
{
  while (port.read() >= 0) {
  }
  step = Step::Sync1;
}

Gps::Gps(ChipSatPlatform::Uart &port, ChipSatPlatform::System &system)
  : port_(port), system_(system)
{
}

void Gps::sendUbx(uint8_t cls, uint8_t id, const uint8_t *payload, uint16_t length)
{
  const uint8_t header[4] = {cls, id, static_cast<uint8_t>(length), static_cast<uint8_t>(length >> 8)};

  // Fletcher-8 over the header and payload, everything between the sync bytes and the checksum
  uint8_t sum[2] = {0, 0};
  for (uint16_t i = 0; i < sizeof header; ++i) {
    sum[0] = static_cast<uint8_t>(sum[0] + header[i]);
    sum[1] = static_cast<uint8_t>(sum[1] + sum[0]);
  }
  for (uint16_t i = 0; i < length; ++i) {
    sum[0] = static_cast<uint8_t>(sum[0] + payload[i]);
    sum[1] = static_cast<uint8_t>(sum[1] + sum[0]);
  }

  const uint8_t sync[2] = {kUbxSync1, kUbxSync2};
  port_.write(sync, sizeof sync);
  port_.write(header, sizeof header);
  if (length != 0) {
    port_.write(payload, length);
  }
  port_.write(sum, sizeof sum);
}

// Reads until one whole frame is in, or the deadline passes
bool Gps::readUbx(uint32_t deadlineMs)
{
  while (system_.nowMs() < deadlineMs) {
    const int next = port_.read();
    if (next >= 0 && feedUbx(static_cast<uint8_t>(next))) {
      return true;
    }
  }
  return false;
}

// Sends one frame and waits for the matching reply. Anything else that arrives is parsed and dropped
bool Gps::waitForUbx(uint8_t cls, uint8_t id, uint16_t maxWaitMs)
{
  const uint32_t deadlineMs = system_.nowMs() + maxWaitMs;
  while (system_.nowMs() < deadlineMs) {
    if (!readUbx(deadlineMs)) {
      return false;
    }
    if (frameClass == cls && frameId == id) {
      // An acknowledgement carries the class and id it answers, so check it is ours
      if (cls != kUbxClassAck || (framePayloadLength == 2 && frame[0] == kUbxClassCfg && frame[1] == kUbxIdValset)) {
        return true;
      }
    }
    if (frameClass == kUbxClassAck && frameId == kUbxIdNak) {
      return false;
    }
  }
  return false;
}

// One CFG-VALSET, one key, one value
void Gps::sendKey(uint32_t key, uint32_t value, uint8_t valueBytes)
{
  uint8_t payload[12] = {0};
  payload[0] = 0x00;                 // version
  payload[1] = kUbxConfigLayers;     // RAM + battery backed
  payload[4] = static_cast<uint8_t>(key);
  payload[5] = static_cast<uint8_t>(key >> 8);
  payload[6] = static_cast<uint8_t>(key >> 16);
  payload[7] = static_cast<uint8_t>(key >> 24);
  for (uint8_t i = 0; i < valueBytes; ++i) {
    payload[8 + i] = static_cast<uint8_t>(value >> (8 * i));
  }
  sendUbx(kUbxClassCfg, kUbxIdValset, payload, static_cast<uint16_t>(8 + valueBytes));
}

// Same, then wait for the acknowledgement
bool Gps::setKey(uint32_t key, uint32_t value, uint8_t valueBytes)
{
  dropQueued(port_);   // so the acknowledgement is the next thing we see
  sendKey(key, value, valueBytes);
  return waitForUbx(kUbxClassAck, kUbxIdAck, kGpsAckWaitMs);
}

// All six settings again, no waiting for acks. On V2.5.0 V_BCKP is tied to VCC, so a receiver that
// browns out comes back on factory settings (NMEA on, no NAV-PVT, portable model) while we keep running
void Gps::resendConfig()
{
  sendKey(kUbxCfgUart1OutprotNmea, 0, 1);
  sendKey(kUbxCfgMsgoutNavPvtUart1, 1, 1);
  sendKey(kUbxCfgNavspgDynmodel, kGpsDynamicModel, 1);
  sendKey(kUbxCfgRateMeas, kGpsMeasurementIntervalMs, 2);
  sendKey(kUbxCfgRateNav, kGpsNavigationRate, 2);
  sendKey(kUbxCfgPmOperateMode, kGpsOperateModeFull, 1);
}

// CFG-RST is the one message the receiver does not acknowledge: it acts on it and restarts, so
// there is nothing to wait for. Clearing only the stored position keeps the almanac and ephemeris,
// so the next fix is still a warm start
void Gps::clearStoredPosition()
{
  const uint8_t payload[4] = {static_cast<uint8_t>(kUbxBbrMaskPositionOnly),
                              static_cast<uint8_t>(kUbxBbrMaskPositionOnly >> 8),
                              kUbxResetModeGnssOnly, 0x00};
  sendUbx(kUbxClassCfg, kUbxIdReset, payload, sizeof payload);
  system_.waitMs(kGpsResetSettleMs);

  dropQueued(port_);   // anything it said while restarting is from before the wipe

  LOG_I(Gps, "wipe") { line.fieldHex("bbr", kUbxBbrMaskPositionOnly, 4); }
}

bool Gps::configureAirborne()
{
  return setKey(kUbxCfgNavspgDynmodel, kGpsDynamicModel, 1);
}

// NMEA off, and NAV-PVT after every solution without being asked
bool Gps::configureOutput()
{
  const bool nmeaOff = setKey(kUbxCfgUart1OutprotNmea, 0, 1);
  const bool pvtOn = setKey(kUbxCfgMsgoutNavPvtUart1, 1, 1);
  outputOkay_ = nmeaOff && pvtOn;
  return outputOkay_;
}

bool Gps::configureNormalContinuous()
{
  rateOkay_ = setKey(kUbxCfgRateMeas, kGpsMeasurementIntervalMs, 2);
  navOkay_ = setKey(kUbxCfgRateNav, kGpsNavigationRate, 2);
  modeOkay_ = setKey(kUbxCfgPmOperateMode, kGpsOperateModeFull, 1);
  return rateOkay_ && navOkay_ && modeOkay_;
}

Status Gps::begin()
{
  LOG_I(Gps, "start");
  const uint32_t startMs = system_.nowMs();

  if (kGpsClearStoredPosition) {
    clearStoredPosition();
  }

  bool airborneOkay = false;
  for (uint8_t attempt = 0; attempt < kGpsInitAttempts; attempt++) {
    // Separate bools so every step still runs if one fails. Output first, with NMEA on the other
    // acks queue behind it and time out
    const bool outputOkay = configureOutput();
    airborneOkay = configureAirborne();
    const bool normalModeOkay = configureNormalContinuous();

    ready_ = airborneOkay && normalModeOkay && outputOkay;
    lastConfigMs_ = system_.nowMs();

    if (ready_) {
      clearCounts();   // the NMEA from before the settings took must not look like a lost config
      LOG_I(Gps, "init") {
        line.field("ok", true);
        line.field("attempts", attempt + 1);
        line.field("dyn", airborneOkay);
        line.field("rate", rateOkay_);
        line.field("nav", navOkay_);
        line.field("mode", modeOkay_);
        line.field("out", outputOkay_);
        line.field("dynmodel", kGpsDynamicModel);
        line.field("ms", system_.nowMs() - startMs);
      }
      return Status::Ok;
    }

    if (attempt + 1 < kGpsInitAttempts) {
      system_.waitMs(kInitRetryDelayMs);
    }
  }

  // Not fatal, read() polls when nothing arrives and resends the settings
  clearCounts();
  LOG_E(Gps, "init") {
    line.field("ok", false);
    line.field("reason", "config");
    line.field("attempts", kGpsInitAttempts);
    line.field("dyn", airborneOkay);
    line.field("rate", rateOkay_);
    line.field("nav", navOkay_);
    line.field("mode", modeOkay_);
    line.field("out", outputOkay_);
    line.field("ms", system_.nowMs() - startMs);
  }
  return Status::NoAck;
}

static int32_t readS32(const uint8_t *from)
{
  return static_cast<int32_t>(static_cast<uint32_t>(from[0]) | static_cast<uint32_t>(from[1]) << 8 |
                              static_cast<uint32_t>(from[2]) << 16 | static_cast<uint32_t>(from[3]) << 24);
}

// Runs from every System::waitMs and from read(), so the 64 byte buffer never fills
void Gps::service()
{
  const size_t queued = port_.available();
  if (queued > mostQueued) {
    mostQueued = static_cast<uint16_t>(queued);
  }
  int next;
  while ((next = port_.read()) >= 0) {
    ++bytesIn;
    const bool wasIdle = step == Step::Sync1;
    const bool complete = feedUbx(static_cast<uint8_t>(next));
    if (wasIdle && step == Step::Sync2) {
      // the time in the frame belongs to this moment, not to when the last byte turns up 100 ms later
      frameStartMs = system_.nowMs();
    }
    if (complete && frameClass == kUbxClassNav && frameId == kUbxIdNavPvt &&
        framePayloadLength == kNavPvtLength) {
      takePvt();
    }
  }
}

void Gps::takePvt()
{
  lastPvtMs_ = system_.nowMs();
  pvtSeen_ = true;

  const int32_t longitudeE7 = readS32(&frame[kNavPvtLon]);
  const int32_t latitudeE7 = readS32(&frame[kNavPvtLat]);
  const int32_t altitudeMm = readS32(&frame[kNavPvtHeightMsl]);
  satellites_ = frame[kNavPvtNumSatellites];

  // Airborne models don't do 2D fixes ("No 2D position fixes supported", DYN_MODEL_AIRBORNE4g in the
  // u-blox library header)
  fixType_ = frame[kNavPvtFixType];
  fixOk_ = (frame[kNavPvtFlags] & kNavPvtGnssFixOk) != 0;
  llhChecked_ = fixType_ == kNavPvtFix3d || fixType_ == kNavPvtFixGnssDeadReckoning;
  invalidLlh_ = false;
  if (llhChecked_) {
    const uint16_t flags3 = static_cast<uint16_t>(frame[kNavPvtFlags3]) |
                            static_cast<uint16_t>(frame[kNavPvtFlags3 + 1]) << 8;
    invalidLlh_ = (flags3 & kNavPvtInvalidLlh) != 0;
  }

  inRange_ = latitudeE7 >= -kMaxLatitudeE7 && latitudeE7 <= kMaxLatitudeE7 &&
             longitudeE7 >= -kMaxLongitudeE7 && longitudeE7 <= kMaxLongitudeE7 &&
             altitudeMm >= kMinAltitudeMm && altitudeMm <= kMaxAltitudeMm;
  const bool trustworthy = llhChecked_ && fixOk_ && !invalidLlh_ && satellites_ >= kGpsMinSatellites && inRange_;

  // Only a fix we trust replaces the stored position. Anything else leaves the last good one in
  // place, marked invalid, so the packet never carries a converging solution as if it were real
  if (trustworthy) {
    data_.longitudeE7 = longitudeE7;
    data_.latitudeE7 = latitudeE7;
    data_.altitudeMSLmm = altitudeMm;
    data_.updatedMs = lastPvtMs_;
  }
  data_.valid = trustworthy;

  takeUtc();
}

// Days since 1970 for a civil date, Howard Hinnant's days_from_civil
static int32_t daysFromCivil(int32_t y, uint32_t m, uint32_t d)
{
  y -= m <= 2;
  const int32_t era = (y >= 0 ? y : y - 399) / 400;
  const uint32_t yoe = static_cast<uint32_t>(y - era * 400);
  const uint32_t doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
  const uint32_t doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  return era * 146097 + static_cast<int32_t>(doe) - 719468;
}

// UTC out of the same frame. The receiver knows the time before it knows where it is, but it also
// reports a plausible looking date before that time is any good, so wait for its own fully resolved bit
void Gps::takeUtc()
{
  const uint8_t valid = frame[kNavPvtValid];
  utcBits_ = valid;
  const bool resolved = (valid & (kNavPvtValidDate | kNavPvtValidTime | kNavPvtFullyResolved)) ==
                        (kNavPvtValidDate | kNavPvtValidTime | kNavPvtFullyResolved);
  if (!resolved) {
    utcValid_ = false;
    return;
  }

  const uint16_t year = static_cast<uint16_t>(frame[kNavPvtYear] | frame[kNavPvtYear + 1] << 8);
  const uint8_t month = frame[kNavPvtMonth];
  const uint8_t day = frame[kNavPvtDay];
  const uint8_t hour = frame[kNavPvtHour];
  const uint8_t minute = frame[kNavPvtMinute];
  const uint8_t second = frame[kNavPvtSecond];
  if (year < 2020 || year > 2100 || month < 1 || month > 12 || day < 1 || day > 31 ||
      hour > 23 || minute > 59 || second > 60) {
    utcValid_ = false;
    return;
  }

  const int32_t days = daysFromCivil(year, month, day);
  utcEpoch_ = static_cast<uint32_t>(days) * 86400UL + hour * 3600UL + minute * 60UL + second;
  utcNano_ = readS32(&frame[kNavPvtNano]);
  utcAccNs_ = static_cast<uint32_t>(readS32(&frame[kNavPvtTimeAcc]));
  utcUptimeMs_ = frameStartMs;
  utcValid_ = true;
}

// Never waits
Status Gps::read()
{
  service();

  // A receiver that browned out is back on factory settings. It still answers our polls, so positions
  // alone may keep looking fresh, but its NMEA gives it away
  const bool stale = !pvtSeen_ || system_.nowMs() - lastPvtMs_ > kGpsPvtMaxAgeMs;
  const bool nmeaOn = notUbx > kGpsNmeaLostBytes;
  if ((stale || nmeaOn) && system_.nowMs() - lastConfigMs_ >= kGpsReconfigMs) {
    resendConfig();
    lastConfigMs_ = system_.nowMs();
    LOG_W(Gps, "reconfig") {
      line.field("stale", stale);
      line.field("notubx", notUbx);
    }
  }

  // Nothing lately. Poll once, the answer gets picked up next time
  if (stale) {
    sendUbx(kUbxClassNav, kUbxIdNavPvt, nullptr, 0);
    data_.valid = false;   // old coordinates stay
    LOG_W(Gps, "read") {
      line.field("ok", false);
      line.field("reason", "pvt");
      line.field("bytes", bytesIn);
      line.field("frames", framesIn);
      line.field("bad", badSums);
      line.field("notubx", notUbx);
      line.field("queued", mostQueued);
    }
    clearCounts();
    return Status::Timeout;
  }

  if (!data_.valid) {
    LOG_W(Gps, "read") {
      line.field("ok", false);
      line.field("reason", inRange_ ? "nofix" : "range");
      line.field("fix", fixType_);
      line.field("fixok", fixOk_);
      line.field("sats", satellites_);
      line.field("bytes", bytesIn);
      line.field("notubx", notUbx);
      line.field("queued", mostQueued);
    }
    clearCounts();
    return Status::BadData;
  }
  clearCounts();
  return Status::Ok;
}

} // namespace ChipSatDevices

#endif
