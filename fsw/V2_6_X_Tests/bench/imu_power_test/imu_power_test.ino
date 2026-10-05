// IMU current draw: asleep, streaming, and woken every 10 s. Run it on the INA260 rig.
// tools/imu_power_analyze.py reads the output.
// Phases, 60 s each:
//   A  asleep (baseline)
//   B  4 reports at 50 ms, what the flight software does
//   C  4 reports at 500 ms
//   D  asleep, every 10 s wake + re-enable reports + wait for one of each
//   E  same as D without the re-enable
// It also counts how many reports the library's own shared-slot path would have seen.
//
// Console lines:
//   SCHED,start_ms=...,phase_ms=...,order=...
//   PH,<idx>,<code>,START,<millis>
//   PH,<idx>,<code>,END,<millis>,la=,gy=,mg=,rv=,vla=,vgy=,vmg=,vrv=,svc=,rst=
//   WAKE,<idx>,<code>,<cycle>,on=,en=,la=,gy=,mg=,rv=,awake=,ok=   (ms since wake began, -1 = never)

#include <Wire.h>
#include <SparkFun_BNO08x_Arduino_Library.h>
#include <SparkFun_u-blox_GNSS_Arduino_Library.h>
#include <sh2.h>

// test settings
#define PARK_GPS 1                          // 1 = GPS in software standby for the whole test
constexpr uint32_t kStartMs = 10000;        // first phase starts this long after power-on
constexpr uint32_t kPhaseMs = 60000;        // length of every phase
constexpr uint32_t kDutyPeriodMs = 10000;   // D/E: one wake every 10 s (V2.5's shortest TX interval)
constexpr uint32_t kDutyOffsetMs = 1000;    // D/E: first wake 1 s into each 10 s slot
constexpr uint16_t kFastReportMs = 50;      // V2.5 report interval
constexpr uint16_t kSlowReportMs = 500;
constexpr uint16_t kFreshTimeoutMs = 3000;  // V2.5 kIMUFreshDataTimeoutMs
constexpr uint16_t kWakeSettleMs = 150;     // V2.5 kIMUWakeStartupDelayMs
const char kOrder[] = "ABCDEABCDEA";

// pins (same as V2.5)
HardwareSerial Console(PB7, PB6);
HardwareSerial GPSSerial(PC1, PC0);
constexpr uint32_t kLedPin = PA9;

BNO08x imu;
SFE_UBLOX_GNSS gnss;

// report bookkeeping
enum { LA, GY, MG, RV, N_REP };

uint32_t rawCount[N_REP];      // every report the IMU delivered
uint32_t visibleCount[N_REP];  // what the V2.5 / library path would have seen
uint32_t serviceCalls = 0;
uint32_t resetCount = 0;
int lastReportThisCall = -1;
bool seenSinceWake[N_REP];
uint32_t firstSeenMs[N_REP];
bool imuAwake = true;

static int reportIndex(uint8_t id) {
  switch (id) {
    case SH2_LINEAR_ACCELERATION: return LA;
    case SH2_GYROSCOPE_CALIBRATED: return GY;
    case SH2_MAGNETIC_FIELD_CALIBRATED: return MG;
    case SH2_ROTATION_VECTOR: return RV;
    default: return -1;
  }
}

// Replaces the library's sensorHandler. Called once per report inside sh2_service().
static void countingHandler(void *, sh2_SensorEvent_t *event) {
  const int i = reportIndex(event->reportId);
  if (i < 0) return;
  rawCount[i]++;
  lastReportThisCall = i;
  if (!seenSinceWake[i]) {
    seenSinceWake[i] = true;
    firstSeenMs[i] = millis();
  }
}

// One service pass. The library's getSensorEvent() would expose only the last
// report decoded in this pass, so that one is what counts as "visible".
static void serviceIMU() {
  lastReportThisCall = -1;
  sh2_service();
  serviceCalls++;
  if (lastReportThisCall >= 0) visibleCount[lastReportThisCall]++;
}

static void clearCounts() {
  for (int i = 0; i < N_REP; i++) rawCount[i] = visibleCount[i] = 0;
  serviceCalls = 0;
  resetCount = 0;
}

