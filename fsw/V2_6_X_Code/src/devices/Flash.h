#pragma once

#include <stddef.h>
#include <stdint.h>
#include "../platform/Status.h"
#include "../platform/System.h"

namespace ChipSatDevices
{

// W25Q16JV, 2 MB of NOR flash on SPI2. Only the commands the log needs, the status registers are
// never written
class Flash
{
public:
  explicit Flash(ChipSatPlatform::System &system);

  // Refuses anything but EF 40 15. Waits out an erase a reset left running, kFlashEraseTimeoutMs at most
  ChipSatPlatform::Status begin();

  ChipSatPlatform::Status read(uint32_t address, uint8_t *buffer, size_t length);
  // Has to stay inside one 256 byte page, the chip wraps to the page start otherwise. Waits for it
  ChipSatPlatform::Status program(uint32_t address, const uint8_t *data, size_t length);
  ChipSatPlatform::Status startSectorErase(uint32_t address);   // returns straight away, see busy()
  bool busy();   // also true when the chip doesn't answer
  ChipSatPlatform::Status eraseChip();   // ground only, blocks for up to 30 s

  // Deep power-down, about 1 uA. The chip ignores it during a program or erase, so then it stays
  // awake and this returns NotReady
  ChipSatPlatform::Status pause();
  ChipSatPlatform::Status resume();   // Ok straight away if not paused, else wakes it and checks the id

  bool ready() const { return ready_; }
  bool paused() const { return paused_; }

private:
  ChipSatPlatform::Status readStatus(uint8_t &status);
  ChipSatPlatform::Status readId(uint8_t id[3]);
  ChipSatPlatform::Status writeEnable();
  ChipSatPlatform::Status waitWhileBusy(uint32_t timeoutMs, uint32_t pollMs);
  bool usable() const { return ready_ && !paused_; }

  ChipSatPlatform::System &system_;
  bool ready_ = false;
  bool paused_ = false;
};

} // namespace ChipSatDevices
