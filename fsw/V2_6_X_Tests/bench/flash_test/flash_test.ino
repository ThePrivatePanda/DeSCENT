// Is the W25Q16JV there and does it work: reads the ids and status, erases and writes the last sector, times it, then
// re-checks once a second. Console PB7/PB6 at 115200. Run it twice with a power cycle in between, the second boot
// should find last run's pattern still there

#include <SPI.h>

HardwareSerial console(PB7, PB6);
SPIClass flashSpi(PA10, PB14, PB13);   // mosi, miso, sck. SPI2, the default SPI object is on other pins

constexpr uint8_t kCsPin = PB9;   // plain gpio, R18 pulls it high while the MCU is in reset
const SPISettings kSpiSettings(4000000, MSBFIRST, SPI_MODE0);   // the core rounds this down to 48 MHz / 16 = 3 MHz

constexpr uint8_t kCmdWriteEnable = 0x06;
constexpr uint8_t kCmdReadStatus1 = 0x05;
constexpr uint8_t kCmdReadStatus2 = 0x35;
constexpr uint8_t kCmdReadStatus3 = 0x15;
constexpr uint8_t kCmdRead = 0x03;
constexpr uint8_t kCmdPageProgram = 0x02;
constexpr uint8_t kCmdSectorErase = 0x20;
constexpr uint8_t kCmdJedecId = 0x9F;
constexpr uint8_t kCmdUniqueId = 0x4B;
constexpr uint8_t kCmdPowerDown = 0xB9;
constexpr uint8_t kCmdReleasePowerDown = 0xAB;

constexpr uint8_t kStatusBusy = 0x01;
constexpr uint8_t kStatusWel = 0x02;

// Last 4 kB sector, so the start of the chip (where an old test may have left data, and where the log will go) is
// never touched
constexpr uint32_t kTestSector = 0x1FF000;
constexpr uint32_t kEraseTimeoutUs = 1000000;   // datasheet max 400 ms
constexpr uint32_t kProgramTimeoutUs = 10000;   // datasheet max 3 ms

uint8_t pattern[512];
uint8_t readBack[512];
bool writesAllowed = false;
uint32_t loopFailures = 0;

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

uint8_t readStatus(uint8_t cmd)
{
  select();
  flashSpi.transfer(cmd);
  const uint8_t value = flashSpi.transfer(0x00);
  deselect();
  return value;
}

void sendAddress(uint32_t address)
{
  flashSpi.transfer(static_cast<uint8_t>(address >> 16));
  flashSpi.transfer(static_cast<uint8_t>(address >> 8));
  flashSpi.transfer(static_cast<uint8_t>(address));
}

void readBytes(uint32_t address, uint8_t *data, uint16_t length)
{
  select();
  flashSpi.transfer(kCmdRead);
  sendAddress(address);
  for (uint16_t i = 0; i < length; ++i) {
    data[i] = flashSpi.transfer(0x00);
  }
  deselect();
}

void readJedecId(uint8_t id[3])
{
  select();
  flashSpi.transfer(kCmdJedecId);
  for (uint8_t i = 0; i < 3; ++i) {
    id[i] = flashSpi.transfer(0x00);
  }
  deselect();
}

// Busy time is measured from /CS going high, which is what the datasheet's tSE and tPP are
bool waitWhileBusy(uint32_t timeoutUs, uint32_t &elapsedUs)
{
  const uint32_t startUs = micros();
  while (true) {
    elapsedUs = micros() - startUs;
    if ((readStatus(kCmdReadStatus1) & kStatusBusy) == 0) {
      return true;
    }
    if (elapsedUs > timeoutUs) {
      return false;
    }
  }
}

// false if the chip didn't set WEL, then an erase or program would be ignored without any error
bool writeEnable()
{
  command(kCmdWriteEnable);
  return (readStatus(kCmdReadStatus1) & kStatusWel) != 0;
}

bool eraseSector(uint32_t address, uint32_t &elapsedUs)
{
  if (!writeEnable()) {
    elapsedUs = 0;
    return false;
  }
  select();
  flashSpi.transfer(kCmdSectorErase);
  sendAddress(address);
  deselect();
  return waitWhileBusy(kEraseTimeoutUs, elapsedUs);
}

