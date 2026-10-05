// Sets four of the MAX-M10S settings, then polls it once a second and prints the fix. Flight now streams
// NAV-PVT, gps_stream_test runs it that way. Works indoors: with no satellites it prints fix=0 and zeros.
// Console PB7/PB6 at 115200.

HardwareSerial console(PB7, PB6);
HardwareSerial gps(PC1, PC0);

// Dynamic model, rates and power mode as flight sets them, already framed and checksummed
const uint8_t kDynModel[] = {0xB5, 0x62, 0x06, 0x8A, 0x09, 0x00, 0x00, 0x03, 0x00, 0x00,
                             0x21, 0x00, 0x11, 0x20, 0x08, 0xF6, 0x61};
const uint8_t kRateMeas[] = {0xB5, 0x62, 0x06, 0x8A, 0x0A, 0x00, 0x00, 0x03, 0x00, 0x00,
                             0x01, 0x00, 0x21, 0x30, 0xE8, 0x03, 0xDA, 0xD6};
const uint8_t kRateNav[] = {0xB5, 0x62, 0x06, 0x8A, 0x0A, 0x00, 0x00, 0x03, 0x00, 0x00,
                            0x02, 0x00, 0x21, 0x30, 0x01, 0x00, 0xF1, 0x0B};
const uint8_t kFullPower[] = {0xB5, 0x62, 0x06, 0x8A, 0x09, 0x00, 0x00, 0x03, 0x00, 0x00,
                              0x01, 0x00, 0xD0, 0x20, 0x00, 0x8D, 0xF6};
const uint8_t kPollPvt[] = {0xB5, 0x62, 0x01, 0x07, 0x00, 0x00, 0x08, 0x19};

uint8_t payload[100];
uint16_t payloadLength = 0;
uint8_t gotClass = 0, gotId = 0;

// Reads one whole UBX frame, checksum verified, or gives up at the deadline
bool readFrame(uint32_t deadlineMs)
{
  uint8_t step = 0, ckA = 0, sumA = 0, sumB = 0;
  uint16_t length = 0, index = 0;

  while (millis() < deadlineMs) {
    const int next = gps.read();
    if (next < 0) {
      continue;
    }
    const uint8_t b = (uint8_t)next;

    switch (step) {
      case 0: step = (b == 0xB5) ? 1 : 0; break;
      case 1: step = (b == 0x62) ? 2 : 0; break;
      case 2: gotClass = b; sumA = b; sumB = b; step = 3; break;
      case 3: gotId = b; sumA += b; sumB += sumA; step = 4; break;
      case 4: length = b; sumA += b; sumB += sumA; step = 5; break;
      case 5:
        length |= (uint16_t)b << 8;
        sumA += b; sumB += sumA;
        index = 0;
        step = (length == 0) ? 7 : 6;
        break;
      case 6:
        if (index < sizeof payload) {
          payload[index] = b;
        }
        sumA += b; sumB += sumA;
        if (++index >= length) {
          step = 7;
        }
        break;
      case 7: ckA = b; step = 8; break;
      case 8:
        if (ckA == sumA && b == sumB && length <= sizeof payload) {
          payloadLength = length;
          return true;
        }
        step = 0;   // bad checksum or too big, start hunting again
        break;
    }
  }
  return false;
}

bool waitFor(uint8_t cls, uint8_t id, uint16_t waitMs)
{
  const uint32_t deadline = millis() + waitMs;
  while (millis() < deadline) {
    if (!readFrame(deadline)) {
      return false;
    }
    if (gotClass == cls && gotId == id) {
      return true;
    }
  }
  return false;
}

bool sendAndAck(const char *what, const uint8_t *frame, size_t length)
{
  while (gps.read() >= 0) {
  }
  gps.write(frame, length);
  const bool acked = waitFor(0x05, 0x01, 1100);
  console.print(what);
  console.println(acked ? " ok" : " no ack");
  return acked;
}

int32_t readS32(const uint8_t *from)
{
  return (int32_t)((uint32_t)from[0] | (uint32_t)from[1] << 8 | (uint32_t)from[2] << 16 |
                   (uint32_t)from[3] << 24);
}

void setup()
{
  console.begin(115200);
  gps.begin(9600);
  delay(500);
  console.println();
  console.println("gps_test");

  sendAndAck("airborne dynamic model", kDynModel, sizeof kDynModel);
  sendAndAck("measurement rate 1000 ms", kRateMeas, sizeof kRateMeas);
  sendAndAck("navigation rate 1", kRateNav, sizeof kRateNav);
  sendAndAck("full power mode", kFullPower, sizeof kFullPower);

  console.println();
  console.println("seconds  fix  sats  lat  lon  alt_m");
}

void loop()
{
  while (gps.read() >= 0) {
  }
  gps.write(kPollPvt, sizeof kPollPvt);

  console.print(millis() / 1000);
  console.print("  ");

  if (!waitFor(0x01, 0x07, 1200) || payloadLength != 92) {
    console.println("no position came back");
    delay(1000);
    return;
  }

  console.print(payload[20]);            // fix type: 0 none, 2 2D, 3 3D
  console.print("  ");
  console.print(payload[23]);            // satellites used
  console.print("  ");
  console.print(readS32(&payload[28]) / 1e7, 7);    // latitude
  console.print("  ");
  console.print(readS32(&payload[24]) / 1e7, 7);    // longitude
  console.print("  ");
  console.println(readS32(&payload[36]) / 1000.0, 2);   // height above sea level

  delay(1000);
}
