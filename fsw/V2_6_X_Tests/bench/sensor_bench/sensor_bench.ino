// Checks a board's parts: every sensor at its own rate on the console, flight watchdogs as on the
// ground, packets back to back as CSID 254 after the first 30 s (R toggles the radio). No phases, no
// flash log. Console PB7/PB6 at 115200.
// ./build.sh build --sketch sensor_bench -DCHIPSAT_RADIO_MODULE=1 (LE) or =2 (HP)

#ifndef CHIPSAT_RADIO_MODULE
#define CHIPSAT_RADIO_MODULE 1   // 1 LE, 2 HP, from the module label
#endif

// 254 is no board (255 means unconfigured). Lowest PA row of each module (constants.h), the receiver
// is a metre away and it loads a supply under test less
#if defined(CHIPSAT_ID) || defined(CHIPSAT_TARGET_DBM) || defined(CHIPSAT_SPREADING_FACTOR)
#error "the bench sets its own id, power and spreading factor"
#endif
#define CHIPSAT_ID 254
#define CHIPSAT_SPREADING_FACTOR 7
#if CHIPSAT_RADIO_MODULE == 2
#define CHIPSAT_TARGET_DBM 14
#else
#define CHIPSAT_TARGET_DBM 10
#endif

#include "../../../V2_6_X_Code/src/platform/arduino/SystemArduino.cpp"
#include "../../../V2_6_X_Code/src/platform/arduino/UartArduino.cpp"
#include "../../../V2_6_X_Code/src/platform/I2cWatch.cpp"
#include "../../../V2_6_X_Code/src/platform/arduino/I2cBusArduino.cpp"
#include "../../../V2_6_X_Code/src/log/Log.cpp"
#include "../../../V2_6_X_Code/src/app/Telemetry.cpp"
// For the radio's active(). begin() is never called, so the image has no settings block to rewrite
#include "../../../V2_6_X_Code/src/Settings.cpp"
#include "../../../V2_6_X_Code/src/devices/own/ImuSh2.cpp"
#include "../../../V2_6_X_Code/src/devices/own/GpsMaxM10s.cpp"
#include "../../../V2_6_X_Code/src/devices/own/EnvSensorBme280.cpp"
#include "../../../V2_6_X_Code/src/devices/own/FuelGaugeMax17048.cpp"
// flash and GPS drivers both have a file static called frame, and here they're one file
#define frame flashFrame
#include "../../../V2_6_X_Code/src/devices/own/FlashW25Q.cpp"
#undef frame
#include "../../../V2_6_X_Code/src/devices/own/RadioSubGhz.cpp"
#include "../../../V2_6_X_Code/src/app/SensorWatch.cpp"
#include "../../../V2_6_X_Code/src/app/Health.cpp"

using ChipSatPlatform::Status;
using namespace ChipSatConfig;
using namespace ChipSatConstants;

ChipSatPlatform::System chipSystem;
ChipSatPlatform::I2cBus i2cBus;
ChipSatPlatform::Uart console(ChipSatPlatform::UartPort::Console);
ChipSatPlatform::Uart gpsPort(ChipSatPlatform::UartPort::Gps);

ChipSatDevices::Imu imu(i2cBus, chipSystem);
ChipSatDevices::Gps gps(gpsPort, chipSystem);
ChipSatDevices::EnvSensor envSensor(i2cBus, chipSystem);
ChipSatDevices::FuelGauge fuelGauge(i2cBus, chipSystem);
ChipSatDevices::Radio radio(chipSystem);
ChipSatDevices::Flash flash(chipSystem);
ChipSatApp::Health health(chipSystem, i2cBus, imu, gps, envSensor, fuelGauge);

constexpr uint32_t kTxOffAtBootMs = 30000;   // the sensors alone first, then the radio as well
constexpr uint32_t kBeatMs = 1000;
constexpr uint32_t kImuLineMinMs = kImuReportIntervalMs;   // one line per IMU tick at most
constexpr uint32_t kImuLineMaxMs = 1000;                    // and one a second with nothing new
constexpr uint8_t kScanFirst = 0x08;   // the addresses I2C leaves for parts
constexpr uint8_t kScanLast = 0x77;
constexpr char kTxKey = 'R';

