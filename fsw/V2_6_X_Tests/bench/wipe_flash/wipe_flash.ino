// Erases the whole W25Q16JV, then reads all 2 MB back to check it. THE LOG IS GONE FOR GOOD.
// Nothing is typed to confirm it, the console's RX isn't wired on every bench lead. There's a 5 s
// countdown instead. Console PB7/PB6 at 115200
//   ./build.sh flash --sketch wipe_flash    then ./build.sh flash to put the flight software back

#include <SPI.h>

HardwareSerial console(PB7, PB6);
SPIClass flashSpi(PA10, PB14, PB13);   // mosi, miso, sck. SPI2

constexpr uint8_t kCsPin = PB9;
const SPISettings kSpiSettings(4000000, MSBFIRST, SPI_MODE0);

constexpr uint8_t kCmdJedecId = 0x9F;
constexpr uint8_t kCmdRead = 0x03;
constexpr uint8_t kCmdWriteEnable = 0x06;
constexpr uint8_t kCmdChipErase = 0xC7;
constexpr uint8_t kCmdStatus1 = 0x05;
constexpr uint8_t kCmdStatus2 = 0x35;
constexpr uint8_t kCmdReleasePowerDown = 0xAB;

constexpr uint8_t kStatusBusy = 0x01;
constexpr uint8_t kStatusWel = 0x02;
constexpr uint8_t kStatusBp = 0x7C;    // BP0-2, TB and SEC. Any of them set and the erase is ignored
constexpr uint8_t kStatus2Cmp = 0x40;

constexpr uint32_t kFlashSize = 2UL * 1024 * 1024;
constexpr uint32_t kEraseTimeoutMs = 30000;   // datasheet max 25 s
constexpr uint8_t kCountdownS = 5;

void select(bool on)
{
  digitalWrite(kCsPin, on ? LOW : HIGH);
}

uint8_t readStatus(uint8_t code)
{
  flashSpi.beginTransaction(kSpiSettings);
  select(true);
  flashSpi.transfer(code);
  const uint8_t value = flashSpi.transfer(0x00);
  select(false);
  flashSpi.endTransaction();
  return value;
}

bool busy()
{
  return (readStatus(kCmdStatus1) & kStatusBusy) != 0;
}

void command(uint8_t code)
{
  flashSpi.beginTransaction(kSpiSettings);
  select(true);
  flashSpi.transfer(code);
  select(false);
  flashSpi.endTransaction();
}

void readBytes(uint32_t address, uint8_t *buffer, size_t length)
{
  flashSpi.beginTransaction(kSpiSettings);
  select(true);
  flashSpi.transfer(kCmdRead);
  flashSpi.transfer(static_cast<uint8_t>(address >> 16));
  flashSpi.transfer(static_cast<uint8_t>(address >> 8));
  flashSpi.transfer(static_cast<uint8_t>(address));
  for (size_t i = 0; i < length; i++) {
    buffer[i] = flashSpi.transfer(0x00);
  }
  select(false);
  flashSpi.endTransaction();
}

void readId(uint8_t *id)
{
  flashSpi.beginTransaction(kSpiSettings);
  select(true);
  flashSpi.transfer(kCmdJedecId);
  for (uint8_t i = 0; i < 3; i++) {
    id[i] = flashSpi.transfer(0x00);
  }
  select(false);
  flashSpi.endTransaction();
}

void printHexByte(uint8_t value)
{
  if (value < 0x10) {
    console.print('0');
  }
  console.print(value, HEX);
}

void setup()
{
  console.begin(115200);
  delay(300);
  console.println();

  pinMode(kCsPin, OUTPUT);
  select(false);
  flashSpi.begin();

  // Resetting the MCU doesn't reset the chip, and the flight software powers it down when it pauses.
  // A powered down chip ignores everything, so this has to come first
  command(kCmdReleasePowerDown);
  delayMicroseconds(5);
  while (busy()) {
    delay(10);
  }

  uint8_t id[3];
  readId(id);
  console.print("jedec ");
  for (uint8_t i = 0; i < 3; i++) {
    printHexByte(id[i]);
    console.print(' ');
  }
  console.println();
  if (id[0] != 0xEF || id[1] != 0x40 || id[2] != 0x15) {
    console.println("that isn't the W25Q16JV-IQ, nothing erased");
    return;
  }

  const uint8_t sr1 = readStatus(kCmdStatus1);
  const uint8_t sr2 = readStatus(kCmdStatus2);
  console.print("status sr1=0x");
  printHexByte(sr1);
  console.print(" sr2=0x");
  printHexByte(sr2);
  console.println();
  if ((sr1 & kStatusBp) != 0 || (sr2 & kStatus2Cmp) != 0) {
    console.println("write protected, a chip erase would be ignored. Nothing erased");
    return;
  }

  console.println("about to erase ALL of it");
  for (uint8_t left = kCountdownS; left > 0; left--) {
    console.print(left);
    console.println(" - pull the power now if this is wrong");
    delay(1000);
  }

  command(kCmdWriteEnable);
  if ((readStatus(kCmdStatus1) & kStatusWel) == 0) {
    console.println("the chip didn't take the write enable, nothing erased");
    return;
  }

  console.println("erasing, 5 s typical, 25 s at most");
  const uint32_t startMs = millis();
  command(kCmdChipErase);
  while (busy() && millis() - startMs < kEraseTimeoutMs) {
    delay(100);
  }
  const uint32_t tookMs = millis() - startMs;
  if (busy()) {
    console.print("still busy after ");
    console.print(tookMs);
    console.println(" ms, giving up. The chip may be part erased");
    return;
  }
  console.print("erased in ");
  console.print(tookMs);
  console.println(" ms, checking every byte. This takes a while");

  // The status bit only says the erase finished, not that it worked, so read the whole chip back
  uint8_t buffer[256];
  uint32_t bad = 0;
  uint32_t firstBad = 0;
  for (uint32_t address = 0; address < kFlashSize; address += sizeof buffer) {
    readBytes(address, buffer, sizeof buffer);
    for (size_t i = 0; i < sizeof buffer; i++) {
      if (buffer[i] != 0xFF) {
        if (bad == 0) {
          firstBad = address + i;
        }
        bad++;
      }
    }
  }

  // all FF also looks like the chip falling off the bus, so make sure it's still answering
  readId(id);
  if (id[0] != 0xEF) {
    console.println("the chip stopped answering during the read back, the check means nothing");
    return;
  }

  if (bad == 0) {
    console.println("all 2097152 bytes are FF, the chip is empty");
  } else {
    console.print(bad);
    console.print(" bytes are not FF, first at 0x");
    console.println(firstBad, HEX);
  }
  console.println("put the flight software back with ./build.sh flash");
}

void loop()
{
}
