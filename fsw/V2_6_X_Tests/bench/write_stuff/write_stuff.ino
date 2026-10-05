// Writes some stuff to the W25Q16JV and times it, nothing else. Uses the last sector, so the start of the
// chip is left alone. Flash needs an erase before a write, so that's timed too. Console PB7/PB6 at 115200

#include <SPI.h>

HardwareSerial console(PB7, PB6);
SPIClass flashSpi(PA10, PB14, PB13);   // mosi, miso, sck. SPI2

constexpr uint8_t kCsPin = PB9;
const SPISettings kSpiSettings(4000000, MSBFIRST, SPI_MODE0);

constexpr uint8_t kCmdWriteEnable = 0x06;
constexpr uint8_t kCmdReadStatus1 = 0x05;
constexpr uint8_t kCmdPageProgram = 0x02;
constexpr uint8_t kCmdSectorErase = 0x20;
constexpr uint8_t kStatusBusy = 0x01;

constexpr uint32_t kSector = 0x1FF000;   // last 4 kB
constexpr uint8_t kRecord = 64;          // one log record
constexpr uint8_t kRecords = 8;

uint8_t stuff[kRecord];

void select()
{
  flashSpi.beginTransaction(kSpiSettings);
  digitalWrite(kCsPin, LOW);
}

void deselect()
{
  digitalWrite(kCsPin, HIGH);
  flashSpi.endTransaction();
}

void command(uint8_t cmd)
{
  select();
  flashSpi.transfer(cmd);
  deselect();
}

void sendAddress(uint32_t address)
{
  flashSpi.transfer(static_cast<uint8_t>(address >> 16));
  flashSpi.transfer(static_cast<uint8_t>(address >> 8));
  flashSpi.transfer(static_cast<uint8_t>(address));
}

// Microseconds until the chip says it's done, or -1 if it never does
int32_t waitDone(uint32_t timeoutUs)
{
  const uint32_t start = micros();
  while (micros() - start < timeoutUs) {
    select();
    flashSpi.transfer(kCmdReadStatus1);
    const uint8_t status = flashSpi.transfer(0x00);
    deselect();
    if ((status & kStatusBusy) == 0) {
      return static_cast<int32_t>(micros() - start);
    }
  }
  return -1;
}

void setup()
{
  console.begin(115200);
  delay(300);
  console.println();

  pinMode(kCsPin, OUTPUT);
  digitalWrite(kCsPin, HIGH);
  flashSpi.begin();

  for (uint8_t i = 0; i < kRecord; i++) {
    stuff[i] = static_cast<uint8_t>(i);
  }

  command(kCmdWriteEnable);
  select();
  flashSpi.transfer(kCmdSectorErase);
  sendAddress(kSector);
  deselect();
  const int32_t eraseUs = waitDone(1000000);   // datasheet max 400 ms
  console.print("erase 4 kB  us=");
  console.println(eraseUs);

  for (uint8_t r = 0; r < kRecords; r++) {
    const uint32_t start = micros();
    command(kCmdWriteEnable);
    select();
    flashSpi.transfer(kCmdPageProgram);
    sendAddress(kSector + r * kRecord);
    for (uint8_t i = 0; i < kRecord; i++) {
      flashSpi.transfer(stuff[i]);
    }
    deselect();
    const int32_t doneUs = waitDone(10000);   // datasheet max 3 ms
    const uint32_t wholeUs = micros() - start;

    console.print("write 64 B  us=");
    console.print(wholeUs);
    console.print("  of which waiting=");
    console.println(doneUs);
  }
}

void loop()
{
}
