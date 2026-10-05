// Reads some stuff back off the W25Q16JV and times it, nothing else. Reads the last sector, where
// write_stuff left its records. Console PB7/PB6 at 115200

#include <SPI.h>

HardwareSerial console(PB7, PB6);
SPIClass flashSpi(PA10, PB14, PB13);   // mosi, miso, sck. SPI2

constexpr uint8_t kCsPin = PB9;
const SPISettings kSpiSettings(4000000, MSBFIRST, SPI_MODE0);

constexpr uint8_t kCmdRead = 0x03;

constexpr uint32_t kSector = 0x1FF000;   // last 4 kB
constexpr uint8_t kRecord = 64;          // one log record
constexpr uint8_t kRecords = 8;

uint8_t stuff[kRecord];

void readAt(uint32_t address, uint8_t *into, size_t length)
{
  flashSpi.beginTransaction(kSpiSettings);
  digitalWrite(kCsPin, LOW);
  flashSpi.transfer(kCmdRead);
  flashSpi.transfer(static_cast<uint8_t>(address >> 16));
  flashSpi.transfer(static_cast<uint8_t>(address >> 8));
  flashSpi.transfer(static_cast<uint8_t>(address));
  for (size_t i = 0; i < length; i++) {
    into[i] = flashSpi.transfer(0x00);
  }
  digitalWrite(kCsPin, HIGH);
  flashSpi.endTransaction();
}

void setup()
{
  console.begin(115200);
  delay(300);
  console.println();

  pinMode(kCsPin, OUTPUT);
  digitalWrite(kCsPin, HIGH);
  flashSpi.begin();

  for (uint8_t r = 0; r < kRecords; r++) {
    const uint32_t start = micros();
    readAt(kSector + r * kRecord, stuff, kRecord);
    const uint32_t us = micros() - start;

    console.print("read 64 B  us=");
    console.print(us);
    console.print("  first 8:");
    for (uint8_t i = 0; i < 8; i++) {
      console.print(' ');
      if (stuff[i] < 0x10) {
        console.print('0');
      }
      console.print(stuff[i], HEX);
    }
    console.println();
  }
}

void loop()
{
}