// Stays inside one 256 byte page, the chip wraps to the page start otherwise
bool programBytes(uint32_t address, const uint8_t *data, uint16_t length, uint32_t &elapsedUs)
{
  if (!writeEnable()) {
    elapsedUs = 0;
    return false;
  }
  select();
  flashSpi.transfer(kCmdPageProgram);
  sendAddress(address);
  for (uint16_t i = 0; i < length; ++i) {
    flashSpi.transfer(data[i]);
  }
  deselect();
  return waitWhileBusy(kProgramTimeoutUs, elapsedUs);
}

void printHexByte(uint8_t value)
{
  if (value < 0x10) {
    console.print('0');
  }
  console.print(value, HEX);
}

void printBytes(const uint8_t *data, uint16_t length)
{
  for (uint16_t i = 0; i < length; ++i) {
    printHexByte(data[i]);
    console.print(i + 1 < length ? " " : "\n");
  }
}

void printMs(const char *label, uint32_t elapsedUs)
{
  console.print(label);
  console.print(elapsedUs / 1000.0f, 2);
  console.print(" ms");
}

uint16_t countNotErased(const uint8_t *data, uint16_t length)
{
  uint16_t count = 0;
  for (uint16_t i = 0; i < length; ++i) {
    if (data[i] != 0xFF) {
      ++count;
    }
  }
  return count;
}

uint16_t countMismatches(const uint8_t *a, const uint8_t *b, uint16_t length)
{
  uint16_t count = 0;
  for (uint16_t i = 0; i < length; ++i) {
    if (a[i] != b[i]) {
      ++count;
    }
  }
  return count;
}

void printStatus()
{
  const uint8_t sr1 = readStatus(kCmdReadStatus1);
  const uint8_t sr2 = readStatus(kCmdReadStatus2);
  const uint8_t sr3 = readStatus(kCmdReadStatus3);
  console.print("status sr1=0x");
  printHexByte(sr1);
  console.print(" sr2=0x");
  printHexByte(sr2);
  console.print(" sr3=0x");
  printHexByte(sr3);
  console.print(" busy=");
  console.print(sr1 & kStatusBusy);
  console.print(" wel=");
  console.print((sr1 >> 1) & 1);
  console.print(" bp=");
  console.print((sr1 >> 2) & 0x07);   // not 0 means part of the array is write protected
  console.print(" tb=");
  console.print((sr1 >> 5) & 1);
  console.print(" sec=");
  console.print((sr1 >> 6) & 1);
  console.print(" srp=");
  console.print((sr1 >> 7) & 1);
  console.print(" qe=");
  console.print((sr2 >> 1) & 1);   // 1 and fixed on IQ parts, which is what disables /HOLD
  console.print(" cmp=");
  console.print((sr2 >> 6) & 1);
  console.print(" sus=");
  console.print((sr2 >> 7) & 1);
  console.print(" wps=");
  console.println((sr3 >> 2) & 1);   // 1 would lock every block at power up
}

// Returns true when it is safe to go on and write
bool checkId()
{
  uint8_t id[3];
  readJedecId(id);
  console.print("jedec ");
  printBytes(id, 3);

  if (id[0] == 0xEF && id[1] == 0x40 && id[2] == 0x15) {
    console.println("W25Q16JV-IQ, as on the schematic");
    return true;
  }
  if (id[0] == 0xEF && id[1] == 0x70 && id[2] == 0x15) {
    console.println("W25Q16JV-IM, not the schematic part. QE is 0 here so /HOLD works, and it floats on this board");
    return true;
  }
  if ((id[0] == 0xFF && id[1] == 0xFF && id[2] == 0xFF) || (id[0] == 0 && id[1] == 0 && id[2] == 0)) {
    console.println("nothing answering. All FF: MISO not driven (no chip, no power, /CS or CLK not reaching it). All 00: MISO held low");
    return false;
  }
  console.println("unexpected id, not writing");
  return false;
}