ChipSatSensors::SensorData data;
ChipSatTelemetry::TelemetryPacket packet{};

bool radioReady = false;
bool txOn = false;
bool txDecided = false;   // R pressed or the 30 s passed, the auto switch-on doesn't happen again
bool txBusy = false;
uint32_t txStartMs = 0;
uint32_t txDeadlineMs = 0;
uint16_t counter = 0;
uint32_t sent = 0;
uint32_t txBad = 0;

uint32_t imuLines = 0;
uint32_t imuLineMs = 0;
uint32_t imuSeenMs[4] = {0, 0, 0, 0};
uint32_t envReads = 0;
uint32_t envStartedMs = 0;
bool envMeasuring = false;
uint32_t gpsReads = 0;
uint32_t gpsReadMs = 0;
uint32_t gaugeReads = 0;
uint32_t gaugeReadMs = 0;

uint32_t beats = 0;
uint32_t beatMs = 0;
uint32_t loopMaxMs = 0;

static void serviceGps()
{
  gps.service();
}

// The console blocks about 5 ms per 64 byte buffer and the GPS port only holds 64 bytes, so it's read
// between lines
static void drainGps()
{
  chipSystem.waitMs(0);
}

// same line as FlightController::logResetCause
static void logResetCause()
{
  const ChipSatPlatform::ResetCause cause = chipSystem.readAndClearResetCause();

  LOG_BOOT(Rst, "cause") {
    line.fieldHex("csr", cause.csr, 8);
    line.field("lpwr", cause.lowPower);
    line.field("wwdg", cause.windowWatchdog);
    line.field("iwdg", cause.independentWatchdog);
    line.field("sft", cause.software);
    line.field("bor", cause.brownOut);
    line.field("pin", cause.pin);
    line.field("obl", cause.optionByteLoad);
    line.field("rfilar", cause.radioIllegalAccess);
  }
}

// Before any driver talks to the bus, so a part that won't start reads differently from one that
// isn't there. Gives up after three timeouts or held-bus failures in a row
static void scanI2c()
{
  char found[5 * (kScanLast - kScanFirst + 1) + 1];
  size_t at = 0;
  unsigned int count = 0;
  unsigned int noAck = 0;
  unsigned int timeouts = 0;
  unsigned int errors = 0;
  uint8_t timeoutsInRow = 0;
  bool stopped = false;
  static const char kHex[] = "0123456789ABCDEF";

  for (uint8_t address = kScanFirst; address <= kScanLast; ++address) {
    chipSystem.feedWatchdog();
    const Status status = i2cBus.probe(address);
    timeoutsInRow = status == Status::Timeout || status == Status::BusError ? timeoutsInRow + 1 : 0;
    if (status == Status::Ok) {
      if (count > 0) {
        found[at++] = ',';
      }
      found[at++] = '0';
      found[at++] = 'x';
      found[at++] = kHex[address >> 4];
      found[at++] = kHex[address & 0x0F];
      ++count;
    } else if (status == Status::NoAck) {
      ++noAck;
    } else if (status == Status::Timeout) {
      ++timeouts;
    } else {
      ++errors;
    }
    if (timeoutsInRow >= kI2cStuckTransfers) {
      stopped = true;
      break;
    }
  }
  found[at] = '\0';

  LOG_BOOT(Boot, "scan") {
    line.field("found", count > 0 ? found : "none");
    line.field("n", count);
    line.field("noack", noAck);
    line.field("timeouts", timeouts);
    line.field("errors", errors);
    line.field("stopped", stopped);
  }
}