// Same retry pattern as V2.5 configureIMUReports().
static bool enableReports(uint16_t intervalMs) {
  bool ok[4] = {false, false, false, false};
  for (int r = 0; r < 4; r++) {
    for (int attempt = 0; attempt < 5 && !ok[r]; attempt++) {
      switch (r) {
        case 0: ok[r] = imu.enableLinearAccelerometer(intervalMs); break;
        case 1: ok[r] = imu.enableGyro(intervalMs); break;
        case 2: ok[r] = imu.enableMagnetometer(intervalMs); break;
        case 3: ok[r] = imu.enableRotationVector(intervalMs); break;
      }
      delay(ok[r] ? 50 : 75);
    }
  }
  const bool all = ok[0] && ok[1] && ok[2] && ok[3];
  if (!all) {
    Console.print(F("WARN,enable failed la/gy/mg/rv="));
    Console.print(ok[0]); Console.print(ok[1]); Console.print(ok[2]); Console.println(ok[3]);
  }
  return all;
}

static void imuSleep() {
  if (!imuAwake) return;
  if (!imu.modeSleep()) Console.println(F("WARN,modeSleep failed"));
  imuAwake = false;
}

static bool imuWake() {
  if (imuAwake) return true;
  const bool ok = imu.modeOn();
  imuAwake = true;
  return ok;
}

static void checkReset(uint16_t reenableMs) {
  if (imu.wasReset()) {
    resetCount++;
    Console.print(F("RST,")); Console.println(millis());
    if (reenableMs > 0) enableReports(reenableMs);
  }
}

static void waitUntil(uint32_t targetMs) {
  while ((int32_t)(millis() - targetMs) < 0) { }
}

// phases
static void phaseAsleep(uint32_t endMs) {
  imuSleep();
  waitUntil(endMs);
}

static void phaseStreaming(uint32_t endMs, uint16_t intervalMs) {
  if (!imuWake()) Console.println(F("WARN,modeOn failed"));
  delay(kWakeSettleMs);
  enableReports(intervalMs);
  while ((int32_t)(millis() - endMs) < 0) {
    serviceIMU();
    checkReset(intervalMs);
  }
}

static void phaseDuty(int idx, char code, uint32_t startMs, uint32_t endMs, bool reenable) {
  imuSleep();
  int cycle = 0;
  for (uint32_t slot = startMs + kDutyOffsetMs; (int32_t)(endMs - slot) > (int32_t)kFreshTimeoutMs + 500;
       slot += kDutyPeriodMs, cycle++) {
    waitUntil(slot);
    const uint32_t t0 = millis();
    for (int i = 0; i < N_REP; i++) seenSinceWake[i] = false;

    const bool on = imuWake();
    int32_t enMs = -1;
    if (reenable) {
      delay(kWakeSettleMs);
      enableReports(kFastReportMs);
      enMs = millis() - t0;
    }
    while (!(seenSinceWake[LA] && seenSinceWake[GY] && seenSinceWake[MG] && seenSinceWake[RV]) &&
           millis() - t0 < kFreshTimeoutMs + (reenable ? (uint32_t)enMs : 0)) {
      serviceIMU();
      checkReset(0);
      delay(2);
    }
    const uint32_t awakeMs = millis() - t0;
    imuSleep();

    const bool ok = seenSinceWake[LA] && seenSinceWake[GY] && seenSinceWake[MG] && seenSinceWake[RV];
    Console.print(F("WAKE,")); Console.print(idx); Console.print(','); Console.print(code);
    Console.print(','); Console.print(cycle);
    Console.print(F(",on=")); Console.print(on ? 1 : 0);
    Console.print(F(",en=")); Console.print(enMs);
    const char *names[N_REP] = {",la=", ",gy=", ",mg=", ",rv="};
    for (int i = 0; i < N_REP; i++) {
      Console.print(names[i]);
      Console.print(seenSinceWake[i] ? (int32_t)(firstSeenMs[i] - t0) : -1);
    }
    Console.print(F(",awake=")); Console.print(awakeMs);
    Console.print(F(",ok=")); Console.println(ok ? 1 : 0);
  }
  waitUntil(endMs);
}

