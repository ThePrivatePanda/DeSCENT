#pragma once

#include <stdint.h>
#include "../devices/SensorData.h"

namespace ChipSatApp
{

// Clears the valid flag of any IMU report older than staleMs. Returns which ones it cleared,
// bit 0 linaccel, 1 gyro, 2 mag, 3 quat
uint8_t clearStaleImuReports(ChipSatSensors::IMUData &imu, uint32_t nowMs, uint32_t staleMs);

// Reports whose count didn't change between two snapshots, same bit order
uint8_t silentImuReports(const uint32_t *countsNow, const uint32_t *countsBefore);

} // namespace ChipSatApp
