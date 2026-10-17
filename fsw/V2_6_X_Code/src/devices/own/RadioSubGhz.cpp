#include <Arduino.h>
#include <math.h>
#include <string.h>
#include "../../Settings.h"
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_RADIO_DRIVER == CHIPSAT_DRIVER_OWN

#include "../Radio.h"
#include "../../log/Log.h"
#include "RadioSubGhzBytes.h"

namespace ChipSatDevices
{

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;
using namespace ChipSatRadioBytes;

// Through ST's HAL SUBGHZ driver, which bounds every wait itself at about 100 ms (not measured yet)

constexpr uint8_t kBandwidth = bandwidthCode(kRadioBandwidthKhz);

constexpr uint32_t kFrequency = frequencyWord(kRadioFrequencyMhz);
constexpr uint16_t kImageCalibration = imageCalibration(kRadioFrequencyMhz);
constexpr bool kBandwidth500 = kBandwidth == 0x06;

// The spreading factor, the power and which module is fitted now come from the settings block, so
// they are not known while compiling. Settings::begin() has already refused anything this firmware
// cannot transmit with - a spreading factor outside 5 to 12, or a power the module has no row for -
// so by the time any of this runs the values are ones the tables below have an answer for
static const ChipSatSettings::Settings& radioSettings()
{
  return ChipSatSettings::active();
}

static PaSettings activePa()
{
  const ChipSatSettings::Settings& s = radioSettings();
  return s.module == ChipSatSettings::kModuleHp ? paSettingsHighPower(s.targetDbm)
                                                : paSettingsLowPower(s.targetDbm);
}

static bool highPowerPa()
{
  return activePa().deviceSel == 0x00;
}

static_assert(kBandwidth != 0xFF, "kRadioBandwidthKhz isn't a LoRa bandwidth the radio has");
static_assert(kImageCalibration != 0, "kRadioFrequencyMhz is outside the image calibration bands");
static_assert(kRadioCodingRate >= 5 && kRadioCodingRate <= 8, "coding rate 5 to 8, for 4/5 to 4/8");
static_assert(kRadioTcxoVoltage == 1.7f, "kRadioTcxoVoltageCode is for 1.7 V");

static SUBGHZ_HandleTypeDef subghz;
static bool ready = false;
static uint8_t packetTypeSet = 0xFF;   // what begin set, for the reset check in startTransmit
static int16_t lastError = 0;   // HAL ErrorCode, the radio's status byte, or -1 for our own checks
static bool errorKept = false;  // lastError is the first failure of the current call
static bool txModulationWarned = false;

// Every step below does nothing once status isn't Ok, so a sequence stops at its first failure
// and the log says which step it was
static void startCall()
{
  lastError = 0;
  errorKept = false;
}

static void fail(Status &status, Status why, const char *step, int16_t code)
{
  status = why;
  if (!errorKept) {
    lastError = code;
    errorKept = true;
  }
  LOG_W(Radio, "step") {
    line.field("step", step);
    line.field("ok", false);
    line.field("code", code);
  }
}

static void checkHal(Status &status, HAL_StatusTypeDef result, const char *step)
{
  if (result == HAL_OK) {
    return;
  }
  // HAL_BUSY means the handle wasn't ready, not a radio timeout
  if (result == HAL_BUSY) {
    fail(status, Status::NotReady, step, -2);
  } else {
    fail(status, Status::Timeout, step, static_cast<int16_t>(subghz.ErrorCode));
  }
}

static void command(Status &status, const char *step, SUBGHZ_RadioSetCmd_t opcode, const uint8_t *params,
                    uint16_t length)
{
  if (status != Status::Ok) {
    return;
  }
  checkHal(status, HAL_SUBGHZ_ExecSetCmd(&subghz, opcode, const_cast<uint8_t *>(params), length), step);
}

static void query(Status &status, const char *step, SUBGHZ_RadioGetCmd_t opcode, uint8_t *reply, uint16_t length)
{
  if (status != Status::Ok) {
    return;
  }
  checkHal(status, HAL_SUBGHZ_ExecGetCmd(&subghz, opcode, reply, length), step);
}

static void writeRegisters(Status &status, const char *step, uint16_t address, const uint8_t *data, uint16_t length)
{
  if (status != Status::Ok) {
    return;
  }
  checkHal(status, HAL_SUBGHZ_WriteRegisters(&subghz, address, const_cast<uint8_t *>(data), length), step);
}

static void readRegisters(Status &status, const char *step, uint16_t address, uint8_t *data, uint16_t length)
{
  if (status != Status::Ok) {
    return;
  }
  checkHal(status, HAL_SUBGHZ_ReadRegisters(&subghz, address, data, length), step);
}

// Read, change some bits, write back, the way RadioLib does its register fixes
static void updateRegister(Status &status, const char *step, uint16_t address, uint8_t clear, uint8_t set)
{
  uint8_t value = 0;
  readRegisters(status, step, address, &value, 1);
  value = static_cast<uint8_t>((value & ~clear) | set);
  writeRegisters(status, step, address, &value, 1);
}

// The HAL throws away the status byte of every command, so ask for it separately. ST's own
// radio_driver.c does the same (SUBGRF_GetStatus, "HAL limitations")
static void checkRadioStatus(Status &status, const char *step)
{
  uint8_t radioStatus = 0;
  query(status, step, RADIO_GET_STATUS, &radioStatus, 1);
  if (status != Status::Ok) {
    return;
  }
  // 0xFF is a radio that isn't answering. 0x00 is let through: the HAL hands us the third byte on
  // the wire, and whether the chip repeats its status there isn't checked on our board yet
  const uint8_t command = radioStatus & kRadioStatusCommandMask;
  if (radioStatus == 0xFF || command == kRadioStatusCommandTimeout ||
      command == kRadioStatusCommandError || command == kRadioStatusCommandFailed) {
    fail(status, Status::Failed, step, radioStatus);
  }
}

static void rfSwitch(bool transmit)
{
  digitalWrite(kRadioSwitchPin1, LOW);
  digitalWrite(kRadioSwitchPin2, transmit ? HIGH : LOW);
}

// The radio's IRQ line only reaches us as the NVIC pending bit, the interrupt itself stays off
static void clearIrqLine()
{
  HAL_NVIC_ClearPendingIRQ(SUBGHZ_Radio_IRQn);
}

Radio::Radio(ChipSatPlatform::System &system) : system_(system)
{
}

Status Radio::begin()
{
  startCall();
  ready = false;
  pinMode(kRadioSwitchPin1, OUTPUT);
  pinMode(kRadioSwitchPin2, OUTPUT);
  rfSwitch(false);

  // DeInit puts the radio in reset and Init takes it out, each waiting (bounded) for the MCU to
  // confirm. Init only does that from the reset state, which DeInit leaves it in, so every begin
  // gets the same full reset. It also clears the HAL's sticky error
  Status status = Status::Ok;
  __HAL_RCC_SUBGHZSPI_CLK_ENABLE();
  checkHal(status, HAL_SUBGHZ_DeInit(&subghz), "reset");
  system_.waitMs(1);   // DS 8.1 wants about 100 us, RadioLib holds it 1 ms
  subghz.Init.BaudratePrescaler = SUBGHZSPI_BAUDRATEPRESCALER_4;   // 12 MHz, same as RadioLib, radio max is 16
  // Init even if DeInit failed, so the radio isn't left held in reset
  const HAL_StatusTypeDef init = HAL_SUBGHZ_Init(&subghz);
  if (status == Status::Ok) {
    checkHal(status, init, "init");
  }

  // RadioLib: "SX126x often refuses first few commands after reset", so it retries standby for 1 s
  // (SX126x.cpp:174). The HAL keeps a failure until Init, so it's cleared before each retry
  const uint8_t standby = kRadioStandbyRc;
  if (status == Status::Ok) {
    for (uint8_t attempt = 1; attempt <= kRadioStandbyTries; ++attempt) {
      status = Status::Ok;
      subghz.ErrorCode = HAL_SUBGHZ_ERROR_NONE;
      command(status, "standby", RADIO_SET_STANDBY, &standby, 1);
      checkRadioStatus(status, "standby");
      if (status == Status::Ok) {
        startCall();   // the refused tries aren't this call's error
        break;
      }
      if (attempt < kRadioStandbyTries) {
        system_.waitMs(kRadioStandbyRetryMs);
      }
    }
  }

  // Every SX126x answers "SX1261", SX1262s too (RadioLib 7.1.2 SX1262.h:15)
  uint8_t version[kRadioVersionLength] = {0};
  readRegisters(status, "version", kRadioRegVersion, version, sizeof version);
  if (status == Status::Ok && memcmp(version, "SX1261", 6) != 0) {
    fail(status, Status::BadData, "version", -1);
  }

  // After a reset the radio doesn't know it has a TCXO yet, so it has flagged the oscillator
  const uint8_t noErrors[2] = {0x00, 0x00};
  command(status, "errors", RADIO_CLR_ERROR, noErrors, sizeof noErrors);

  const uint8_t tcxo[4] = {kRadioTcxoVoltageCode, static_cast<uint8_t>(kRadioTcxoDelaySteps >> 16),
                           static_cast<uint8_t>(kRadioTcxoDelaySteps >> 8), static_cast<uint8_t>(kRadioTcxoDelaySteps)};
  command(status, "tcxo", RADIO_SET_TCXOMODE, tcxo, sizeof tcxo);

  packetTypeSet = kRadioPacketTypeLora;
  command(status, "packettype", RADIO_SET_PACKETTYPE, &packetTypeSet, 1);
  const uint8_t fallback = kRadioFallbackStandbyRc;
  command(status, "fallback", RADIO_SET_TXFALLBACKMODE, &fallback, 1);

  const uint8_t clearAll[2] = {static_cast<uint8_t>(kRadioIrqAll >> 8), static_cast<uint8_t>(kRadioIrqAll)};
  command(status, "clearirq", RADIO_CLR_IRQSTATUS, clearAll, sizeof clearAll);
  clearIrqLine();

  // TxDone and Timeout on, only TxDone to DIO1. RadioLib resends it every packet, once is enough
  const uint16_t irqMask = kRadioIrqTxDone | kRadioIrqTimeout;
  const uint8_t irq[8] = {static_cast<uint8_t>(irqMask >> 8), static_cast<uint8_t>(irqMask),
                          static_cast<uint8_t>(kRadioIrqTxDone >> 8), static_cast<uint8_t>(kRadioIrqTxDone),
                          0x00, 0x00, 0x00, 0x00};
  command(status, "irq", RADIO_CFG_DIOIRQ, irq, sizeof irq);

  // Needed once the TCXO is declared. Takes about 3.5 ms plus the TCXO start, the HAL waits it out
  const uint8_t calibrate = kRadioCalibrateAll;
  command(status, "calibrate", RADIO_CALIBRATE, &calibrate, 1);
  checkRadioStatus(status, "calibrate");

  const uint8_t regulator = kRadioRegulatorDcDc;
  command(status, "regulator", RADIO_SET_REGULATORMODE, &regulator, 1);

  const uint8_t image[2] = {static_cast<uint8_t>(kImageCalibration >> 8), static_cast<uint8_t>(kImageCalibration)};
  command(status, "image", RADIO_CALIBRATEIMAGE, image, sizeof image);

  const uint8_t frequency[4] = {static_cast<uint8_t>(kFrequency >> 24), static_cast<uint8_t>(kFrequency >> 16),
                                static_cast<uint8_t>(kFrequency >> 8), static_cast<uint8_t>(kFrequency)};
  command(status, "frequency", RADIO_SET_RFFREQUENCY, frequency, sizeof frequency);

  const uint8_t bufferBase[2] = {0x00, 0x00};
  command(status, "buffer", RADIO_SET_BUFFERBASEADDRESS, bufferBase, sizeof bufferBase);

  const uint8_t spreadingFactor = radioSettings().spreadingFactor;
  const uint8_t modulation[4] = {spreadingFactor, kBandwidth, static_cast<uint8_t>(kRadioCodingRate - 4),
                                 lowDataRateOptimize(spreadingFactor, kRadioBandwidthKhz)};
  command(status, "modulation", RADIO_SET_MODULATIONPARAMS, modulation, sizeof modulation);

  // Both are the chip's reset values already (DS Table 12-1), written anyway so begin doesn't
  // depend on the reset having worked
  const uint8_t syncWord[2] = {syncWordMsb(kRadioSyncWord, kRadioSyncControlBits),
                               syncWordLsb(kRadioSyncWord, kRadioSyncControlBits)};
  writeRegisters(status, "syncword", kRadioRegSyncWord, syncWord, sizeof syncWord);
  updateRegister(status, "iq", kRadioRegIqPolarity, 0x00, 0x04);   // bit 2 set for standard IQ, DS 15.4

  // DS 15.1, bit 2 set for any bandwidth but 500 kHz. startTransmit does it again before each packet
  updateRegister(status, "txmodulation", kRadioRegTxModulation, kBandwidth500 ? 0x04 : 0x00,
                 kBandwidth500 ? 0x00 : 0x04);

  const uint8_t dio2AsIrq = 0x00;   // on the WL DIO2 is the radio IRQ, not an RF switch
  command(status, "dio2", RADIO_SET_RFSWITCHMODE, &dio2AsIrq, 1);

  checkRadioStatus(status, "setup");

  rfSwitch(false);
  lastCode_ = status == Status::Ok ? 0 : lastError;
  ldroCode_ = 0;   // no LDRO call any more, it's a constant in the modulation bytes
  ready = status == Status::Ok;
  return status;
}

Status Radio::setCurrentLimit()
{
  startCall();
  if (!ready) {
    return Status::NotReady;
  }
  if (!(kRadioCurrentLimitMa >= 0.0f && kRadioCurrentLimitMa <= 140.0f)) {
    lastCode_ = -1;
    return Status::Failed;
  }
  Status status = Status::Ok;
  const uint8_t limit = static_cast<uint8_t>(kRadioCurrentLimitMa / 2.5f);   // 2.5 mA steps
  writeRegisters(status, "ocp", kRadioRegOcp, &limit, 1);
  lastCode_ = status == Status::Ok ? 0 : lastError;
  return status;
}

// nan if the read fails, so the log can't show a made up limit
float Radio::currentLimitMa()
{
  startCall();
  if (!ready) {
    return NAN;
  }
  Status status = Status::Ok;
  uint8_t limit = 0;
  readRegisters(status, "ocp", kRadioRegOcp, &limit, 1);
  return status == Status::Ok ? limit * 2.5f : NAN;
}

Status Radio::setOutputPower()
{
  startCall();
  if (!ready) {
    return Status::NotReady;
  }
  // RadioLib 7.1.2's ranges for each PA, STM32WLx.cpp:67 and :80. Every row in the table is already
  // inside its PA's range, so this can only fire on a table mistake - which is why it stays
  const PaSettings pa = activePa();
  const bool inRange = highPowerPa() ? (pa.txParamsDbm >= -9 && pa.txParamsDbm <= 22)
                                     : (pa.txParamsDbm >= -17 && pa.txParamsDbm <= 14);
  if (!inRange) {
    lastCode_ = -1;
    return Status::Failed;
  }
  Status status = Status::Ok;
  const uint8_t txParams[2] = {static_cast<uint8_t>(pa.txParamsDbm), kRadioRamp200Us};
  command(status, "power", RADIO_SET_TXPARAMS, txParams, sizeof txParams);
  lastCode_ = status == Status::Ok ? 0 : lastError;
  return status;
}

// SetPaConfig then SetTxParams, the order in DS 14.2. RadioLib's last two go the other way round,
// the registers end up the same (not read back on the bench yet). OCP is left as SetPaConfig resets
// it, which is what flies today: 60 mA LE, 140 mA HP. The LE value isn't confirmed on hardware
Status Radio::applyPaConfig()
{
  startCall();
  if (!ready) {
    return Status::NotReady;
  }
  Status status = Status::Ok;
  const PaSettings pa = activePa();
  const uint8_t paConfig[4] = {pa.dutyCycle, pa.hpMax, pa.deviceSel, pa.lut};   // this order on the wire
  command(status, "paconfig", RADIO_SET_PACONFIG, paConfig, sizeof paConfig);

  // DS 15.2 raises the PA clamp threshold on the HP PA. On LE, RadioLib puts bits 4:1 back to 0100
  updateRegister(status, "clamp", kRadioRegTxClamp, 0x1E, highPowerPa() ? 0x1E : 0x08);

  const uint8_t txParams[2] = {static_cast<uint8_t>(pa.txParamsDbm), kRadioRamp200Us};
  command(status, "power", RADIO_SET_TXPARAMS, txParams, sizeof txParams);
  lastCode_ = status == Status::Ok ? 0 : lastError;
  return status;
}

Status Radio::startTransmit(const uint8_t *data, size_t length)
{
  startCall();
  if (!ready) {
    return Status::NotReady;
  }
  if (length > 255) {
    lastCode_ = -1;
    return Status::BadData;
  }

  Status status = Status::Ok;
  rfSwitch(false);
  const uint8_t standby = kRadioStandbyRc;
  command(status, "standby", RADIO_SET_STANDBY, &standby, 1);

  // A radio that has reset itself since begin has lost everything we set. The packet type not
  // being the one we set is what stops the packet (the DS doesn't give the reset value)
  uint8_t packetType = 0xFF;
  query(status, "packettype", RADIO_GET_PACKETTYPE, &packetType, 1);
  if (status == Status::Ok && packetType != packetTypeSet) {
    fail(status, Status::Failed, "packettype", packetType);
  }
  // Bit 2 of 0x0889 back to 0 is a second sign (DS Table 12-1 resets it to 0x01, begin sets it), but
  // only logged, once a boot. Not seen on hardware yet whether a transmission clears it, and if it
  // does, failing on it would drop every other packet
  uint8_t txModulation = 0;
  readRegisters(status, "txmodulation", kRadioRegTxModulation, &txModulation, 1);
  if (status == Status::Ok && !kBandwidth500 && (txModulation & 0x04) == 0 && !txModulationWarned) {
    txModulationWarned = true;
    LOG_W(Radio, "txmodulation") { line.fieldHex("value", txModulation, 2); }
  }
  // DS 15.1 wants it set before every packet, RadioLib writes it each time
  txModulation = static_cast<uint8_t>(kBandwidth500 ? (txModulation & ~0x04) : (txModulation | 0x04));
  writeRegisters(status, "txmodulation", kRadioRegTxModulation, &txModulation, 1);

  // Below SF7 the receiver's SX127x cannot read a header, so none is sent and the length is
  // the fixed 55 it is told. Measured 2026-10-17: with a header at SF6 it heard every packet at
  // -75 dBm and failed every CRC
  const uint8_t packet[6] = {static_cast<uint8_t>(kRadioPreambleLength >> 8),
                             static_cast<uint8_t>(kRadioPreambleLength), headerType(),
                             static_cast<uint8_t>(length), kRadioCrcOn, kRadioIqStandard};
  command(status, "packet", RADIO_SET_PACKETPARAMS, packet, sizeof packet);

  if (status == Status::Ok) {
    checkHal(status, HAL_SUBGHZ_WriteBuffer(&subghz, 0x00, const_cast<uint8_t *>(data), length), "payload");
  }

  const uint8_t clearAll[2] = {static_cast<uint8_t>(kRadioIrqAll >> 8), static_cast<uint8_t>(kRadioIrqAll)};
  command(status, "clearirq", RADIO_CLR_IRQSTATUS, clearAll, sizeof clearAll);
  clearIrqLine();

  // The HAL can't see a rejected command, so ask before the PA goes on
  checkRadioStatus(status, "load");

  // SetTx with no hardware timeout, FlightController keeps the time. The HAL waits for BUSY to
  // drop, which is the TCXO start plus the PA ramp, about 5 ms
  if (status == Status::Ok) {
    rfSwitch(true);
  }
  const uint8_t noTimeout[3] = {0x00, 0x00, 0x00};
  command(status, "tx", RADIO_SET_TX, noTimeout, sizeof noTimeout);

  if (status != Status::Ok) {
    // SetTx can have gone out even when its BUSY wait failed, so stop the radio before the switch
    // moves. The HAL still sends the bytes after an error. FlightController only calls
    // finishTransmit after a good start
    Status stop = Status::Ok;
    command(stop, "standby", RADIO_SET_STANDBY, &standby, 1);
    rfSwitch(false);
    ready = false;
  }
  lastCode_ = status == Status::Ok ? 0 : lastError;
  return status;
}

// No SPI while the packet is on air. Only TxDone is routed to DIO1, so a pending IRQ is TxDone.
// finishTransmit reads the real flags
bool Radio::transmitDone()
{
  return HAL_NVIC_GetPendingIRQ(SUBGHZ_Radio_IRQn) != 0;
}

// Runs every step even after a failure, so the transmitter is always told to stop
Status Radio::finishTransmit()
{
  startCall();
  Status status = Status::Ok;
  uint8_t flags[2] = {0x00, 0x00};
  query(status, "irqstatus", RADIO_GET_IRQSTATUS, flags, sizeof flags);
  const uint16_t irq = static_cast<uint16_t>(flags[0] << 8 | flags[1]);
  if (status == Status::Ok && (irq & kRadioIrqTxDone) == 0) {
    fail(status, Status::Failed, "txdone", static_cast<int16_t>(irq));
  }

  Status cleanup = Status::Ok;
  const uint8_t clearAll[2] = {static_cast<uint8_t>(kRadioIrqAll >> 8), static_cast<uint8_t>(kRadioIrqAll)};
  command(cleanup, "clearirq", RADIO_CLR_IRQSTATUS, clearAll, sizeof clearAll);
  clearIrqLine();
  Status stop = Status::Ok;
  const uint8_t standby = kRadioStandbyRc;
  command(stop, "standby", RADIO_SET_STANDBY, &standby, 1);
  rfSwitch(false);

  if (status == Status::Ok) {
    status = cleanup != Status::Ok ? cleanup : stop;
  }
  if (status != Status::Ok) {
    ready = false;
  }
  lastCode_ = status == Status::Ok ? 0 : lastError;
  return status;
}

uint8_t Radio::headerType()
{
  return radioSettings().spreadingFactor <= 6 ? kRadioHeaderImplicit : kRadioHeaderExplicit;
}

uint32_t Radio::timeOnAirMs(size_t length)
{
  constexpr uint32_t bandwidthHundredths = static_cast<uint32_t>(kRadioBandwidthKhz * 100.0f + 0.5f);
  return timeOnAirUs(radioSettings().spreadingFactor, bandwidthHundredths, kRadioCodingRate, kRadioPreambleLength,
                     kRadioCrcOn != 0, headerType() == kRadioHeaderExplicit, length) / 1000;
}

} // namespace ChipSatDevices

#endif