// FlightController::configRadio, the same steps and lines
static bool configRadio()
{
  LOG_I(Radio, "start");
  Status status = radio.begin();
  LOG_D(Radio, "ldro") { line.field("code", radio.ldroCode()); }
  if (status != Status::Ok) {
    LOG_E(Radio, "failed") {
      line.field("reason", "radiobegin");
      line.field("code", radio.lastCode());
    }
    return false;
  }
  LOG_I(Radio, "init") {
    line.field("ok", true);
    line.field("code", radio.lastCode());
  }

  status = radio.setCurrentLimit();
  if (status != Status::Ok) {
    LOG_E(Radio, "failed") {
      line.field("reason", "radioocp");
      line.field("code", radio.lastCode());
    }
    return false;
  }
  status = radio.setOutputPower();
  if (status != Status::Ok) {
    LOG_E(Radio, "failed") {
      line.field("reason", "radiopower");
      line.field("code", radio.lastCode());
    }
    return false;
  }
  LOG_I(Radio, "power") { line.field("ok", true); }

  status = radio.applyPaConfig();
  if (status != Status::Ok) {
    LOG_E(Radio, "failed") {
      line.field("reason", "radiopa");
      line.field("code", radio.lastCode());
    }
    return false;
  }
  LOG_I(Radio, "pa") {
    line.field("ok", true);
    const ChipSatSettings::Settings& s = ChipSatSettings::active();
    const PaSettings pa = s.module == ChipSatSettings::kModuleHp ? paSettingsHighPower(s.targetDbm)
                                                                 : paSettingsLowPower(s.targetDbm);
    line.field("module", s.module == ChipSatSettings::kModuleHp ? "hp" : "le");
    line.field("want", s.targetDbm);
    line.field("dbm", pa.txParamsDbm);
    line.field("code", radio.lastCode());
  }
  LOG_I(Radio, "ocp") { line.field("ma", radio.currentLimitMa(), 2); }
  return true;
}

static void setTx(bool on, const char *why)
{
  txOn = on;
  LOG_I(Radio, "tx") {
    line.field("on", on);
    line.field("why", why);
  }
}

// R turns the radio on or off. Anything else is read and dropped
static void readConsole()
{
  while (console.available() > 0) {
    if (console.read() == kTxKey) {
      txDecided = true;
      setTx(!txOn, "key");
    }
  }
}

// At most every 50 ms, and once a second with nothing new so a frozen IMU still shows. A report
// that stops keeps its flag, new= and ev= are what show it
static void imuLine(uint32_t now)
{
  const ChipSatSensors::IMUData &d = imu.data();
  const uint32_t updated[4] = {d.linearAccelerationUpdatedMs, d.gyroscopeUpdatedMs, d.magnetometerUpdatedMs,
                               d.orientationUpdatedMs};
  uint8_t fresh = 0;
  for (uint8_t i = 0; i < 4; ++i) {
    if (updated[i] != imuSeenMs[i]) {
      fresh |= static_cast<uint8_t>(1U << i);
    }
  }
  const uint32_t sinceMs = now - imuLineMs;
  if (sinceMs < kImuLineMinMs || (fresh == 0 && sinceMs < kImuLineMaxMs)) {
    return;
  }
  imuLineMs = now;
  for (uint8_t i = 0; i < 4; ++i) {
    imuSeenMs[i] = updated[i];
  }
  ++imuLines;
  const uint8_t valid = (d.linearAccelerationValid ? 1U : 0U) | (d.gyroscopeValid ? 2U : 0U) |
                        (d.magnetometerValid ? 4U : 0U) | (d.orientationValid ? 8U : 0U);

  LOG_I(Imu, "bench") {
    line.field("n", imuLines);
    line.field("ok", imu.ready());
    line.fieldBits("valid", valid, 4);
    line.fieldBits("new", fresh, 4);
    line.field("ax", d.linearAccelerationMps2.x, 3);
    line.field("ay", d.linearAccelerationMps2.y, 3);
    line.field("az", d.linearAccelerationMps2.z, 3);
    line.field("gx", d.gyroscopeRadPerSec.x, 4);
    line.field("gy", d.gyroscopeRadPerSec.y, 4);
    line.field("gz", d.gyroscopeRadPerSec.z, 4);
    line.field("mx", d.magnetometerMicroTesla.x, 2);
    line.field("my", d.magnetometerMicroTesla.y, 2);
    line.field("mz", d.magnetometerMicroTesla.z, 2);
    line.field("qi", d.orientation.i, 5);
    line.field("qj", d.orientation.j, 5);
    line.field("qk", d.orientation.k, 5);
    line.field("qr", d.orientation.real, 5);
    line.fieldList("ev", imu.eventCounts(), 4);
  }
  drainGps();
}

