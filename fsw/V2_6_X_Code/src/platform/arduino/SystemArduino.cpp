#include <Arduino.h>
#include "../../config.h"

#if CHIPSAT_SYSTEM_DRIVER == CHIPSAT_DRIVER_LIBRARY

#include <IWatchdog.h>
#include "../System.h"

extern "C" caddr_t _sbrk(int incr);

namespace ChipSatPlatform
{

namespace
{
constexpr uint32_t kStackPaint = 0x5AC3A55Cu;
uint32_t *paintBottom = nullptr;
uint32_t *paintTop = nullptr;
}

uint32_t System::nowMs() const
{
  return millis();
}

void System::waitMs(uint32_t ms) const
{
  const uint32_t startMs = millis();
  do {
    IWatchdog.reload();
    if (idle_ != nullptr) {
      idle_();
    }
  } while (millis() - startMs < ms);
}

void System::startWatchdog(uint32_t timeoutMs)
{
  IWatchdog.begin(timeoutMs * 1000UL);
}

void System::feedWatchdog() const
{
  IWatchdog.reload();
}

ResetCause System::readAndClearResetCause()
{
  ResetCause cause;
  cause.csr = RCC->CSR;
  cause.lowPower = (cause.csr & RCC_CSR_LPWRRSTF) != 0;
  cause.windowWatchdog = (cause.csr & RCC_CSR_WWDGRSTF) != 0;
  cause.independentWatchdog = (cause.csr & RCC_CSR_IWDGRSTF) != 0;
  cause.software = (cause.csr & RCC_CSR_SFTRSTF) != 0;
  cause.brownOut = (cause.csr & RCC_CSR_BORRSTF) != 0;
  cause.pin = (cause.csr & RCC_CSR_PINRSTF) != 0;
  cause.optionByteLoad = (cause.csr & RCC_CSR_OBLRSTF) != 0;
  cause.radioIllegalAccess = (cause.csr & RCC_CSR_RFILARSTF) != 0;

  __HAL_RCC_CLEAR_RESET_FLAGS();
  return cause;
}

// Stops 64 bytes below the stack pointer so this frame is left alone. Heap taken later shows up as
// used too, which is fine, it's room the two share
void System::paintStack()
{
  const uintptr_t bottom = (reinterpret_cast<uintptr_t>(_sbrk(0)) + 3U) & ~static_cast<uintptr_t>(3U);
  const uintptr_t top = (static_cast<uintptr_t>(__get_MSP()) - 64U) & ~static_cast<uintptr_t>(3U);
  if (top <= bottom) {
    return;
  }
  paintBottom = reinterpret_cast<uint32_t *>(bottom);
  paintTop = reinterpret_cast<uint32_t *>(top);
  for (volatile uint32_t *at = paintBottom; at < paintTop; ++at) {
    *at = kStackPaint;
  }
}

// From the heap's top as it is now, so heap the libraries took after boot isn't counted as stack
uint32_t System::stackUntouchedBytes() const
{
  if (paintBottom == nullptr) {
    return 0;
  }
  const uintptr_t heapTop = (reinterpret_cast<uintptr_t>(_sbrk(0)) + 3U) & ~static_cast<uintptr_t>(3U);
  const volatile uint32_t *from = heapTop > reinterpret_cast<uintptr_t>(paintBottom)
    ? reinterpret_cast<const uint32_t *>(heapTop) : paintBottom;
  const volatile uint32_t *at = from;
  while (at < paintTop && *at == kStackPaint) {
    ++at;
  }
  return at > from ? static_cast<uint32_t>(reinterpret_cast<uintptr_t>(at) - reinterpret_cast<uintptr_t>(from)) : 0;
}

} // namespace ChipSatPlatform

#endif
