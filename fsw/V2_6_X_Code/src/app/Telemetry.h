#pragma once

#include <Arduino.h>
#include <stddef.h>
#include "../devices/SensorData.h"

namespace ChipSatTelemetry
{

// Sensor validity bit assignments.
// Bits 0 through 6 show whether each stored measurement is valid.
// Bit 7 shows whether every required measurement was freshly collected for
// the current packet cycle.
constexpr uint8_t kLinearAccelerationValidBit = 1U << 0;
constexpr uint8_t kGyroscopeValidBit          = 1U << 1;
constexpr uint8_t kMagnetometerValidBit       = 1U << 2;
constexpr uint8_t kQuaternionValidBit         = 1U << 3;
constexpr uint8_t kGPSValidBit                = 1U << 4;
constexpr uint8_t kStateOfChargeValidBit      = 1U << 5;
constexpr uint8_t kEnvironmentalValidBit      = 1U << 6;
constexpr uint8_t kAllDataFreshBit            = 1U << 7;

constexpr uint8_t kAllSensorDataValidMask =
    kLinearAccelerationValidBit |
    kGyroscopeValidBit |
    kMagnetometerValidBit |
    kQuaternionValidBit |
    kGPSValidBit |
    kStateOfChargeValidBit |
    kEnvironmentalValidBit;

struct __attribute__((packed)) TelemetryPacket
{
  // GPS and environmental altitude: 16 bytes
  int32_t latitudeE7;
  int32_t longitudeE7;
  int32_t gpsAltitudeMSLmm;
  int32_t environmentalAltitudeCm;

  // IMU: 26 bytes
  int16_t accelerationCentiMps2[3];
  int16_t gyroDeciDegPerSec[3];
  int16_t magneticFieldDeciMicroTesla[3];
  int16_t orientationQ15[4];

  // Environmental measurements: 6 bytes
  int16_t temperatureCentiC;
  uint16_t pressureDeciHpa;
  uint16_t humidityCentiPercent;

  // Packet information: 5 bytes
  uint16_t packetCounter;
  uint8_t CSID;
  uint8_t cellPercentageX2;
  uint8_t sensorValidity;

  // CRC over the preceding 53 bytes
  uint16_t crc16;
};

static_assert(
    sizeof(TelemetryPacket) == 55,
    "TelemetryPacket must be exactly 55 bytes");

static_assert(
    offsetof(TelemetryPacket, sensorValidity) == 52,
    "Sensor validity byte must begin at byte 52");

static_assert(
    offsetof(TelemetryPacket, crc16) == 53,
    "CRC must begin at byte 53");

// Convert readable sensor data into the compact packet.
// allDataFresh should be true only when all required sensor measurements were
// successfully refreshed during the current packet cycle.
void encodePacket(
    const ChipSatSensors::SensorData &source,
    uint16_t packetCounter,
    uint8_t chipsatID,
    bool allDataFresh,
    TelemetryPacket &destination);

// Calculate the packet CRC. CRC-16-CCITT-FALSE: polynomial 0x1021, initial value 0xFFFF.
//
// Defined here rather than in the .cpp, and constexpr, because the settings block needs its own CRC
// computed while the image is being compiled - a block whose CRC were filled in afterwards would be
// wrong on any board flashed from the ELF. Three things use this one function now: the packet, the
// flash record and the settings block
constexpr uint16_t calculateCRC16(
    const uint8_t *data,
    size_t length) {
  uint16_t crc = 0xFFFF;

  for (size_t byteIndex = 0;
       byteIndex < length;
       byteIndex++) {
    crc ^= static_cast<uint16_t>(data[byteIndex]) << 8;

    for (uint8_t bit = 0; bit < 8; bit++) {
      if (crc & 0x8000) {
        crc = static_cast<uint16_t>(
          (crc << 1) ^ 0x1021);
      } else {
        crc = static_cast<uint16_t>(crc << 1);
      }
    }
  }

  return crc;
}

// Verify a received packet.
bool packetCRCIsValid(
    const TelemetryPacket &packet);

} // namespace ChipSatTelemetry
