// Talks to the BNO085 the way our driver does: reset, product id, four reports at 50 ms, then prints
// the latest of each once a second. At boot it also prints the report lengths from the IMU's own
// advertisement, to check the table in ImuSh2.cpp. Console PB7/PB6 at 115200.
// kRestartTest true: at 10 s it resets the IMU and replays the driver's background restart at the
// flight loop's pace, printing each packet's channel and ms after the reset.

#include <Wire.h>

HardwareSerial console(PB7, PB6);

constexpr uint8_t kAddress = 0x4A;
constexpr uint32_t kIntervalUs = 50000;
const uint8_t kReportIds[4] = {0x04, 0x02, 0x03, 0x05};   // linaccel, gyro, mag, rotation vector
const uint8_t kQ[4] = {8, 9, 4, 14};

constexpr bool kRestartTest = true;
constexpr uint32_t kCycleMs = 650;   // one flight cycle with the IMU lost, LE, SF9 (about)
bool restartTested = false;
bool tracing = false;
uint32_t resetSentMs = 0;

uint8_t packet[384];
uint16_t packetLength = 0;
uint8_t seq[4] = {0, 0, 0, 0};

float values[4][4];
uint32_t counts[4];
uint32_t granted[4];
uint32_t sentMs[4];
int32_t replyMs[4] = {-1, -1, -1, -1};   // -1 = no 0xFC reply
bool resetDone = false;
bool sh2Init = false;
bool productIdShown = false;

bool readBytes(uint8_t *buffer, uint8_t length)
{
  if (Wire.requestFrom(kAddress, length) != length) {
    return false;
  }
  for (uint8_t i = 0; i < length; i++) {
    buffer[i] = Wire.read();
  }
  return true;
}

// header first, then the whole packet again in 32-byte chunks, each after the first with its own header
bool readPacket()
{
  packetLength = 0;
  uint8_t header[4];
  if (!readBytes(header, 4)) {
    return false;
  }
  const uint16_t raw = header[0] | header[1] << 8;
  const uint16_t length = raw & 0x7FFF;
  if (length < 4 || length > sizeof packet) {
    return false;
  }
  // continuation = the rest of a packet an earlier failed read gave up on. read it out, don't use it
  const bool continuation = raw & 0x8000;
  uint8_t chunk[32];
  uint16_t stored = 0;
  bool first = true;
  while (stored < length) {
    const uint16_t wanted = first ? length : length - stored + 4;
    const uint8_t size = wanted < 32 ? wanted : 32;
    if (!readBytes(chunk, size)) {
      return false;
    }
    const uint8_t skip = first ? 0 : 4;
    memcpy(&packet[stored], &chunk[skip], size - skip);
    stored += size - skip;
    first = false;
  }
  if (continuation) {
    console.println("dropped a continuation");
    return false;
  }
  packetLength = length;
  return true;
}

bool send(uint8_t channel, const uint8_t *payload, uint8_t length)
{
  Wire.beginTransmission(kAddress);
  Wire.write(length + 4);
  Wire.write(0);
  Wire.write(channel);
  Wire.write(seq[channel]++);
  Wire.write(payload, length);
  return Wire.endTransmission() == 0;
}

float fixedPoint(const uint8_t *p, uint8_t q)
{
  return (int16_t)(p[0] | p[1] << 8) / (float)(1UL << q);
}

uint32_t u32(const uint8_t *p)
{
  return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}

// advertisement: tag, length, value. Tag 0x81 is the sensor hub's report length table, id then length
void printLengths(const uint8_t *cargo, uint16_t length)
{
  uint16_t at = 1;
  while (at + 2 <= length) {
    const uint8_t tag = cargo[at];
    const uint8_t size = cargo[at + 1];
    if (tag == 0x81) {
      console.print("lengths");
      for (uint8_t i = 0; i + 1 < size; i += 2) {
        console.print(" ");
        console.print(cargo[at + 2 + i], HEX);
        console.print(":");
        console.print(cargo[at + 3 + i]);
      }
      console.println();
    }
    at += 2 + size;
  }
}

void handle()
{
  const uint8_t channel = packet[2];
  const uint8_t *cargo = &packet[4];
  const uint16_t length = packetLength - 4;

  if (tracing) {
    console.print("packet channel=");
    console.print(channel);
    console.print(" length=");
    console.print(length);
    console.print(" ms=");
    console.println(millis() - resetSentMs);
  }

  if (channel == 0) {
    memset(seq, 0, sizeof seq);   // the advertisement, sequence numbers start over like the driver's
    printLengths(cargo, length);
  } else if (channel == 1 && length == 1 && cargo[0] == 1) {
    resetDone = true;
  } else if (channel == 2) {
    for (uint16_t at = 0; at < length;) {
      if (cargo[at] == 0xFC && at + 17 <= length) {
        for (uint8_t i = 0; i < 4; i++) {
          if (cargo[at + 1] == kReportIds[i]) {
            granted[i] = u32(&cargo[at + 5]);
            replyMs[i] = millis() - sentMs[i];
          }
        }
        at += 17;
      } else if ((cargo[at] == 0xF8 || cargo[at] == 0xF1) && at + 16 <= length) {
        if (cargo[at] == 0xF1 && cargo[at + 2] == 0x84 && cargo[at + 6] == 1) {
          sh2Init = true;   // the sensor hub's own init message after a reset
        }
        if (cargo[at] == 0xF8 && !productIdShown) {
          console.print("product part=");
          console.print(u32(&cargo[at + 4]));
          console.print(" version=");
          console.print(cargo[at + 2]);
          console.print(".");
          console.print(cargo[at + 3]);
          console.print(".");
          console.print(cargo[at + 12] | cargo[at + 13] << 8);
          console.print(" build=");
          console.print(u32(&cargo[at + 8]));
          console.print(" reset_cause=");
          console.println(cargo[at + 1]);
          productIdShown = true;
        }
        at += 16;
      } else {
        break;
      }
    }
  } else if (channel == 3) {
    for (uint16_t at = 0; at < length;) {
      const uint8_t id = cargo[at];
      const uint8_t size = (id == 0xFB || id == 0xFA) ? 5 : (id == 0x05) ? 14 : (id >= 0x02 && id <= 0x04) ? 10 : 0;
      if (size == 0 || at + size > length) {
        console.print("unknown report 0x");
        console.println(id, HEX);
        break;
      }
      for (uint8_t i = 0; i < 4; i++) {
        if (id == kReportIds[i]) {
          for (uint8_t v = 0; v < (id == 0x05 ? 4 : 3); v++) {
            values[i][v] = fixedPoint(&cargo[at + 4 + 2 * v], kQ[i]);
          }
          counts[i]++;
        }
      }
      at += size;
    }
  }
}

