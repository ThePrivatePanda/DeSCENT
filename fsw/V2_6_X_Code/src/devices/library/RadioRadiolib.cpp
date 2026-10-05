#include <Arduino.h>
#include "../../config.h"
#include "../../constants.h"

#if CHIPSAT_RADIO_DRIVER == CHIPSAT_DRIVER_LIBRARY

// This driver still takes the spreading factor, the power and the module from the compile-time
// constants. The flight driver takes them from the settings block instead, so a board flashed with a
// configured image and built against this one would transmit on the values it was compiled with
// while every tool and the boot line reported the configured ones. Silently. Convert the five uses
// below before using this driver again - they are begin(), setOutputPower(), applyPaConfig() and
// timeOnAirMs()
#error "RadioRadiolib does not read the settings block: it would ignore a configured board's radio settings"

#include <RadioLib.h>
#include "../Radio.h"

namespace ChipSatDevices
{

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

static STM32WLx lora = new STM32WLx_Module();

// RadioLib keeps a pointer to these, they need static storage
static const uint32_t rfswitchPins[] = {PA4, PA5, RADIOLIB_NC, RADIOLIB_NC, RADIOLIB_NC};

// RadioLib picks the PA from which TX mode is in this table
// RX and TX_HP levels are the same as Seeed's E5 mini BSP (LoRaWan-E5-Node,
// Drivers/BSP/STM32WLxx_LoRa_E5_mini/stm32wlxx_LoRa_E5_mini_radio.c, BSP_RADIO_ConfigRFSwitch)
static const Module::RfSwitchMode_t rfswitchTable[] = {
  {STM32WLx::MODE_IDLE, {LOW, LOW}},
  {STM32WLx::MODE_RX, {HIGH, LOW}},
#if CHIPSAT_RADIO_MODULE == CHIPSAT_RADIO_MODULE_LE
  // Seeed's BSP and Erlkoenig90/WioE5-Demo (an LE project) use {HIGH, HIGH} for RFO_LP, but the
  // Seeed board is HP only so that case never runs there. {LOW, HIGH} is what flew on the roof drop
  // {HIGH, HIGH} not tried yet
  {STM32WLx::MODE_TX_LP, {LOW, HIGH}},
#else
  {STM32WLx::MODE_TX_HP, {LOW, HIGH}},
#endif
  END_OF_MODE_TABLE,
};

static Status fromCode(int16_t code)
{
  if (code == RADIOLIB_ERR_NONE) {
    return Status::Ok;
  }
  if (code == RADIOLIB_ERR_TX_TIMEOUT) {
    return Status::Timeout;
  }
  return Status::Failed;
}

Radio::Radio(ChipSatPlatform::System &system)
  : system_(system)
{
}

Status Radio::begin()
{
  lora.setRfSwitchTable(rfswitchPins, rfswitchTable);

  lastCode_ = lora.begin(kRadioFrequencyMhz, kRadioBandwidthKhz, kRadioSpreadingFactor, kRadioCodingRate,
                         kRadioSyncWord, kRadioPowerDbm, kRadioPreambleLength, kRadioTcxoVoltage, kRadioUseLdo);
  ldroCode_ = lora.autoLDRO();   // runs before begin's result is checked

  return fromCode(lastCode_);
}

Status Radio::setCurrentLimit()
{
  lastCode_ = lora.setCurrentLimit(kRadioCurrentLimitMa);
  return fromCode(lastCode_);
}

float Radio::currentLimitMa()
{
  return lora.getCurrentLimit();
}

Status Radio::setOutputPower()
{
  lastCode_ = lora.setOutputPower(kRadioPowerDbm);
  return fromCode(lastCode_);
}

Status Radio::applyPaConfig()
{
  lastCode_ = lora.setOutputPower(kPaPowerDbm);
  if (lastCode_ != RADIOLIB_ERR_NONE) {
    return fromCode(lastCode_);
  }
  lastCode_ = lora.setPaConfig(kPaDutyCycle, kPaDeviceSel, kPaHpMax, kPaLut);
  return fromCode(lastCode_);
}

// Same steps as RadioLib's own transmit() (SX126x.cpp:195), minus its wait loop
Status Radio::startTransmit(const uint8_t *data, size_t length)
{
  lastCode_ = lora.standby();
  if (lastCode_ != RADIOLIB_ERR_NONE) {
    return fromCode(lastCode_);
  }
  lastCode_ = lora.startTransmit(data, length);
  return fromCode(lastCode_);
}

// The interrupt line, like RadioLib's own loop. Reading the flags over SPI instead can give 0xFFFF
// from a dead radio, which would look done
bool Radio::transmitDone()
{
  return SubGhz.isInterruptPending();
}

Status Radio::finishTransmit()
{
  lastCode_ = lora.finishTransmit();
  return fromCode(lastCode_);
}

uint32_t Radio::timeOnAirMs(size_t length)
{
  return static_cast<uint32_t>(lora.getTimeOnAir(length) / 1000);
}

} // namespace ChipSatDevices

#endif