static void envLine(Status status)
{
  ++envReads;
  const ChipSatSensors::EnvironmentalData &d = envSensor.data();
  LOG_I(Env, "bench") {
    line.field("n", envReads);
    line.field("ok", status == Status::Ok);
    line.field("status", ChipSatPlatform::statusName(status));
    line.field("valid", d.valid);
    line.field("temp", d.temperatureC, 2);
    line.field("phpa", d.pressurePa / 100.0f, 2);
    line.field("rh", d.humidityPercent, 2);
    line.field("altm", d.altitudeM, 2);
  }
  drainGps();
}

// The barometer as the flight loop paces it: a forced measurement every 200 ms, collected once the
// conversion is over. A start that fails still goes to read(), so every try has its line
static void sampleEnv(uint32_t now)
{
  if (!envMeasuring) {
    if (now - envStartedMs >= kEnvIntervalMs) {
      envStartedMs = now;
      if (envSensor.startMeasurement() == Status::Ok) {
        envMeasuring = true;
      } else {
        envLine(envSensor.read());
      }
    }
  } else if (now - envStartedMs >= kEnvConversionMs) {
    envMeasuring = false;
    envLine(envSensor.read());   // the conversion is over, so this doesn't wait
  }
}

static void sampleGps(uint32_t now)
{
  if (now - gpsReadMs < kGpsIntervalMs) {
    return;
  }
  gpsReadMs = now;
  // read() clears the driver's counters, so they are taken first
  const uint16_t bytes = ChipSatDevices::bytesIn;
  const uint16_t frames = ChipSatDevices::framesIn;
  const uint16_t bad = ChipSatDevices::badSums;
  const uint16_t notUbxBytes = ChipSatDevices::notUbx;
  const uint16_t queued = ChipSatDevices::mostQueued;
  const Status status = gps.read();
  const ChipSatSensors::GPSData &d = gps.data();
  ++gpsReads;

  LOG_I(Gps, "bench") {
    line.field("n", gpsReads);
    line.field("ok", status == Status::Ok);
    line.field("status", ChipSatPlatform::statusName(status));
    line.field("valid", d.valid);
    line.field("fix", gps.fixType());
    line.field("sats", gps.satellites());
    line.field("utc", gps.utcValid());
    line.fieldFixed("lat", d.latitudeE7, 7);
    line.fieldFixed("lon", d.longitudeE7, 7);
    line.field("altmm", static_cast<long>(d.altitudeMSLmm));
    line.field("bytes", bytes);
    line.field("frames", frames);
    line.field("bad", bad);
    line.field("notubx", notUbxBytes);
    line.field("queued", queued);
  }
  drainGps();
}

static void sampleGauge(uint32_t now)
{
  if (now - gaugeReadMs < kGaugeIntervalMs) {
    return;
  }
  gaugeReadMs = now;
  const Status status = fuelGauge.read();
  const ChipSatSensors::StateOfChargeData &d = fuelGauge.data();
  ++gaugeReads;

  LOG_I(Soc, "bench") {
    line.field("n", gaugeReads);
    line.field("ok", status == Status::Ok);
    line.field("status", ChipSatPlatform::statusName(status));
    line.field("valid", d.valid);
    line.field("pct", d.cellPercentage, 2);
    line.field("v", d.cellVoltage, 3);
    line.field("raw", fuelGauge.rawPercent(), 2);
  }
  drainGps();
}

static void finishPacket(Status status);

// Bit 7 (all fresh) is never set, the bench has no staleness rule
static void startPacket()
{
  if (!radioReady) {
    radioReady = configRadio();
    if (!radioReady) {
      ++txBad;
      LOG_E(Tx, "done") {
        line.field("ok", false);
        line.field("ctr", static_cast<unsigned int>(counter));
        line.field("status", "radio");
        line.field("code", radio.lastCode());
        line.field("ms", 0U);
      }
      return;
    }
  }
  data.imu = imu.data();
  data.gps = gps.data();
  data.stateOfCharge = fuelGauge.data();
  data.environmental = envSensor.data();
  ChipSatTelemetry::encodePacket(data, counter, ChipSatSettings::active().csid, false, packet);
  LOG_PACKET(reinterpret_cast<const uint8_t *>(&packet), sizeof(packet));
  drainGps();

  txStartMs = chipSystem.nowMs();
  const Status status = radio.startTransmit(reinterpret_cast<const uint8_t *>(&packet), sizeof(packet));
  if (status != Status::Ok) {
    finishPacket(status);
    return;
  }
  // SetTx has no timeout of its own (RadioSubGhz.cpp). 100 ms over time on air for the TCXO start
  // and the ramp, as in FlightController::transmit
  txDeadlineMs = radio.timeOnAirMs(sizeof(packet)) + 100;
  txBusy = true;
}

