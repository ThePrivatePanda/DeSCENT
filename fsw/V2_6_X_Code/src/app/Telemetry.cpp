#include "Telemetry.h"
#include <Arduino.h>

#include <math.h>
#include <limits.h>

namespace ChipSatTelemetry {

// ---------------------------------------------------------------------------
// Safe scaling helpers
// ---------------------------------------------------------------------------

static int16_t scaleToInt16(
  float value,
  float scaleFactor) {
  if (!isfinite(value)) {
    return 0;
  }

  float scaled = roundf(value * scaleFactor);

  if (scaled > INT16_MAX) {
    return INT16_MAX;
  }

  if (scaled < INT16_MIN) {
    return INT16_MIN;
  }

  return static_cast<int16_t>(scaled);
}

static uint16_t scaleToUint16(
  float value,
  float scaleFactor) {
  if (!isfinite(value) || value <= 0.0f) {
    return 0;
  }

  float scaled = roundf(value * scaleFactor);

  if (scaled > UINT16_MAX) {
    return UINT16_MAX;
  }

  return static_cast<uint16_t>(scaled);
}

static uint8_t scaleToUint8(
  float value,
  float scaleFactor) {
  if (!isfinite(value) || value <= 0.0f) {
    return 0;
  }

  float scaled = roundf(value * scaleFactor);

  if (scaled > UINT8_MAX) {
    return UINT8_MAX;
  }

  return static_cast<uint8_t>(scaled);
}

static int32_t scaleToInt32(
  float value,
  float scaleFactor) {
  if (!isfinite(value)) {
    return 0;
  }

  double scaled = round(
    static_cast<double>(value) * static_cast<double>(scaleFactor));

  if (scaled > INT32_MAX) {
    return INT32_MAX;
  }

  if (scaled < INT32_MIN) {
    return INT32_MIN;
  }

  return static_cast<int32_t>(scaled);
}

// ---------------------------------------------------------------------------
// Encode readable sensor data into the compact packet
// ---------------------------------------------------------------------------

void encodePacket(
  const ChipSatSensors::SensorData &source,
  uint16_t packetCounter,
  uint8_t chipsatID,
  bool allDataFresh,
  TelemetryPacket &packet) {
  // Clear the entire packet first.
  packet = {};

  // GPS values are already supplied by the u-blox library in compact units.
  packet.latitudeE7 =
    source.gps.latitudeE7;

  packet.longitudeE7 =
    source.gps.longitudeE7;

  packet.gpsAltitudeMSLmm =
    source.gps.altitudeMSLmm;

  // Environmental altitude:
  // meters -> centimeters
  packet.environmentalAltitudeCm =
    scaleToInt32(
      source.environmental.altitudeM,
      100.0f);

  // Linear acceleration:
  // m/s^2 -> 0.01 m/s^2
  packet.accelerationCentiMps2[0] =
    scaleToInt16(
      source.imu.linearAccelerationMps2.x,
      100.0f);

  packet.accelerationCentiMps2[1] =
    scaleToInt16(
      source.imu.linearAccelerationMps2.y,
      100.0f);

  packet.accelerationCentiMps2[2] =
    scaleToInt16(
      source.imu.linearAccelerationMps2.z,
      100.0f);

  // Gyroscope:
  // rad/s -> degrees/s -> 0.1 degrees/s
  constexpr float GYRO_SCALE =
    RAD_TO_DEG * 10.0f;

  packet.gyroDeciDegPerSec[0] =
    scaleToInt16(
      source.imu.gyroscopeRadPerSec.x,
      GYRO_SCALE);

  packet.gyroDeciDegPerSec[1] =
    scaleToInt16(
      source.imu.gyroscopeRadPerSec.y,
      GYRO_SCALE);

  packet.gyroDeciDegPerSec[2] =
    scaleToInt16(
      source.imu.gyroscopeRadPerSec.z,
      GYRO_SCALE);

  // Magnetometer:
  // microtesla -> 0.1 microtesla
  packet.magneticFieldDeciMicroTesla[0] =
    scaleToInt16(
      source.imu.magnetometerMicroTesla.x,
      10.0f);

  packet.magneticFieldDeciMicroTesla[1] =
    scaleToInt16(
      source.imu.magnetometerMicroTesla.y,
      10.0f);

  packet.magneticFieldDeciMicroTesla[2] =
    scaleToInt16(
      source.imu.magnetometerMicroTesla.z,
      10.0f);

  // Quaternion:
  // -1.0 through +1.0 -> approximately -32767 through +32767
  packet.orientationQ15[0] =
    scaleToInt16(
      constrain(source.imu.orientation.i,
                -1.0f,
                1.0f),
      32767.0f);

  packet.orientationQ15[1] =
    scaleToInt16(
      constrain(source.imu.orientation.j,
                -1.0f,
                1.0f),
      32767.0f);

  packet.orientationQ15[2] =
    scaleToInt16(
      constrain(source.imu.orientation.k,
                -1.0f,
                1.0f),
      32767.0f);

  packet.orientationQ15[3] =
    scaleToInt16(
      constrain(source.imu.orientation.real,
                -1.0f,
                1.0f),
      32767.0f);

  // Temperature:
  // degrees C -> 0.01 degrees C
  packet.temperatureCentiC =
    scaleToInt16(
      source.environmental.temperatureC,
      100.0f);

  // Pressure:
  // SensorData is in Pa.
  // Divide by 10 to produce units of 0.1 hPa.
  packet.pressureDeciHpa =
    scaleToUint16(
      source.environmental.pressurePa,
      0.1f);

  // Humidity:
  // percent -> 0.01 percent
  packet.humidityCentiPercent =
    scaleToUint16(
      constrain(
        source.environmental.humidityPercent,
        0.0f,
        100.0f),
      100.0f);

  // State of charge:
  // percent -> 0.5 percent increments
  packet.cellPercentageX2 =
    scaleToUint8(
      constrain(
        source.stateOfCharge.cellPercentage,
        0.0f,
        100.0f),
      2.0f);

  packet.packetCounter = packetCounter;
  packet.CSID = chipsatID;

  packet.sensorValidity = 0;

  if (source.imu.linearAccelerationValid) { packet.sensorValidity |= kLinearAccelerationValidBit; }
  if (source.imu.gyroscopeValid) { packet.sensorValidity |= kGyroscopeValidBit; }
  if (source.imu.magnetometerValid) { packet.sensorValidity |= kMagnetometerValidBit; }
  if (source.imu.orientationValid) { packet.sensorValidity |= kQuaternionValidBit; }
  if (source.gps.valid) { packet.sensorValidity |= kGPSValidBit; }
  if (source.stateOfCharge.valid) { packet.sensorValidity |= kStateOfChargeValidBit; }
  if (source.environmental.valid) { packet.sensorValidity |= kEnvironmentalValidBit; }

  if (allDataFresh &&
      (packet.sensorValidity & kAllSensorDataValidMask) == kAllSensorDataValidMask) {
    packet.sensorValidity |= kAllDataFreshBit;
  }

  // Calculate this last. It covers bytes 0 through 52.
  packet.crc16 = calculateCRC16(
    reinterpret_cast<const uint8_t *>(&packet),
    offsetof(TelemetryPacket, crc16));
}

bool packetCRCIsValid(
  const TelemetryPacket &packet) {
  uint16_t calculatedCRC = calculateCRC16(
    reinterpret_cast<const uint8_t *>(&packet),
    offsetof(TelemetryPacket, crc16));

  return calculatedCRC == packet.crc16;
}


}  // namespace ChipSatTelemetry
