// Ground only. Prints the flash log as the raw 64 byte slots: "raw <index> <addr> <128 hex>", one a line.
// Bytes 62-63 of a slot are its own CRC16 over 0-61, so the index, boot count and uptime are checked by the
// number the flight software wrote - the readable lines this replaced had them outside any check, and a
// character lost on the wire read as a real timestamp. Slots the CRC rejects are printed too, so a reader can
// compare its own count against the "found" line. Stops at the first erased record. Send D to print it again,
// E then Y to erase the whole chip before a flight. Console PB7/PB6 at 115200. The ground software
// puts this sketch on a board and reads it back itself: the ChipSats screen's pull

#include <stdio.h>
#include "../../../V2_6_X_Code/src/platform/arduino/SystemArduino.cpp"
#include "../../../V2_6_X_Code/src/platform/arduino/UartArduino.cpp"
#include "../../../V2_6_X_Code/src/log/Log.cpp"
#include "../../../V2_6_X_Code/src/app/Telemetry.cpp"
#include "../../../V2_6_X_Code/src/app/FlashLog.cpp"
#include "../../../V2_6_X_Code/src/devices/own/FlashW25Q.cpp"

using ChipSatPlatform::Status;
using ChipSatApp::kLogRecordSize;

ChipSatPlatform::System chipSystem;
ChipSatPlatform::Uart console(ChipSatPlatform::UartPort::Console);
ChipSatDevices::Flash flash(chipSystem);

constexpr uint32_t kConfirmWaitMs = 10000;

bool flashReady = false;

void printHex(const uint8_t *bytes, size_t length)
{
  char text[3];
  for (size_t i = 0; i < length; ++i) {
    snprintf(text, sizeof text, "%02X", bytes[i]);
    console.write(text);
  }
}

// Prints the log. Returns true if an E arrived while it was printing: the ground software clears a
// chip by sending E as soon as the board boots, and a full chip would otherwise print for seven
// minutes before the E was read
bool dump()
{
  char text[160];   // the found line is 120 with a full chip, and a short buffer cuts it off
  uint32_t good = 0;
  uint32_t anchors = 0;
  uint32_t bad = 0;
  uint32_t boots = 0;
  int32_t lastBoot = -1;
  uint32_t index = 0;
  for (; index < ChipSatApp::kLogRecordCount; ++index) {
    if ((index & 31) == 0) {
      const int c = console.read();
      if (c == 'E' || c == 'e') {
        console.write("\r\nstopped printing: E received\r\n");
        return true;
      }
    }
    uint8_t bytes[kLogRecordSize];
    const Status status = flash.read(index * kLogRecordSize, bytes, sizeof bytes);
    if (status != Status::Ok) {
      snprintf(text, sizeof text, "read failed at record %lu (%s), stopping here\r\n", static_cast<unsigned long>(index),
               ChipSatPlatform::statusName(status));
      console.write(text);
      break;
    }
    if (ChipSatApp::recordErased(bytes)) {
      break;
    }

    // The whole 64 byte slot, as it sits on the chip. Bytes 62-63 are the record's own CRC16 over
    // 0-61, so the index, boot count and uptime are checked by the same number the flight software
    // wrote. The readable lines this replaced put those three fields outside any check, and a digit
    // lost on the way up the wire then read as a real timestamp - one 9 digit uptime in the 09-29
    // pull dated a boot 152 hours long. Anchors come up the same path, so there is one format
    ChipSatApp::LogRecord record;
    const bool okay = ChipSatApp::decodeRecord(bytes, record);
    if (okay) {
      ++good;
      if (record.bootCount != lastBoot) {
        ++boots;
        lastBoot = record.bootCount;
      }
      if (record.type == ChipSatApp::kLogTypeTime) {
        ++anchors;
      }
    } else {
      ++bad;   // cut off by a reset while it was being written, or damaged on the way here
    }
    snprintf(text, sizeof text, "raw %lu %06lX ", static_cast<unsigned long>(index),
             static_cast<unsigned long>(index * kLogRecordSize));
    console.write(text);
    printHex(bytes, sizeof bytes);
    console.write("\r\n");
  }

  snprintf(text, sizeof text, "found %lu records, %lu good (%lu time anchors), %lu bad, from %lu boots, log ends at address 0x%06lX\r\n",
           static_cast<unsigned long>(good + bad), static_cast<unsigned long>(good),
           static_cast<unsigned long>(anchors), static_cast<unsigned long>(bad),
           static_cast<unsigned long>(boots), static_cast<unsigned long>(index * kLogRecordSize));
  console.write(text);
  if (index == ChipSatApp::kLogRecordCount) {
    console.write("the chip is full\r\n");
  }
  return false;
}

// Skips the line ending that came with the E, anything else cancels
int waitForAnswer()
{
  const uint32_t startMs = chipSystem.nowMs();
  while (chipSystem.nowMs() - startMs < kConfirmWaitMs) {
    const int c = console.read();
    if (c >= 0 && c != '\r' && c != '\n') {
      return c;
    }
    chipSystem.waitMs(10);
  }
  return -1;
}

void eraseAll()
{
  console.write("erase the WHOLE chip? the log on it is gone for good. send Y to go ahead, anything else cancels\r\n");
  if (waitForAnswer() != 'Y') {
    console.write("cancelled, nothing erased\r\n");
    return;
  }
  console.write("erasing, 5 s typical, 25 s at most\r\n");
  const uint32_t startMs = chipSystem.nowMs();
  const Status status = flash.eraseChip();
  char text[64];
  snprintf(text, sizeof text, "erase %s after %lu ms\r\n", ChipSatPlatform::statusName(status),
           static_cast<unsigned long>(chipSystem.nowMs() - startMs));
  console.write(text);
}

void prompt()
{
  console.write("send D to print the log again, E to erase the whole chip\r\n");
}

void setup()
{
  console.begin(ChipSatConfig::kConsoleBaud);
  ChipSatLog::begin(console, chipSystem);
  LOG_I(Boot, "start") { line.field("test", "flashdump"); }

  flashReady = flash.begin() == Status::Ok;
  if (!flashReady) {
    return;   // the init line says why
  }
  if (dump()) {
    eraseAll();
  }
  prompt();
}

void loop()
{
  if (!flashReady) {
    return;
  }
  const int c = console.read();
  if (c == 'D' || c == 'd') {
    if (dump()) {
      eraseAll();
    }
    prompt();
  } else if (c == 'E' || c == 'e') {
    eraseAll();
    prompt();
  }
  chipSystem.waitMs(10);
}