static void finishPacket(Status status)
{
  txBusy = false;
  const uint32_t txMs = chipSystem.nowMs() - txStartMs;
  if (status == Status::Ok) {
    ++sent;
    LOG_I(Tx, "done") {
      line.field("ok", true);
      line.field("ctr", static_cast<unsigned int>(counter));
      line.field("code", radio.lastCode());
      line.field("ms", txMs);
    }
  } else {
    ++txBad;
    radioReady = false;   // set the radio up again before the next packet
    LOG_E(Tx, "done") {
      line.field("ok", false);
      line.field("ctr", static_cast<unsigned int>(counter));
      line.field("status", ChipSatPlatform::statusName(status));
      line.field("code", radio.lastCode());
      line.field("ms", txMs);
    }
  }
  ++counter;
  drainGps();
}

// Sensors keep being read while a packet is on air
static void serviceRadio(uint32_t now)
{
  if (txBusy) {
    if (radio.transmitDone()) {
      finishPacket(radio.finishTransmit());
    } else if (now - txStartMs > txDeadlineMs) {
      radio.finishTransmit();
      finishPacket(Status::Timeout);
    }
    return;
  }
  if (txOn) {
    startPacket();
  }
}

static void beat(uint32_t now)
{
  if (now - beatMs < kBeatMs) {
    return;
  }
  beatMs = now;
  ++beats;
  ChipSatLog::setCycle(beats);
  LOG_I(Cyc, "beat") {
    line.field("n", beats);
    line.field("txon", txOn);
    line.field("loopmaxms", loopMaxMs);
    line.field("sent", sent);
    line.field("txbad", txBad);
  }
  loopMaxMs = 0;
  drainGps();
}

void setup()
{
  console.begin(kConsoleBaud);
  ChipSatLog::begin(console, chipSystem);
  LOG_BOOT(Boot, "start") {
    line.field("test", "bench");
    line.field("module", kRadioModuleName);
    line.field("csid", ChipSatSettings::active().csid);
    line.field("sf", ChipSatSettings::active().spreadingFactor);
    line.field("dbm", ChipSatSettings::active().targetDbm);
    line.field("txoffms", kTxOffAtBootMs);
  }
  logResetCause();

  // waitMs feeds it too, so this only catches a hard lock-up. A hung sensor shows as its lines
  // stopping and in the beat's loopmaxms
  chipSystem.startWatchdog(kWatchdogTimeoutMs);
  chipSystem.onWait(serviceGps);

  i2cBus.begin(kI2cSdaPin, kI2cSclPin);
  scanI2c();
  gpsPort.begin(kGpsBaud);   // before the IMU, as in flight

  // The order FlightController::beginSensors starts them in, the flash log left out
  flash.begin();
  imu.begin(kImuReportIntervalMs);
  gps.begin();
  fuelGauge.begin();
  envSensor.begin();
  radioReady = configRadio();
  setTx(false, "start");
  health.begin();

  LOG_BOOT(Boot, "setup") {
    line.field("done", true);
    line.field("watchdogms", kWatchdogTimeoutMs);
  }
  beatMs = chipSystem.nowMs();
}

void loop()
{
  const uint32_t passStartMs = chipSystem.nowMs();
  chipSystem.feedWatchdog();

  readConsole();
  if (!txDecided && passStartMs >= kTxOffAtBootMs) {
    txDecided = true;
    setTx(true, "auto");
  }

  imu.service();
  gps.service();
  const uint32_t now = chipSystem.nowMs();
  imuLine(now);
  sampleEnv(now);
  sampleGps(now);
  sampleGauge(now);
  health.service(false);   // the bench is the ground
  serviceRadio(chipSystem.nowMs());
  beat(chipSystem.nowMs());

  const uint32_t passMs = chipSystem.nowMs() - passStartMs;
  if (passMs > loopMaxMs) {
    loopMaxMs = passMs;
  }
}
