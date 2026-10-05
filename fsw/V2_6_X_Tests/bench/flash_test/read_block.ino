#include <SPI.h>

HardwareSerial console(PB7, PB6);
SPIClass flashSPI(PA10, PB14, PB13);

const uint8_t kCsPin = PB9;

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

uint8_t readStatus(uint8_t cmd) {
    select();
    flashSpi.transfer(cmd);
    
}
