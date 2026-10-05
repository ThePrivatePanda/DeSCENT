// Writes "Hello, World!" to the W25Q16JV and reads it back, timing each step. Uses the last sector,
// so the start of the chip is left alone. Console PB7/PB6 at 115200

#include <SPI.h>

HardwareSerial console(PB7, PB6);
SPIClass flashSpi(PA10, PB14, PB13);   // mosi, miso, sck. SPI2

constexpr uint8_t kCsPin = PB9;
const SPISettings kSpiSettings(4000000, MSBFIRST, SPI_MODE0);

constexpr uint8_t kCmdWriteEnable = 0x06;
constexpr uint8_t kCmdReadStatus1 = 0x05;
constexpr uint8_t kCmdRead = 0x03;
constexpr uint8_t kCmdPageProgram = 0x02;
constexpr uint8_t kCmdSectorErase = 0x20;
constexpr uint8_t kStatusBusy = 0x01;

constexpr uint32_t kAddress = 0x1FF000;   // last 4 kB sector

const char kText[] = "Hello, World!";
char readBack[sizeof kText];

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

  // flash has to be erased before it can be written
  select();
  flashSpi.transfer(kCmdWriteEnable);
  deselect();
  select();
  flashSpi.transfer(kCmdSectorErase);
  sendAddress(kAddress);
  deselect();
  console.print("erase 4 kB  us=");
  console.println(waitDone(1000000));   // datasheet max 400 ms

  const uint32_t writeStart = micros();
  select();
  flashSpi.transfer(kCmdWriteEnable);
  deselect();
  select();
  flashSpi.transfer(kCmdPageProgram);
  sendAddress(kAddress);
  for (size_t i = 0; i < sizeof kText; i++) {
    flashSpi.transfer(static_cast<uint8_t>(kText[i]));
  }
  deselect();
  const int32_t busyUs = waitDone(10000);   // datasheet max 3 ms
  console.print("write \"");
  console.print(kText);
  console.print("\"  us=");
  console.print(micros() - writeStart);
  console.print("  of which waiting=");
  console.println(busyUs);

  const uint32_t readStart = micros();
  select();
  flashSpi.transfer(kCmdRead);
  sendAddress(kAddress);
  for (size_t i = 0; i < sizeof readBack; i++) {
    readBack[i] = static_cast<char>(flashSpi.transfer(0x00));
  }
  deselect();
  const uint32_t readUs = micros() - readStart;

  readBack[sizeof readBack - 1] = '\0';
  console.print("read  \"");
  console.print(readBack);
  console.print("\"  us=");
  console.println(readUs);

  console.println(strcmp(readBack, kText) == 0 ? "same" : "different");
}

void loop()
{
}