void setup()
{
  console.begin(115200);
  delay(300);   // also covers the 5 ms after power up before the chip takes a write
  console.println();

  pinMode(kCsPin, OUTPUT);
  digitalWrite(kCsPin, HIGH);
  flashSpi.begin();

  // In case something left it powered down, it ignores everything else then
  command(kCmdReleasePowerDown);
  delayMicroseconds(5);

  writesAllowed = checkId();
  printStatus();

  uint8_t uniqueId[8];
  select();
  flashSpi.transfer(kCmdUniqueId);
  for (uint8_t i = 0; i < 4; ++i) {
    flashSpi.transfer(0x00);
  }
  for (uint8_t i = 0; i < 8; ++i) {
    uniqueId[i] = flashSpi.transfer(0x00);
  }
  deselect();
  console.print("unique id ");
  printBytes(uniqueId, 8);

  // Read only. An older test sketch wrote 00 01 02 ... to address 0, if it's there that test ran on this board
  uint8_t start[32];
  readBytes(0, start, sizeof start);
  console.print("address 0: ");
  printBytes(start, sizeof start);

  for (uint16_t i = 0; i < sizeof pattern; ++i) {
    pattern[i] = static_cast<uint8_t>(i ^ 0x5A);
  }

  readBytes(kTestSector, readBack, sizeof readBack);
  console.print("test sector before: not erased=");
  console.print(countNotErased(readBack, sizeof readBack));
  console.print(" of 512, pattern from last run=");
  console.println(countMismatches(readBack, pattern, sizeof pattern) == 0);

  if (!writesAllowed) {
    console.println("skipping erase and write");
    return;
  }

  uint32_t elapsedUs = 0;
  const bool erased = eraseSector(kTestSector, elapsedUs);
  printMs(erased ? "erase ok " : "erase FAILED (write enable not taken or busy timeout) ", elapsedUs);
  console.println(" (datasheet 45 typ, 400 max)");

  // Whole sector should read FF now, read in 512 byte pieces
  uint16_t notErased = 0;
  uint32_t readUs = 0;
  for (uint32_t offset = 0; offset < 4096; offset += sizeof readBack) {
    const uint32_t startUs = micros();
    readBytes(kTestSector + offset, readBack, sizeof readBack);
    readUs += micros() - startUs;
    notErased += countNotErased(readBack, sizeof readBack);
  }
  console.print("after erase: not erased=");
  console.print(notErased);
  console.print(" of 4096, ");
  printMs("read 4 kB in ", readUs);
  console.println();

  // First page in one program, second page as four 64 byte programs the way a log would add records
  bool programmed = programBytes(kTestSector, pattern, 256, elapsedUs);
  printMs(programmed ? "program 256 B ok " : "program 256 B FAILED ", elapsedUs);
  console.println(" (datasheet 0.4 typ, 3 max)");
  for (uint16_t slot = 0; slot < 4; ++slot) {
    const uint16_t offset = 256 + slot * 64;
    programmed = programBytes(kTestSector + offset, &pattern[offset], 64, elapsedUs);
    printMs(programmed ? "program 64 B ok " : "program 64 B FAILED ", elapsedUs);
    console.println();
  }

  readBytes(kTestSector, readBack, sizeof readBack);
  console.print("read back: mismatches=");
  console.print(countMismatches(readBack, pattern, sizeof pattern));
  console.print(" of 512, first 16: ");
  printBytes(readBack, 16);

  // Power down should make the chip ignore the id read (all FF), release should bring it back
  command(kCmdPowerDown);
  delayMicroseconds(5);
  uint8_t id[3];
  readJedecId(id);
  console.print("powered down, jedec ");
  printBytes(id, 3);
  command(kCmdReleasePowerDown);
  delayMicroseconds(5);
  readJedecId(id);
  console.print("released, jedec ");
  printBytes(id, 3);

  printStatus();
  console.println("seconds  jedec  sr1  mismatches  failures");
}

void loop()
{
  uint8_t id[3];
  readJedecId(id);
  const uint8_t sr1 = readStatus(kCmdReadStatus1);
  readBytes(kTestSector, readBack, sizeof readBack);
  const uint16_t mismatches = writesAllowed ? countMismatches(readBack, pattern, sizeof pattern) : 0;
  const bool idOk = id[0] == 0xEF && id[2] == 0x15;
  if (!idOk || mismatches != 0) {
    ++loopFailures;
  }

  console.print(millis() / 1000);
  console.print("  ");
  printHexByte(id[0]);
  printHexByte(id[1]);
  printHexByte(id[2]);
  console.print("  ");
  printHexByte(sr1);
  console.print("  ");
  console.print(mismatches);
  console.print("  ");
  console.println(loopFailures);

  delay(1000);
}
