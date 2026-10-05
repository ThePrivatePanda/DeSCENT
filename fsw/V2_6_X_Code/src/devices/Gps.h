#pragma once

#include <stdint.h>
#include "../platform/Status.h"
#include "../platform/System.h"
#include "../platform/Uart.h"
#include "SensorData.h"

namespace ChipSatDevices
{

// MAX-M10S on LPUART1. Configured at boot and again when the settings look lost, never put to sleep.
// Which driver is behind this is picked by CHIPSAT_GPS_DRIVER in config.h
class Gps
{
public:
  Gps(ChipSatPlatform::Uart &port, ChipSatPlatform::System &system);

  // The port has to be open already
  ChipSatPlatform::Status begin();

  // Ok only with a trusted 3D fix, received within kGpsPvtMaxAgeMs
  ChipSatPlatform::Status read();

  // Call often, the port buffers 64 bytes
  void service();

  bool ready() const { return ready_; }
  const ChipSatSensors::GPSData &data() const { return data_; }

  // For logging, kept from the last position so nothing has to ask the receiver again
  uint8_t fixType() const { return fixType_; }
  uint8_t satellites() const { return satellites_; }
  bool llhChecked() const { return llhChecked_; }

  // UTC from the receiver, which it has before it has a position. utcValid() only goes true once the
  // receiver calls its own time fully resolved. utcUptimeMs() is when the frame carrying it started
  // arriving, so the two can be lined up
  bool utcValid() const { return utcValid_; }
  uint32_t utcEpoch() const { return utcEpoch_; }      // seconds since 1970
  int32_t utcNano() const { return utcNano_; }         // the fraction of that second
  uint32_t utcAccNs() const { return utcAccNs_; }      // the receiver's own error estimate
  uint8_t utcBits() const { return utcBits_; }         // its validity bits, kept as they came
  uint32_t utcUptimeMs() const { return utcUptimeMs_; }
  bool invalidLlh() const { return invalidLlh_; }

private:
  bool configureAirborne();
  bool configureNormalContinuous();

  // only the own driver uses these, the library one leaves them undefined
  void sendUbx(uint8_t cls, uint8_t id, const uint8_t *payload, uint16_t length);
  bool readUbx(uint32_t deadlineMs);
  bool waitForUbx(uint8_t cls, uint8_t id, uint16_t maxWaitMs);
  void sendKey(uint32_t key, uint32_t value, uint8_t valueBytes);
  bool setKey(uint32_t key, uint32_t value, uint8_t valueBytes);
  void resendConfig();
  void clearStoredPosition();
  bool configureOutput();
  void takePvt();
  void takeUtc();

  ChipSatPlatform::Uart &port_;
  ChipSatPlatform::System &system_;
  ChipSatSensors::GPSData data_;
  bool ready_ = false;

  bool rateOkay_ = false;
  bool navOkay_ = false;
  bool modeOkay_ = false;
  bool outputOkay_ = false;
  bool pvtSeen_ = false;
  uint32_t lastPvtMs_ = 0;
  uint32_t lastConfigMs_ = 0;
  bool inRange_ = true;
  bool utcValid_ = false;
  uint32_t utcEpoch_ = 0;
  int32_t utcNano_ = 0;
  uint32_t utcAccNs_ = 0;
  uint8_t utcBits_ = 0;
  uint32_t utcUptimeMs_ = 0;
  uint8_t fixType_ = 0;
  bool fixOk_ = false;
  uint8_t satellites_ = 0;
  bool llhChecked_ = false;
  bool invalidLlh_ = false;
};

} // namespace ChipSatDevices
