// Which W25Q16JV is fitted: IQ or IM. Reads the jedec id and says which, nothing else.
// IM needs /WP and /HOLD driven, and on this board they float. Console PB7/PB6 at 115200

#include <SPI.h>

HardwareSerial console(PB7, PB6);
SPIClass flashSpi(PA10, PB14, PB13);   // mosi, miso, sck. SPI2

constexpr uint8_t kCsPin = PB9;
const SPISettings kSpiSettings(4000000, MSBFIRST, SPI_MODE0);
constexpr uint8_t kCmdJedecId = 0x9F;

void setup()
{
  console.begin(115200);
  delay(300);
  console.println();

  pinMode(kCsPin, OUTPUT);
  digitalWrite(kCsPin, HIGH);
  flashSpi.begin();

  uint8_t id[3];
  flashSpi.beginTransaction(kSpiSettings);
  digitalWrite(kCsPin, LOW);
  flashSpi.transfer(kCmdJedecId);
  for (uint8_t i = 0; i < 3; i++) {
    id[i] = flashSpi.transfer(0x00);
  }
  digitalWrite(kCsPin, HIGH);
  flashSpi.endTransaction();

  console.print("jedec ");
  for (uint8_t i = 0; i < 3; i++) {
    if (id[i] < 0x10) {
      console.print('0');
    }
    console.print(id[i], HEX);
    console.print(' ');
  }
  console.println();

  if (id[0] == 0xEF && id[1] == 0x40 && id[2] == 0x15) {
    console.println("IQ");
  } else if (id[0] == 0xEF && id[1] == 0x70 && id[2] == 0x15) {
    console.println("IM");
  } else {
    console.println("neither, no flash answering");
  }
}

void loop()
{
}
