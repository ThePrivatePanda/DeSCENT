#pragma once
// Our radio settings as command bytes, in RadioLib 7.1.2's maths (line numbers are its SX126x.cpp)

#include <stdint.h>

namespace ChipSatRadioBytes
{

// SetModulationParams bandwidth code, DS Table 13-48. 0xFF if the chip can't do it. Matched on
// half the bandwidth with RadioLib's rounding, setBandwidth :860
constexpr uint8_t halfBandwidth(float bandwidthKhz)
{
  return static_cast<uint8_t>(bandwidthKhz / 2 + 0.01f);
}

constexpr uint8_t bandwidthCode(float bandwidthKhz)
{
  return halfBandwidth(bandwidthKhz) == 3     ? 0x00   // 7.8
         : halfBandwidth(bandwidthKhz) == 5   ? 0x08   // 10.4
         : halfBandwidth(bandwidthKhz) == 7   ? 0x01   // 15.6
         : halfBandwidth(bandwidthKhz) == 10  ? 0x09   // 20.8
         : halfBandwidth(bandwidthKhz) == 15  ? 0x02   // 31.25
         : halfBandwidth(bandwidthKhz) == 20  ? 0x0A   // 41.7
         : halfBandwidth(bandwidthKhz) == 31  ? 0x03   // 62.5
         : halfBandwidth(bandwidthKhz) == 62  ? 0x04   // 125
         : halfBandwidth(bandwidthKhz) == 125 ? 0x05   // 250
         : halfBandwidth(bandwidthKhz) == 250 ? 0x06   // 500
                                              : 0xFF;
}

// RadioLib turns LDRO on when a symbol lasts 16 ms or more, :2041-2047
constexpr uint8_t lowDataRateOptimize(uint8_t spreadingFactor, float bandwidthKhz)
{
  return static_cast<float>(1UL << spreadingFactor) / bandwidthKhz >= 16.0f ? 0x01 : 0x00;
}

// Sync word register pair, :934. 0x12 with control bits 0x44 gives 0x14 0x24
constexpr uint8_t syncWordMsb(uint8_t syncWord, uint8_t controlBits)
{
  return static_cast<uint8_t>((syncWord & 0xF0) | ((controlBits & 0xF0) >> 4));
}

constexpr uint8_t syncWordLsb(uint8_t syncWord, uint8_t controlBits)
{
  return static_cast<uint8_t>(((syncWord & 0x0F) << 4) | (controlBits & 0x0F));
}

// SetRfFrequency word, MHz x 2^25 / 32 MHz, :2115. Same truncation as RadioLib's float maths,
// since scaling by a power of two is exact in float
constexpr uint32_t frequencyWord(float frequencyMhz)
{
  return static_cast<uint32_t>(frequencyMhz * 1048576.0f);
}

// CalibrateImage band from the DS Table 9-2 list RadioLib uses, :1928-1948. 0 if the frequency is
// in none of them, RadioLib then works one out itself and that isn't copied here
constexpr bool inBand(float frequencyMhz, int low, int high)
{
  return static_cast<int>(frequencyMhz) >= low && static_cast<int>(frequencyMhz) <= high;
}

constexpr uint16_t imageCalibration(float frequencyMhz)
{
  return inBand(frequencyMhz, 902, 928)   ? 0xE1E9
         : inBand(frequencyMhz, 863, 870) ? 0xD7DB
         : inBand(frequencyMhz, 779, 787) ? 0xC1C5
         : inBand(frequencyMhz, 470, 510) ? 0x7581
         : inBand(frequencyMhz, 430, 440) ? 0x6B6F
                                          : 0;
}

// LoRa time on air in microseconds, RadioLib's getTimeOnAir, :1457-1487, in integers.
// codingRate is the 5 to 8 of 4/5 to 4/8. The bandwidth goes in hundredths of a kHz so it stays
// integer, 31.25 included
inline uint32_t timeOnAirUs(uint8_t spreadingFactor, uint32_t bandwidthHundredthsKhz, uint8_t codingRate,
                            uint16_t preambleLength, bool crcOn, bool explicitHeader, uint32_t length)
{
  const uint32_t symbolUs = (100000UL << spreadingFactor) / bandwidthHundredthsKhz;
  uint32_t coeff1x4 = 17;   // 4.25 symbols, x4
  int32_t coeff2 = 8;
  if (spreadingFactor == 5 || spreadingFactor == 6) {
    coeff1x4 = 25;
    coeff2 = 0;
  }
  const uint32_t divisor = 4 * (symbolUs >= 16000 ? spreadingFactor - 2 : spreadingFactor);

  int32_t bits = static_cast<int32_t>(8 * length) + (crcOn ? 16 : 0) - 4 * spreadingFactor + coeff2 +
                 (explicitHeader ? 20 : 0);
  if (bits < 0) {
    bits = 0;
  }
  const uint32_t blocks = (static_cast<uint32_t>(bits) + divisor - 1) / divisor;
  const uint32_t symbolsX4 = (static_cast<uint32_t>(preambleLength) + 8) * 4 + coeff1x4 + blocks * codingRate * 4;
  return symbolUs * symbolsX4 / 4;
}

} // namespace ChipSatRadioBytes
