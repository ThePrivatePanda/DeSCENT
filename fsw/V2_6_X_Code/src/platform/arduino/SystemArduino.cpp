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
    // not for 0, the IMU's read loops run it once a packet
    if (ms > 0 && millis() - startMs < ms) {
      idleUntilTick();
    }
  } while (millis() - startMs < ms);
}

void System::idleUntilTick() const
{
  if (ChipSatConfig::kSleepWhenIdle) {
    __WFI();
  }
}

// LPTIM1 counts LSI / 32, so about a ms a tick (LSI's own error, a few percent, is in the uptime
// too), and its match is the one interrupt Stop2 waits for. sleepFor clears it before interrupts are
// back on, so this only runs if that ever misses one; the default handler would hang
extern "C" void LPTIM1_IRQHandler(void)
{
  LPTIM1->ICR = LPTIM_ICR_ARRMCF;
}

uint32_t System::sleepFor(uint32_t ms) const
{
  static bool timerReady = false;
  if (!timerReady) {
    __HAL_RCC_LSI_ENABLE();   // the IWDG has it on already
    while ((RCC->CSR & RCC_CSR_LSIRDY) == 0) {
    }
    __HAL_RCC_LPTIM1_CONFIG(RCC_LPTIM1CLKSOURCE_LSI);
    __HAL_RCC_LPTIM1_CLK_ENABLE();
    LPTIM1->CR = 0;
    LPTIM1->CFGR = 5U << LPTIM_CFGR_PRESC_Pos;   // these two only while it's off
    LPTIM1->IER = LPTIM_IER_ARRMIE;
    EXTI->IMR1 |= EXTI_IMR1_IM29;   // LPTIM1's wake-up line
    NVIC_EnableIRQ(LPTIM1_IRQn);
    timerReady = true;
  }
  if (ms > ChipSatConfig::kSleepMaxMs) {
    ms = ChipSatConfig::kSleepMaxMs;
  }
  const uint32_t ticks = ms * LSI_VALUE / 32000UL;
  if (ticks < 2) {
    return 0;
  }

  IWatchdog.reload();
  LPTIM1->CR = LPTIM_CR_ENABLE;
  LPTIM1->ICR = LPTIM_ICR_ARROKCF | LPTIM_ICR_ARRMCF;
  LPTIM1->ARR = ticks;
  while ((LPTIM1->ISR & LPTIM_ISR_ARROK) == 0) {
  }
  HAL_SuspendTick();
  SCB->ICSR = SCB_ICSR_PENDSTCLR_Msk;   // a tick already pending would end the WFI at once

  // With interrupts masked the match can't be taken between starting the timer and the WFI, which
  // would sleep until the watchdog. A pending one still ends the WFI
  __disable_irq();
  LPTIM1->CR = LPTIM_CR_ENABLE | LPTIM_CR_SNGSTRT;
  HAL_PWREx_EnterSTOP2Mode(PWR_STOPENTRY_WFI);
  // Stop2 wakes on MSI, at the range MSIRANGE or MSISRANGE gives (RM0461 not checked here), so the
  // core's own setup puts back MSI range 11, 48 MHz, and two flash wait states either way
  SystemClock_Config();
  uint32_t count = LPTIM1->CNT;
  while (count != LPTIM1->CNT) {   // read twice to agree, it runs on another clock
    count = LPTIM1->CNT;
  }
  const bool matched = (LPTIM1->ISR & LPTIM_ISR_ARRM) != 0;
  LPTIM1->ICR = LPTIM_ICR_ARRMCF;
  NVIC_ClearPendingIRQ(LPTIM1_IRQn);
  LPTIM1->CR = 0;
  const uint32_t slept = (matched ? ticks : count) * 32000UL / LSI_VALUE;
  uwTick += slept;   // before the tick's own interrupt can run again
  HAL_ResumeTick();
  __enable_irq();
  return slept;
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
