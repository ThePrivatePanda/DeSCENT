#pragma once
// Fixed by the board, a datasheet or a protocol. Named so the code reads, but changing one means
// the hardware or the message format changed, not that you tuned something.
// Settings you would realistically change live in config.h

#include <Arduino.h>
#include "config.h"

namespace ChipSatConstants
{

// Pins, set by the V2.5.x board layout
constexpr uint32_t kConsoleRxPin = PB7;
constexpr uint32_t kConsoleTxPin = PB6;
constexpr uint32_t kLedPin = PA9;
constexpr uint32_t kI2cSdaPin = PA15;
constexpr uint32_t kI2cSclPin = PB15;
constexpr uint32_t kGpsRxPin = PC1;   // (rx, tx) is right, the core swaps the LPUART pins itself
constexpr uint32_t kGpsTxPin = PC0;

// Wire timeouts in a row before the bus gets started again
constexpr uint8_t kI2cStuckTransfers = 3;

// I2C addresses, set by the parts themselves
constexpr uint8_t kImuAddress = 0x4A;
constexpr uint8_t kEnvAddress = 0x77;
constexpr uint8_t kGaugeAddress = 0x36;

// MAX17048 registers. All 16 bit, big endian
constexpr uint8_t kGaugeRegVcell = 0x02;   // cell voltage, 78.125 uV per count
constexpr uint8_t kGaugeRegSoc = 0x04;
constexpr uint8_t kGaugeRegVersion = 0x08;
constexpr uint8_t kGaugeRegStatus = 0x1A;
constexpr uint16_t kGaugeVersionMask = 0xFFF0;
constexpr uint16_t kGaugeVersionValue = 0x0010;
constexpr uint8_t kGaugeStatusResetIndicator = 0x01;   // high byte, set once after a real power-up

// BME280 registers
constexpr uint8_t kEnvRegCalib1 = 0x88;     // 26 bytes, temperature and pressure constants plus H1
constexpr uint8_t kEnvRegCalib2 = 0xE1;     // 7 bytes, the humidity constants
constexpr uint8_t kEnvRegChipId = 0xD0;
constexpr uint8_t kEnvRegReset = 0xE0;
constexpr uint8_t kEnvRegStatus = 0xF3;
constexpr uint8_t kEnvRegCtrlHum = 0xF2;
constexpr uint8_t kEnvRegCtrlMeas = 0xF4;
constexpr uint8_t kEnvRegConfig = 0xF5;
constexpr uint8_t kEnvRegData = 0xF7;       // 8 bytes: pressure, temperature, humidity
constexpr uint8_t kEnvCalib1Length = 26;
constexpr uint8_t kEnvCalib2Length = 7;
constexpr uint8_t kEnvDataLength = 8;
constexpr uint8_t kEnvChipIdValue = 0x60;
constexpr uint8_t kEnvResetCommand = 0xB6;
constexpr uint8_t kEnvStatusImUpdate = 0x01;   // bit 0, copying calibration out of its own memory
constexpr uint8_t kEnvStatusMeasuring = 0x08;  // bit 3, a conversion is running
constexpr uint8_t kEnvModeSleep = 0x00;
constexpr uint8_t kEnvModeForced = 0x01;
constexpr uint8_t kEnvFilterOff = 0x00;

// The BME280's operating range (datasheet rev 1.23, key parameters). A reading outside it is a bad
// part or bad calibration, not weather
constexpr float kEnvMinPressurePa = 30000.0f;
constexpr float kEnvMaxPressurePa = 110000.0f;
constexpr float kEnvMinTemperatureC = -40.0f;
constexpr float kEnvMaxTemperatureC = 85.0f;
constexpr float kEnvMinHumidityPercent = 0.0f;
constexpr float kEnvMaxHumidityPercent = 100.0f;

// BNO085 SHTP. Every packet starts with length (2 bytes, bit 15 = continuation), channel, sequence.
// Channel numbers are fixed on the BNO085 (datasheet 1000-3927 v1.17 section 1.3.1)
constexpr uint8_t  kShtpHeaderLength = 4;
constexpr uint16_t kShtpContinuation = 0x8000;
constexpr uint8_t  kShtpChannelCommand = 0;     // the advertisement comes on this one
constexpr uint8_t  kShtpChannelExecutable = 1;
constexpr uint8_t  kShtpChannelControl = 2;
constexpr uint8_t  kShtpChannelReports = 3;
constexpr uint8_t  kShtpChunkLength = 32;       // Wire's own buffer size, so it never reallocs
constexpr uint16_t kShtpPacketMax = 384;        // SparkFun's receive buffer, the advertisement fits

// Executable channel, one byte each way (Figure 1-27)
constexpr uint8_t kImuCommandReset = 1;
constexpr uint8_t kImuCommandOn = 2;
constexpr uint8_t kImuCommandSleep = 3;
constexpr uint8_t kImuResetComplete = 1;

// Own driver, background restart
constexpr uint8_t kImuRestartReads = 8;           // packets read per restart step, while there are any
constexpr uint8_t kImuLostAfterEmptyWaits = 3;    // fresh waits in a row with no report at all, then restarted

// SH-2 report ids and lengths on the control and report channels
constexpr uint8_t kSh2SetFeature = 0xFD;
constexpr uint8_t kSh2SetFeatureLength = 17;
constexpr uint8_t kSh2GetFeatureResponse = 0xFC;
constexpr uint8_t kSh2GetFeatureResponseLength = 17;
constexpr uint8_t kSh2ProductIdRequest = 0xF9;
constexpr uint8_t kSh2ProductIdResponse = 0xF8;
constexpr uint8_t kSh2ProductIdResponseLength = 16;
constexpr uint8_t kSh2CommandResponse = 0xF1;
constexpr uint8_t kSh2CommandResponseLength = 16;
constexpr uint8_t kSh2UnsolicitedInit = 0x84;   // command byte of the SH-2 init message after a reset
constexpr uint8_t kSh2InitSystem = 1;           // its r[1], sh2.c:433-434
constexpr uint8_t kShtpTagReportLengths = 0x81; // in the advertisement, report id then length
constexpr uint8_t kSh2Timebase = 0xFB;
constexpr uint8_t kSh2TimestampRebase = 0xFA;
constexpr uint8_t kSh2TimeReportLength = 5;
constexpr uint8_t kSh2LinearAcceleration = 0x04;
constexpr uint8_t kSh2Gyroscope = 0x02;         // calibrated
constexpr uint8_t kSh2Magnetometer = 0x03;      // calibrated
constexpr uint8_t kSh2RotationVector = 0x05;
constexpr uint8_t kSh2ReportLength = 10;        // id, sequence, status, delay, x y z
constexpr uint8_t kSh2RotationVectorLength = 14;   // plus real and accuracy

// Fixed point: value / 2^Q (sh2_SensorValue.c)
constexpr uint8_t kSh2LinearAccelerationQ = 8;  // m/s^2
constexpr uint8_t kSh2GyroscopeQ = 9;           // rad/s
constexpr uint8_t kSh2MagnetometerQ = 4;        // uT
constexpr uint8_t kSh2RotationVectorQ = 14;

// UBX framing: B5 62, class, id, length (little endian), payload, two checksum bytes
constexpr uint8_t kUbxSync1 = 0xB5;
constexpr uint8_t kUbxSync2 = 0x62;
constexpr uint8_t kUbxClassNav = 0x01;
constexpr uint8_t kUbxIdNavPvt = 0x07;
constexpr uint8_t kUbxClassAck = 0x05;
constexpr uint8_t kUbxIdAck = 0x01;
constexpr uint8_t kUbxIdNak = 0x00;
constexpr uint8_t kUbxClassCfg = 0x06;
constexpr uint8_t kUbxIdValset = 0x8A;
constexpr uint8_t kUbxIdReset = 0x04;

// The configuration keys we write, from the M10 interface description
constexpr uint32_t kUbxCfgRateMeas = 0x30210001;
constexpr uint32_t kUbxCfgRateNav = 0x30210002;
constexpr uint32_t kUbxCfgPmOperateMode = 0x20D00001;
constexpr uint32_t kUbxCfgNavspgDynmodel = 0x20110021;
constexpr uint32_t kUbxCfgUart1OutprotNmea = 0x10740002;
constexpr uint32_t kUbxCfgMsgoutNavPvtUart1 = 0x20910007;   // NAV-PVT every n solutions, 0 off
constexpr uint8_t kUbxConfigLayers = 0x03;   // RAM + battery backed, so a receiver reset keeps them
constexpr uint8_t kGpsOperateModeFull = 0;   // CFG-PM-OPERATEMODE, full power
constexpr uint32_t kGpsBaud = 9600;   // the receiver's factory setting, we never change it

// NAV-PVT payload is 92 bytes. The fields we read, by offset
constexpr uint16_t kNavPvtLength = 92;
constexpr uint8_t kNavPvtYear = 4;      // UTC, then month, day, hour, minute, second
constexpr uint8_t kNavPvtMonth = 6;
constexpr uint8_t kNavPvtDay = 7;
constexpr uint8_t kNavPvtHour = 8;
constexpr uint8_t kNavPvtMinute = 9;
constexpr uint8_t kNavPvtSecond = 10;
constexpr uint8_t kNavPvtValid = 11;    // bit 0 date, bit 1 time, bit 2 fully resolved
constexpr uint8_t kNavPvtValidDate = 0x01;
constexpr uint8_t kNavPvtValidTime = 0x02;
constexpr uint8_t kNavPvtFullyResolved = 0x04;
constexpr uint8_t kNavPvtTimeAcc = 12;  // uint32, the receiver's estimate of its time error, ns
constexpr uint8_t kNavPvtNano = 16;     // int32, the fraction of the second, ns
constexpr uint8_t kNavPvtFixType = 20;
constexpr uint8_t kNavPvtFlags = 21;
constexpr uint8_t kNavPvtNumSatellites = 23;
constexpr uint8_t kNavPvtLon = 24;
constexpr uint8_t kNavPvtLat = 28;
constexpr uint8_t kNavPvtHeightMsl = 36;
constexpr uint8_t kNavPvtFlags3 = 78;
constexpr uint16_t kNavPvtInvalidLlh = 0x0001;   // bit 0 of flags3
constexpr uint8_t kNavPvtGnssFixOk = 0x01;   // bit 0 of flags, the fix is within the DOP and accuracy masks
constexpr uint8_t kNavPvtFix3d = 3;
constexpr uint8_t kNavPvtFixGnssDeadReckoning = 4;   // 5 is time only, its position means nothing

// Clearing the receiver's stored position: bit 4 of the battery-backed mask. 0xFFFF would be a cold
// start and would cost minutes of acquisition. Reset mode 0x02 restarts navigation, not the chip
constexpr uint16_t kUbxBbrMaskPositionOnly = 0x0010;
constexpr uint8_t kUbxResetModeGnssOnly = 0x02;

// What the position fields can physically hold. Outside these is corruption, not a poor fix
constexpr int32_t kMaxLatitudeE7 = 900000000;
constexpr int32_t kMaxLongitudeE7 = 1800000000;
constexpr int32_t kMinAltitudeMm = -500000;     // -500 m, lower than any land on earth
constexpr int32_t kMaxAltitudeMm = 120000000;   // 120 km, higher than the mission goes

// Radio settings fixed by the module rather than chosen: its TCXO runs at 1.7 V (Seeed
// radio_driver.c TCXO_CTRL_1_7V) and it has an SMPS, so the DC-DC is used rather than the LDO
constexpr float kRadioTcxoVoltage = 1.7f;
constexpr bool kRadioUseLdo = false;

// Our own radio driver only, the library path doesn't use these. DS = DS_SX1261-2 Rev 2.2
constexpr uint32_t kRadioSwitchPin1 = PA4;   // RF switch, both low when idle, PA5 high to transmit
constexpr uint32_t kRadioSwitchPin2 = PA5;
constexpr uint8_t  kRadioTcxoVoltageCode = 0x01;   // 1.7 V, DS Table 13-35
constexpr uint32_t kRadioTcxoDelaySteps = 320;     // 5 ms in 15.625 us steps, RadioLib's default

constexpr uint16_t kRadioRegVersion = 0x0320;   // "SX1261..." string, not in the DS, RadioLib reads it
constexpr uint16_t kRadioRegIqPolarity = 0x0736;
constexpr uint16_t kRadioRegSyncWord = 0x0740;   // 2 bytes
constexpr uint16_t kRadioRegTxModulation = 0x0889;
constexpr uint16_t kRadioRegTxClamp = 0x08D8;
constexpr uint16_t kRadioRegOcp = 0x08E7;
constexpr uint8_t  kRadioVersionLength = 16;

constexpr uint8_t  kRadioStandbyRc = 0x00;
constexpr uint8_t  kRadioStandbyTries = 5;       // after a reset, begin gives up after this many
constexpr uint32_t kRadioStandbyRetryMs = 10;   // RadioLib waits 10 ms between its tries
constexpr uint8_t  kRadioFallbackStandbyRc = 0x20;
constexpr uint8_t  kRadioPacketTypeLora = 0x01;
constexpr uint8_t  kRadioRegulatorDcDc = 0x01;
constexpr uint8_t  kRadioCalibrateAll = 0x7F;
constexpr uint8_t  kRadioRamp200Us = 0x04;
constexpr uint8_t  kRadioHeaderExplicit = 0x00;
constexpr uint8_t  kRadioHeaderImplicit = 0x01;   // SF5 and SF6: the T-Beam's SX127x has no header there
constexpr uint8_t  kRadioCrcOn = 0x01;
constexpr uint8_t  kRadioIqStandard = 0x00;
constexpr uint8_t  kRadioSyncControlBits = 0x44;   // how RadioLib 7.1.2 turns 0x12 into 0x1424, SX126x.cpp:934
constexpr uint16_t kRadioIrqTxDone = 0x0001;
constexpr uint16_t kRadioIrqTimeout = 0x0200;
constexpr uint16_t kRadioIrqAll = 0x43FF;
constexpr uint8_t  kRadioStatusCommandMask = 0x0E;   // bits 3:1, DS Table 13-76
constexpr uint8_t  kRadioStatusCommandTimeout = 0x06;
constexpr uint8_t  kRadioStatusCommandError = 0x08;
constexpr uint8_t  kRadioStatusCommandFailed = 0x0A;

// Power amplifier settings, from DS_SX1261-2 Rev 2.2 Table 13-21 "PA Operating Modes with Optimal
// Settings" p.82. The module says which amplifier the silicon brings out; kRadioTargetDbm picks the
// row for it. The row is what sets the output: the number in SetTxParams is part of the row and is
// +22 for every SX1262 row, so reading it as the output power is the mistake to avoid.
// Neither RadioLib nor ST's own driver use these rows - both write the maximum row whatever power is
// asked for and trim with SetTxParams, which costs current (Table 3-6 p.18: the +14 dBm row draws
// 45 mA, the +22 row trimmed to +14 draws 90 mA)
struct PaSettings
{
  int8_t txParamsDbm;
  uint8_t dutyCycle;
  uint8_t hpMax;
  uint8_t deviceSel;
  uint8_t lut;
};

// SX1261, the low power amplifier. deviceSel 0x01, hpMax unused
constexpr PaSettings paSettingsLowPower(int8_t dbm)
{
  return dbm == 15 ? PaSettings{14, 0x06, 0x00, 0x01, 0x01}
       : dbm == 14 ? PaSettings{14, 0x04, 0x00, 0x01, 0x01}
                   : PaSettings{13, 0x01, 0x00, 0x01, 0x01};   // +10 dBm
}

// SX1262, the high power amplifier. deviceSel 0x00, and paDutyCycle must stay at or under 0x04 or
// the PA can be damaged (same page, Caution)
constexpr PaSettings paSettingsHighPower(int8_t dbm)
{
  return dbm == 22 ? PaSettings{22, 0x04, 0x07, 0x00, 0x01}
       : dbm == 20 ? PaSettings{22, 0x03, 0x05, 0x00, 0x01}
       : dbm == 17 ? PaSettings{22, 0x02, 0x03, 0x00, 0x01}
                   : PaSettings{22, 0x02, 0x02, 0x00, 0x01};   // +14 dBm
}

#if CHIPSAT_RADIO_MODULE == CHIPSAT_RADIO_MODULE_LE
constexpr const char *kRadioModuleName = "le";
constexpr bool     kImuSleepsDuringTx = false;
constexpr PaSettings kPa = paSettingsLowPower(ChipSatConfig::kRadioTargetDbm);
static_assert(ChipSatConfig::kRadioTargetDbm == 10 || ChipSatConfig::kRadioTargetDbm == 14 ||
              ChipSatConfig::kRadioTargetDbm == 15,
              "an LE module has rows for 10, 14 and 15 dBm only");
#elif CHIPSAT_RADIO_MODULE == CHIPSAT_RADIO_MODULE_HP
constexpr const char *kRadioModuleName = "hp";
constexpr bool     kImuSleepsDuringTx = true;
constexpr PaSettings kPa = paSettingsHighPower(ChipSatConfig::kRadioTargetDbm);
static_assert(ChipSatConfig::kRadioTargetDbm == 14 || ChipSatConfig::kRadioTargetDbm == 17 ||
              ChipSatConfig::kRadioTargetDbm == 20 || ChipSatConfig::kRadioTargetDbm == 22,
              "an HP module has rows for 14, 17, 20 and 22 dBm only");
#else
#error "CHIPSAT_RADIO_MODULE has to be CHIPSAT_RADIO_MODULE_LE or CHIPSAT_RADIO_MODULE_HP"
#endif

constexpr int8_t   kPaPowerDbm = kPa.txParamsDbm;
constexpr int8_t   kRadioPowerDbm = kPa.txParamsDbm;   // the same, so the two writes can't disagree
constexpr uint8_t  kPaDutyCycle = kPa.dutyCycle;
constexpr uint8_t  kPaHpMax = kPa.hpMax;
constexpr uint8_t  kPaDeviceSel = kPa.deviceSel;
constexpr uint8_t  kPaLut = kPa.lut;
static_assert(kPaDeviceSel == 0x01 || kPaDutyCycle <= 0x04, "SX1262 paDutyCycle over 0x04 damages the PA");

// W25Q16JV flash on SPI2. Page numbers are the printed ones in Winbond's datasheet Rev I
constexpr uint32_t kFlashMosiPin = PA10;
constexpr uint32_t kFlashMisoPin = PB14;
constexpr uint32_t kFlashSckPin = PB13;
constexpr uint32_t kFlashCsPin = PB9;   // plain gpio, R18 pulls it high while the MCU is in reset

constexpr uint8_t kFlashCmdWriteEnable = 0x06;
constexpr uint8_t kFlashCmdReadStatus1 = 0x05;
constexpr uint8_t kFlashCmdRead = 0x03;
constexpr uint8_t kFlashCmdPageProgram = 0x02;
constexpr uint8_t kFlashCmdSectorErase = 0x20;
constexpr uint8_t kFlashCmdChipErase = 0xC7;
constexpr uint8_t kFlashCmdJedecId = 0x9F;
constexpr uint8_t kFlashCmdPowerDown = 0xB9;
constexpr uint8_t kFlashCmdReleasePowerDown = 0xAB;
constexpr uint8_t kFlashStatusBusy = 0x01;
constexpr uint8_t kFlashStatusWel = 0x02;

// EF 40 15 is the IQ part on the schematic. The IM part (EF 70 15) has a working /HOLD, and /HOLD
// floats on our board (p.19, p.72)
constexpr uint8_t kFlashManufacturerId = 0xEF;
constexpr uint8_t kFlashMemoryType = 0x40;
constexpr uint8_t kFlashCapacity = 0x15;

constexpr uint32_t kFlashSize = 2097152;
constexpr uint32_t kFlashPageSize = 256;
constexpr uint32_t kFlashSectorSize = 4096;

// Datasheet max is 3 ms, 400 ms and 25 s (p.63). The bench measured 0.35 ms and 36 ms
constexpr uint32_t kFlashProgramTimeoutMs = 10;
constexpr uint32_t kFlashEraseTimeoutMs = 1000;
constexpr uint32_t kFlashChipEraseTimeoutMs = 30000;
constexpr uint32_t kFlashSpiTimeoutMs = 50;   // per HAL transfer, a 4 kB read is about 11 ms at 3 MHz
constexpr uint32_t kFlashWakeMs = 2;   // tRES1 is 3 us, but waitMs(1) can return in a few us if the tick is about to change

} // namespace ChipSatConstants