// read and handle packets for this long
void serviceFor(uint32_t ms)
{
  const uint32_t start = millis();
  while (millis() - start < ms) {
    if (readPacket()) {
      handle();
    } else {
      delay(2);
    }
  }
}

// ImuSh2.cpp restartStep(): reset, then up to 8 packets per cycle until SH-2 init (or reset complete
// plus 1000 ms), 2500 ms limit, then one Set Feature per cycle with no reads in between
void restartTest()
{
  memset(seq, 0, sizeof seq);
  resetDone = false;
  sh2Init = false;
  tracing = true;
  const uint8_t reset = 1;
  resetSentMs = millis();
  console.print("restart reset sent=");
  console.println(send(1, &reset, 1));

  bool up = false;
  while (millis() - resetSentMs < 2500) {
    delay(kCycleMs);
    for (uint8_t read = 0; read < 8 && !sh2Init; read++) {
      if (!readPacket()) {
        break;
      }
      handle();
    }
    if (sh2Init || (resetDone && millis() - resetSentMs >= 1000)) {
      up = true;
      break;
    }
  }
  console.print("restart up=");
  console.print(up);
  console.print(" reset_complete=");
  console.print(resetDone);
  console.print(" sh2_init=");
  console.print(sh2Init);
  console.print(" ms=");
  console.println(millis() - resetSentMs);

  for (uint8_t i = 0; i < 4; i++) {
    delay(kCycleMs);
    uint8_t command[17] = {0xFD, kReportIds[i]};
    command[5] = kIntervalUs & 0xFF;
    command[6] = (kIntervalUs >> 8) & 0xFF;
    command[7] = (kIntervalUs >> 16) & 0xFF;
    sentMs[i] = millis();
    send(2, command, sizeof command);
  }
  serviceFor(500);
  tracing = false;
  console.print("restart reply_ms");
  for (uint8_t i = 0; i < 4; i++) {
    console.print(" ");
    console.print(replyMs[i]);
  }
  console.println("   counts below should keep going up");
}

void setup()
{
  console.begin(115200);
  delay(300);
  console.println();

  Wire.setSDA(PA15);
  Wire.setSCL(PB15);
  Wire.begin();

  const uint8_t reset = 1;
  console.print("reset sent=");
  console.println(send(1, &reset, 1));
  const uint32_t start = millis();
  while (!resetDone && millis() - start < 1500) {
    serviceFor(5);
  }
  console.print("reset_complete=");
  console.print(resetDone);
  console.print(" ms=");
  console.println(millis() - start);
  while (!sh2Init && millis() - start < 2500) {
    serviceFor(5);
  }
  console.print("sh2_init=");
  console.print(sh2Init);
  console.print(" ms=");
  console.println(millis() - start);

  const uint8_t productRequest[2] = {0xF9, 0};
  send(2, productRequest, 2);
  serviceFor(100);
  if (!productIdShown) {
    console.println("no product id");
  }

  for (uint8_t i = 0; i < 4; i++) {
    uint8_t command[17] = {0xFD, kReportIds[i]};
    command[5] = kIntervalUs & 0xFF;
    command[6] = (kIntervalUs >> 8) & 0xFF;
    command[7] = (kIntervalUs >> 16) & 0xFF;
    sentMs[i] = millis();
    send(2, command, sizeof command);
    serviceFor(100);
  }
  console.print("granted_us");
  for (uint8_t i = 0; i < 4; i++) {
    console.print(" ");
    console.print(granted[i]);
  }
  console.println();
  console.print("reply_ms");
  for (uint8_t i = 0; i < 4; i++) {
    console.print(" ");
    console.print(replyMs[i]);
  }
  console.println();
  console.println("seconds  ax ay az  gx gy gz  mx my mz  qi qj qk qr  counts");
}

void loop()
{
  if (kRestartTest && !restartTested && millis() > 10000) {
    restartTested = true;
    for (uint8_t i = 0; i < 4; i++) {
      replyMs[i] = -1;
    }
    restartTest();
  }
  serviceFor(1000);

  console.print(millis() / 1000);
  for (uint8_t i = 0; i < 4; i++) {
    console.print(" ");
    for (uint8_t v = 0; v < (i == 3 ? 4 : 3); v++) {
      console.print(" ");
      console.print(values[i][v], i == 3 ? 4 : 3);
    }
  }
  console.print("  ");
  for (uint8_t i = 0; i < 4; i++) {
    console.print(counts[i]);
    console.print(i < 3 ? "," : "\n");
  }
}
