#include <Arduino.h>
#include <pinconfig.h>
#include <pinmap.h>
#include "../../config.h"
#include "../../constants.h"
#include "../Flash.h"
#include "../../log/Log.h"

namespace ChipSatDevices
{

using ChipSatPlatform::Status;
using namespace ChipSatConstants;

// Straight onto ST's HAL SPI driver, the core's SPI library isn't in the CMake build. The HAL gives
// up on each transfer after kFlashSpiTimeoutMs

static SPI_HandleTypeDef spi;

static Status fromHal(HAL_StatusTypeDef result)
{
  if (result == HAL_OK) {
    return Status::Ok;
  }
  return result == HAL_TIMEOUT ? Status::Timeout : Status::BusError;
}

// One command: /CS low, the opcode and maybe an address, then data out or in, /CS high
static Status frame(const uint8_t *header, uint16_t headerLength, const uint8_t *out, uint8_t *in, size_t length)
{
  digitalWrite(kFlashCsPin, LOW);
  HAL_StatusTypeDef result = HAL_SPI_Transmit(&spi, header, headerLength, kFlashSpiTimeoutMs);
  if (result == HAL_OK && length > 0) {
    // Receive clocks the buffer's own bytes out while reading, the chip ignores them
    result = out != nullptr ? HAL_SPI_Transmit(&spi, out, static_cast<uint16_t>(length), kFlashSpiTimeoutMs)
                            : HAL_SPI_Receive(&spi, in, static_cast<uint16_t>(length), kFlashSpiTimeoutMs);
  }
  digitalWrite(kFlashCsPin, HIGH);
  return fromHal(result);
}

static Status command(uint8_t opcode)
{
  return frame(&opcode, 1, nullptr, nullptr, 0);
}

static void addressed(uint8_t header[4], uint8_t opcode, uint32_t address)
{
  header[0] = opcode;
  header[1] = static_cast<uint8_t>(address >> 16);
  header[2] = static_cast<uint8_t>(address >> 8);
  header[3] = static_cast<uint8_t>(address);
}

Flash::Flash(ChipSatPlatform::System &system) : system_(system)
{
}

Status Flash::readStatus(uint8_t &status)
{
  const uint8_t opcode = kFlashCmdReadStatus1;
  return frame(&opcode, 1, nullptr, &status, 1);
}

Status Flash::readId(uint8_t id[3])
{
  const uint8_t opcode = kFlashCmdJedecId;
  return frame(&opcode, 1, nullptr, id, 3);
}

// The chip drops a program or erase without any error if WEL isn't set, so check it took
Status Flash::writeEnable()
{
  uint8_t status = 0;
  Status result = readStatus(status);
  if (result != Status::Ok) {
    return result;
  }
  if ((status & kFlashStatusBusy) != 0) {
    return Status::NotReady;
  }
  result = command(kFlashCmdWriteEnable);
  if (result == Status::Ok) {
    result = readStatus(status);
  }
  if (result == Status::Ok && (status & kFlashStatusWel) == 0) {
    result = Status::Failed;
  }
  return result;
}

Status Flash::waitWhileBusy(uint32_t timeoutMs, uint32_t pollMs)
{
  const uint32_t startMs = system_.nowMs();
  while (true) {
    uint8_t status = 0;
    const Status result = readStatus(status);
    if (result != Status::Ok) {
      return result;
    }
    // MISO has a pull-up, so all FF is nothing answering. We never write the status register, the
    // protect bits that would make a real FF stay 0
    if (status == 0xFF) {
      return Status::NoAck;
    }
    if ((status & kFlashStatusBusy) == 0) {
      return Status::Ok;
    }
    if (system_.nowMs() - startMs > timeoutMs) {
      return Status::Timeout;
    }
    system_.waitMs(pollMs);
  }
}

Status Flash::begin()
{
  LOG_I(Flash, "start");
  ready_ = false;
  paused_ = false;

  pinMode(kFlashCsPin, OUTPUT);
  digitalWrite(kFlashCsPin, HIGH);

  // Same set-up as the core's spi_init, which the bench sketches ran on
  pinmap_pinout(digitalPinToPinName(kFlashMosiPin), PinMap_SPI_MOSI);
  pinmap_pinout(digitalPinToPinName(kFlashMisoPin), PinMap_SPI_MISO);
  pinmap_pinout(digitalPinToPinName(kFlashSckPin), PinMap_SPI_SCLK);
  const PinName sck = digitalPinToPinName(kFlashSckPin);
  pin_PullConfig(get_GPIO_Port(STM_PORT(sck)), STM_LL_GPIO_PIN(sck), GPIO_PULLDOWN);   // idles low in mode 0
  __HAL_RCC_SPI2_CLK_ENABLE();
  __HAL_RCC_SPI2_FORCE_RESET();
  __HAL_RCC_SPI2_RELEASE_RESET();

  spi.Instance = SPI2;
  spi.Init.Mode = SPI_MODE_MASTER;
  spi.Init.Direction = SPI_DIRECTION_2LINES;
  spi.Init.DataSize = SPI_DATASIZE_8BIT;
  spi.Init.CLKPolarity = SPI_POLARITY_LOW;
  spi.Init.CLKPhase = SPI_PHASE_1EDGE;
  spi.Init.NSS = SPI_NSS_SOFT;
  spi.Init.BaudRatePrescaler = SPI_BAUDRATEPRESCALER_16;   // 3 MHz off the 48 MHz PCLK1, what the bench sketches got
  spi.Init.FirstBit = SPI_FIRSTBIT_MSB;
  spi.Init.TIMode = SPI_TIMODE_DISABLE;
  spi.Init.CRCCalculation = SPI_CRCCALCULATION_DISABLE;
  spi.Init.CRCPolynomial = 7;
  spi.Init.CRCLength = SPI_CRC_LENGTH_DATASIZE;
  spi.Init.NSSPMode = SPI_NSS_PULSE_DISABLE;
  Status status = fromHal(HAL_SPI_Init(&spi));
  if (status == Status::Ok) {
    __HAL_SPI_ENABLE(&spi);
  }

  // A reset of the MCU alone doesn't reset the chip. It might still be powered down, or busy with
  // an erase we started before the reset
  if (status == Status::Ok) {
    status = command(kFlashCmdReleasePowerDown);
    system_.waitMs(kFlashWakeMs);
  }
  if (status == Status::Ok) {
    status = waitWhileBusy(kFlashEraseTimeoutMs, 1);
  }

  uint8_t id[3] = {0, 0, 0};
  const Status idStatus = readId(id);
  if (status == Status::Ok) {
    status = idStatus;
  }
  if (status == Status::Ok &&
      (id[0] != kFlashManufacturerId || id[1] != kFlashMemoryType || id[2] != kFlashCapacity)) {
    status = Status::BadData;
  }
  const uint32_t idWord = static_cast<uint32_t>(id[0]) << 16 | static_cast<uint32_t>(id[1]) << 8 | id[2];

  if (status != Status::Ok) {
    LOG_E(Flash, "init") {
      line.field("ok", false);
      line.fieldHex("id", idWord, 6);
      line.field("reason", ChipSatPlatform::statusName(status));
    }
    return status;
  }
  ready_ = true;
  LOG_I(Flash, "init") {
    line.field("ok", true);
    line.fieldHex("id", idWord, 6);
  }
  return Status::Ok;
}

// A read during an erase would come back as all FF and look like an empty log, so it's refused
Status Flash::read(uint32_t address, uint8_t *buffer, size_t length)
{
  if (!usable()) {
    return Status::NotReady;
  }
  if (busy()) {
    return Status::NotReady;
  }
  uint8_t header[4];
  addressed(header, kFlashCmdRead, address);
  return frame(header, sizeof header, nullptr, buffer, length);
}

Status Flash::program(uint32_t address, const uint8_t *data, size_t length)
{
  if (!usable()) {
    return Status::NotReady;
  }
  if (length == 0 || address % kFlashPageSize + length > kFlashPageSize || address + length > kFlashSize) {
    return Status::Failed;
  }
  Status status = writeEnable();
  if (status != Status::Ok) {
    return status;
  }
  uint8_t header[4];
  addressed(header, kFlashCmdPageProgram, address);
  status = frame(header, sizeof header, data, nullptr, length);
  if (status != Status::Ok) {
    return status;
  }
  return waitWhileBusy(kFlashProgramTimeoutMs, 1);
}

Status Flash::startSectorErase(uint32_t address)
{
  if (!usable()) {
    return Status::NotReady;
  }
  if (address % kFlashSectorSize != 0 || address >= kFlashSize) {
    return Status::Failed;
  }
  const Status status = writeEnable();
  if (status != Status::Ok) {
    return status;
  }
  uint8_t header[4];
  addressed(header, kFlashCmdSectorErase, address);
  return frame(header, sizeof header, nullptr, nullptr, 0);
}

bool Flash::busy()
{
  // Nothing can be running: pause only goes ahead when the chip is idle
  if (!usable()) {
    return false;
  }
  uint8_t status = 0;
  if (readStatus(status) != Status::Ok) {
    return true;
  }
  return (status & kFlashStatusBusy) != 0;
}

Status Flash::eraseChip()
{
  if (!usable()) {
    return Status::NotReady;
  }
  LOG_I(Flash, "erase") { line.field("what", "chip"); }
  Status status = writeEnable();
  if (status == Status::Ok) {
    status = command(kFlashCmdChipErase);
  }
  if (status == Status::Ok) {
    status = waitWhileBusy(kFlashChipEraseTimeoutMs, 100);
  }
  LOG_I(Flash, "erase") {
    line.field("what", "chip");
    line.field("ok", status == Status::Ok);
  }
  return status;
}

Status Flash::pause()
{
  if (!ready_) {
    return Status::NotReady;
  }
  if (paused_) {
    return Status::Ok;
  }
  uint8_t status = 0;
  const Status result = readStatus(status);
  if (result != Status::Ok) {
    return result;
  }
  if ((status & kFlashStatusBusy) != 0) {
    return Status::NotReady;
  }
  const Status sent = command(kFlashCmdPowerDown);
  if (sent == Status::Ok) {
    paused_ = true;
  }
  return sent;
}

Status Flash::resume()
{
  if (!ready_) {
    return Status::NotReady;
  }
  if (!paused_) {
    return Status::Ok;
  }
  // The id only reads back once the chip is awake, so it doubles as the check. A second try in
  // case the wake came within tDP (3 us) of the power-down and got ignored
  uint8_t id[3] = {0, 0, 0};
  Status status = Status::Ok;
  for (uint8_t attempt = 0; attempt < 2; ++attempt) {
    status = command(kFlashCmdReleasePowerDown);
    system_.waitMs(kFlashWakeMs);
    if (status == Status::Ok) {
      status = readId(id);
    }
    if (status == Status::Ok &&
        (id[0] != kFlashManufacturerId || id[1] != kFlashMemoryType || id[2] != kFlashCapacity)) {
      status = Status::NoAck;
    }
    if (status == Status::Ok) {
      paused_ = false;
      return Status::Ok;
    }
  }
  LOG_W(Flash, "wake") {
    line.field("ok", false);
    line.fieldHex("id", static_cast<uint32_t>(id[0]) << 16 | static_cast<uint32_t>(id[1]) << 8 | id[2], 6);
  }
  return status;
}

} // namespace ChipSatDevices
