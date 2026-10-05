// Our radio driver's command bytes and time on air, against RadioLib 7.1.2's own formulas and the bytes on air today
//   g++ -std=gnu++17 -Wall -Wextra test/test_radio_bytes.cpp -o /tmp/radio && /tmp/radio
#include <cmath>
#include <cstdint>
#include <cstdio>
#include "../../V2_6_X_Code/src/devices/own/RadioSubGhzBytes.h"

using namespace ChipSatRadioBytes;

static int checks = 0;
static int failures = 0;

static void check(bool ok, const char *what)
{
  ++checks;
  if (!ok) {
    ++failures;
    printf("FAIL %s\n", what);
  }
}

// RadioLib 7.1.2, copied from src/modules/SX126x/SX126x.cpp with the radio state passed in
// instead of read from the object
namespace library {

// getTimeOnAir, :1457-1487, LoRa branch
static uint32_t getTimeOnAir(uint8_t spreadingFactor, float bandwidthKhz, uint8_t codingRate, uint16_t preambleLengthLoRa,
                             uint8_t crcTypeLoRa, bool headerExplicit, size_t len)
{
  uint32_t symbolLength_us = ((uint32_t)(1000 * 10) << spreadingFactor) / (bandwidthKhz * 10) ;
  uint8_t sfCoeff1_x4 = 17; // (4.25 * 4)
  uint8_t sfCoeff2 = 8;
  if(spreadingFactor == 5 || spreadingFactor == 6) {
    sfCoeff1_x4 = 25; // 6.25 * 4
    sfCoeff2 = 0;
  }
  uint8_t sfDivisor = 4*spreadingFactor;
  if(symbolLength_us >= 16000) {
    sfDivisor = 4*(spreadingFactor - 2);
  }
  const int8_t bitsPerCrc = 16;
  const int8_t N_symbol_header = headerExplicit ? 20 : 0;
  int16_t bitCount = (int16_t) 8 * len + crcTypeLoRa * bitsPerCrc - 4 * spreadingFactor  + sfCoeff2 + N_symbol_header;
  if(bitCount < 0) {
    bitCount = 0;
  }
  uint16_t nPreCodedSymbols = (bitCount + (sfDivisor - 1)) / (sfDivisor);
  uint32_t nSymbol_x4 = (preambleLengthLoRa + 8) * 4 + sfCoeff1_x4 + nPreCodedSymbols * (codingRate + 4) * 4;
  return((symbolLength_us * nSymbol_x4) / 4);
}

// setModulationParams, :2041-2047
static uint8_t ldro(uint8_t spreadingFactor, float bandwidthKhz)
{
  float symbolLength = (float)(uint32_t(1) << spreadingFactor) / (float)bandwidthKhz;
  return symbolLength >= 16.0 ? 0x01 : 0x00;
}

// setFrequencyRaw, :2115, RADIOLIB_SX126X_DIV_EXPONENT 25 and CRYSTAL_FREQ 32.0 from SX126x.h:17-18
static uint32_t frf(float freqMHz)
{
  return (freqMHz * (uint32_t(1) << 25)) / 32.0;
}

} // namespace library

int main()
{
  // What's on air today: 915 MHz, BW 125, SF9, CR 4/7, sync word 0x12
  check(bandwidthCode(125.0f) == 0x04, "BW 125 is 0x04");
  check(lowDataRateOptimize(9, 125.0f) == 0x00, "LDRO off at SF9 BW125");
  check(syncWordMsb(0x12, 0x44) == 0x14 && syncWordLsb(0x12, 0x44) == 0x24, "sync word 0x12 is 0x1424");
  check(frequencyWord(915.0f) == 0x39300000, "915 MHz is 0x39300000");
  check(imageCalibration(915.0f) == 0xE1E9, "915 MHz calibrates E1 E9");
  check(timeOnAirUs(9, 12500, 7, 8, true, true, 55) == 455680, "55 bytes take 455680 us");

  // Every bandwidth in DS Table 13-48, and one that isn't
  const float bandwidths[] = {7.8f, 10.4f, 15.6f, 20.8f, 31.25f, 41.7f, 62.5f, 125.0f, 250.0f, 500.0f};
  const uint8_t codes[] = {0x00, 0x08, 0x01, 0x09, 0x02, 0x0A, 0x03, 0x04, 0x05, 0x06};
  for (int i = 0; i < 10; ++i) {
    check(bandwidthCode(bandwidths[i]) == codes[i], "bandwidth code");
  }
  check(bandwidthCode(100.0f) == 0xFF, "100 kHz refused");

  // LDRO and time on air against RadioLib for every SF, bandwidth, coding rate and length
  int compared = 0;
  for (uint8_t sf = 5; sf <= 12; ++sf) {
    for (float bw : bandwidths) {
      check(lowDataRateOptimize(sf, bw) == library::ldro(sf, bw), "LDRO matches RadioLib");
      const uint32_t hundredths = static_cast<uint32_t>(bw * 100.0f + 0.5f);
      for (uint8_t cr = 5; cr <= 8; ++cr) {
        for (uint32_t len = 0; len <= 255; ++len) {
          const uint32_t ours = timeOnAirUs(sf, hundredths, cr, 8, true, true, len);
          const uint32_t theirs = library::getTimeOnAir(sf, bw, cr - 4, 8, 1, true, len);
          ++compared;
          if (ours != theirs) {
            printf("  sf %u bw %.2f cr 4/%u len %u: ours %u, RadioLib %u\n", sf, bw, cr, len, ours, theirs);
            check(false, "time on air matches RadioLib");
          }
        }
      }
    }
  }
  printf("ok   %d time on air values compared\n", compared);

  // Frequency word across the band, every 100 kHz
  int frequencies = 0;
  for (int f = 9020; f <= 9280; ++f) {
    const float mhz = f / 10.0f;
    ++frequencies;
    if (frequencyWord(mhz) != library::frf(mhz)) {
      printf("  %.1f MHz: ours %08x, RadioLib %08x\n", mhz, frequencyWord(mhz), library::frf(mhz));
      check(false, "frequency word matches RadioLib");
    }
  }
  printf("ok   %d frequencies compared\n", frequencies);

  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