// setup / loop
static void haltWith(const __FlashStringHelper *msg) {
  while (true) {
    Console.println(msg);
    delay(5000);
  }
}

void setup() {
  pinMode(kLedPin, OUTPUT);
  digitalWrite(kLedPin, LOW);  // LED on for the whole test

  Console.begin(115200);
  Console.println(F("t01_imu_power boot"));
  Console.print(F("SCHED,start_ms=")); Console.print(kStartMs);
  Console.print(F(",phase_ms=")); Console.print(kPhaseMs);
  Console.print(F(",order="));
  for (size_t i = 0; i + 1 < sizeof(kOrder); i++) {
    if (i) Console.print(',');
    Console.print(kOrder[i]);
  }
  Console.println();

  Wire.setSDA(PA15);
  Wire.setSCL(PB15);
  Wire.begin();

  bool imuOk = false;
  for (int attempt = 0; attempt < 5 && !imuOk; attempt++) {
    imuOk = imu.begin(0x4A, Wire, -1, -1);
    if (!imuOk) delay(500);
  }
  if (!imuOk) haltWith(F("FATAL,BNO085 begin failed"));
  sh2_setSensorCallback(countingHandler, nullptr);  // count reports ourselves from now on
  imu.wasReset();                                    // clear the reset flag left by startup
  Console.println(F("INFO,BNO085 ok"));

#if PARK_GPS
  GPSSerial.begin(9600);
  bool gpsOk = false;
  for (int attempt = 0; attempt < 3 && !gpsOk; attempt++) gpsOk = gnss.begin(GPSSerial);
  if (gpsOk) {
    // Indefinite software standby, woken only by UART RX. We never write to the
    // GPS again, so it stays asleep until the next power cycle.
    const bool parked = gnss.powerOffWithInterrupt(0, VAL_RXM_PMREQ_WAKEUPSOURCE_UARTRX);
    Console.println(parked ? F("INFO,GPS parked in standby") : F("WARN,GPS standby command failed"));
  } else {
    Console.println(F("WARN,GPS not found; it stays in whatever mode it was in"));
  }
#endif

  if (millis() > kStartMs) Console.println(F("WARN,boot overran start_ms; first phase starts late"));
}

void loop() {
  static bool done = false;
  if (done) return;

  const int phases = sizeof(kOrder) - 1;
  for (int idx = 0; idx < phases; idx++) {
    const char code = kOrder[idx];
    const uint32_t startMs = kStartMs + idx * kPhaseMs;
    const uint32_t endMs = startMs + kPhaseMs;
    waitUntil(startMs);
    clearCounts();
    Console.print(F("PH,")); Console.print(idx); Console.print(','); Console.print(code);
    Console.print(F(",START,")); Console.println(millis());

    switch (code) {
      case 'A': phaseAsleep(endMs); break;
      case 'B': phaseStreaming(endMs, kFastReportMs); break;
      case 'C': phaseStreaming(endMs, kSlowReportMs); break;
      case 'D': phaseDuty(idx, code, startMs, endMs, true); break;
      case 'E': phaseDuty(idx, code, startMs, endMs, false); break;
    }

    Console.print(F("PH,")); Console.print(idx); Console.print(','); Console.print(code);
    Console.print(F(",END,")); Console.print(millis());
    Console.print(F(",la=")); Console.print(rawCount[LA]);
    Console.print(F(",gy=")); Console.print(rawCount[GY]);
    Console.print(F(",mg=")); Console.print(rawCount[MG]);
    Console.print(F(",rv=")); Console.print(rawCount[RV]);
    Console.print(F(",vla=")); Console.print(visibleCount[LA]);
    Console.print(F(",vgy=")); Console.print(visibleCount[GY]);
    Console.print(F(",vmg=")); Console.print(visibleCount[MG]);
    Console.print(F(",vrv=")); Console.print(visibleCount[RV]);
    Console.print(F(",svc=")); Console.print(serviceCalls);
    Console.print(F(",rst=")); Console.println(resetCount);
  }

  imuSleep();
  Console.println(F("DONE"));
  done = true;
}
