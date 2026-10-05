// Is the orientation right after a wake if the board was moved while the IMU slept?
// Checked against gravity from the accelerometer. Follow the console prompts. IMU only.

#include <Wire.h>
#include <SparkFun_BNO08x_Arduino_Library.h>
#include <sh2.h>
#include <sh2_SensorValue.h>

HardwareSerial Console = HardwareSerial(PB7, PB6);
BNO08x imu;

// Same as the flight software.
constexpr uint16_t kReportMs = 50;
constexpr uint32_t kWakeSettleMs = 150;

// Test timing.
constexpr uint32_t kMeasureMs = 3500;     // how long each wake is watched
constexpr uint32_t kMoveWindowMs = 9000;  // time you get to move the board
constexpr uint32_t kHandsOffMs = 3000;    // hands off before the IMU wakes
constexpr float    kGoodDeg = 5.0f;       // "correct" means within this
constexpr float    kMovingDps = 10.0f;    // faster than this = someone moved it

// The steps. Each step: you do the move while the IMU sleeps, take your hands
// away, then the IMU wakes and is checked.
struct Step {
  const char *label;
  const char *instruction;
};

const Step kSteps[] = {
  { "flat, not moved",
    "Leave the board lying flat, TOP up. Don't touch it." },
  { "standing on an edge",
    "Stand the board up on one of its edges and lean it against your prop, so it stays up on its own." },
  { "edge, not moved",
    "Leave it standing on that edge. Don't touch it." },
  { "back flat",
    "Lay it back down flat, TOP up, in the same spot." },
  { "standing on another edge",
    "Stand it up on a DIFFERENT edge (one next to the first), leaning on the prop again." },
  { "back flat",
    "Lay it back down flat, TOP up, in the same spot." },
  { "quarter turn, flat",
    "Keep it flat, TOP up, and turn it on the desk by a quarter turn, like a clock hand going from 12 to 3." },
  { "flat, not moved",
    "Leave it flat. Don't touch it." },
};
constexpr int kStepCount = sizeof(kSteps) / sizeof(kSteps[0]);

// What the report handler keeps.
struct Vec3 {
  float x;
  float y;
  float z;
};

enum { LA, GY, MG, RV, N_FLIGHT };  // the four reports the flight software uses

bool arrived[N_FLIGHT];
uint32_t orientationCount = 0;       // goes up with every orientation report
float qw = 1.0f, qx = 0.0f, qy = 0.0f, qz = 0.0f;
uint8_t accuracyStatus = 0;          // IMU's own rating: 0 unreliable ... 3 high
float headingErrorDeg = 180.0f;      // IMU's own heading error estimate
Vec3 accel = {0.0f, 0.0f, 0.0f};     // includes gravity
bool haveAccel = false;
float gyroDps = 0.0f;                // how fast the board is turning

// One step's result. Defined up here, above every function, because the
// Arduino builder inserts function declarations above the first function in
// the file, and those declarations mention Result.
struct Result {
  bool valid = true;         // false if the board moved while the IMU was awake
  bool gotRead = false;
  float tiltDeg = 0.0f;      // measured tilt away from the start position
  uint32_t readMs = 0;       // when the flight software would read (after wake)
  float errAtRead = 0.0f;    // THE number that matters
  float errAt1s = -1.0f;
  float errAtEnd = 0.0f;
  bool settled = false;
  uint32_t settledMs = 0;    // from here on the error stayed under kGoodDeg
  uint8_t accuracyAtRead = 0;
  uint8_t accuracyAtEnd = 0;
  float headingAtRead = 0.0f;
  float headingAtEnd = 0.0f;
};

// Every report the IMU sends arrives here.
static void onReport(void *, sh2_SensorEvent_t *event) {
  sh2_SensorValue_t v;
  if (sh2_decodeSensorEvent(&v, event) != SH2_OK) return;

  switch (v.sensorId) {
    case SH2_ACCELEROMETER:
      accel.x = v.un.accelerometer.x;
      accel.y = v.un.accelerometer.y;
      accel.z = v.un.accelerometer.z;
      haveAccel = true;
      break;
    case SH2_LINEAR_ACCELERATION:
      arrived[LA] = true;
      break;
    case SH2_GYROSCOPE_CALIBRATED:
      arrived[GY] = true;
      gyroDps = sqrt(v.un.gyroscope.x * v.un.gyroscope.x +
                     v.un.gyroscope.y * v.un.gyroscope.y +
                     v.un.gyroscope.z * v.un.gyroscope.z) * RAD_TO_DEG;
      break;
    case SH2_MAGNETIC_FIELD_CALIBRATED:
      arrived[MG] = true;
      break;
    case SH2_ROTATION_VECTOR:
      arrived[RV] = true;
      qw = v.un.rotationVector.real;
      qx = v.un.rotationVector.i;
      qy = v.un.rotationVector.j;
      qz = v.un.rotationVector.k;
      accuracyStatus = v.status & 0x03;
      headingErrorDeg = v.un.rotationVector.accuracy * RAD_TO_DEG;
      orientationCount++;
      break;
    default:
      break;
  }
}

static bool allFourArrived() {
  return arrived[LA] && arrived[GY] && arrived[MG] && arrived[RV];
}

// Geometry. "Up" according to the IMU's orientation, and "up" according to
// gravity. The angle between them is the IMU's orientation error.
int upForm = 0;       // which of two equivalent ways to read "up" from the orientation
int gravitySign = 0;  // 0 = accelerometer as-is, 1 = flipped
Vec3 startUp = {0.0f, 0.0f, 1.0f};  // gravity "up" in the start position

static Vec3 orientationUp(int form) {
  Vec3 u;
  if (form == 0) {
    u.x = 2.0f * (qx * qz - qw * qy);
    u.y = 2.0f * (qy * qz + qw * qx);
  } else {
    u.x = 2.0f * (qx * qz + qw * qy);
    u.y = 2.0f * (qy * qz - qw * qx);
  }
  u.z = 1.0f - 2.0f * (qx * qx + qy * qy);
  return u;
}

static Vec3 gravityUp(int sign) {
  Vec3 g = accel;
  if (sign == 1) {
    g.x = -g.x;
    g.y = -g.y;
    g.z = -g.z;
  }
  return g;
}

static float angleBetweenDeg(const Vec3 &a, const Vec3 &b) {
  const float dot = a.x * b.x + a.y * b.y + a.z * b.z;
  const float na = sqrt(a.x * a.x + a.y * a.y + a.z * a.z);
  const float nb = sqrt(b.x * b.x + b.y * b.y + b.z * b.z);
  if (na < 1e-6f || nb < 1e-6f) return 180.0f;
  return acos(constrain(dot / (na * nb), -1.0f, 1.0f)) * RAD_TO_DEG;
}

static float orientationErrorDeg() {
  return angleBetweenDeg(orientationUp(upForm), gravityUp(gravitySign));
}

static float headingDeg() {
  return atan2(2.0f * (qw * qz + qx * qy), 1.0f - 2.0f * (qy * qy + qz * qz)) * RAD_TO_DEG;
}

static float headingDifferenceDeg(float from, float to) {
  float d = to - from;
  while (d > 180.0f) d -= 360.0f;
  while (d < -180.0f) d += 360.0f;
  return d;
}

// IMU control.
// Accelerometer first (the ground truth), then the flight software's four.
static bool enableReports() {
  bool ok[5] = {false, false, false, false, false};
  for (int r = 0; r < 5; r++) {
    for (int attempt = 0; attempt < 5 && !ok[r]; attempt++) {
      switch (r) {
        case 0: ok[r] = imu.enableAccelerometer(kReportMs); break;
        case 1: ok[r] = imu.enableLinearAccelerometer(kReportMs); break;
        case 2: ok[r] = imu.enableGyro(kReportMs); break;
        case 3: ok[r] = imu.enableMagnetometer(kReportMs); break;
        case 4: ok[r] = imu.enableRotationVector(kReportMs); break;
      }
      delay(ok[r] ? 50 : 75);
    }
  }
  return ok[0] && ok[1] && ok[2] && ok[3] && ok[4];
}

// Keep collecting reports for a while, doing nothing else.
static void serviceFor(uint32_t ms) {
  const uint32_t start = millis();
  while (millis() - start < ms) {
    sh2_service();
    delay(2);
  }
}

// Collect reports and print the IMU's accuracy every 5 s.
static void streamAndReport(uint32_t ms) {
  const uint32_t start = millis();
  uint32_t nextPrint = 5000;
  while (millis() - start < ms) {
    sh2_service();
    delay(2);
    if (millis() - start >= nextPrint) {
      Console.print("    streaming  +");
      Console.print(nextPrint / 1000);
      Console.print(" s   accuracy status ");
      Console.print(accuracyStatus);
      Console.print("   heading error ");
      Console.print(headingErrorDeg, 0);
      Console.println(" deg");
      nextPrint += 5000;
    }
  }
}

// With the board flat and still, work out how to compare the IMU with
// gravity, and remember "up" for the start position.
static bool calibrateComparison(uint32_t ms) {
  float sum[4] = {0.0f, 0.0f, 0.0f, 0.0f};
  uint32_t samples = 0;
  uint32_t lastSeen = orientationCount;
  const uint32_t start = millis();

  while (millis() - start < ms) {
    sh2_service();
    if (orientationCount != lastSeen && haveAccel) {
      lastSeen = orientationCount;
      for (int c = 0; c < 4; c++) {
        sum[c] += angleBetweenDeg(orientationUp(c & 1), gravityUp((c >> 1) & 1));
      }
      samples++;
    }
    delay(2);
  }
  if (samples == 0) return false;

  int best = 0;
  for (int c = 1; c < 4; c++) {
    if (sum[c] < sum[best]) best = c;
  }
  upForm = best & 1;
  gravitySign = (best >> 1) & 1;
  startUp = gravityUp(gravitySign);

  const float agreement = sum[best] / samples;
  Console.print("    IMU and gravity agree to within ");
  Console.print(agreement, 1);
  Console.print(" deg while flat and still (");
  Console.print(samples);
  Console.println(" samples)");
  if (agreement > 10.0f) {
    Console.println("    WARNING: they disagree a lot even when still - tell me before trusting the results");
  }
  return true;
}

// One step's measurement.
Result results[kStepCount];

static Result measureWake() {
  Result r;
  for (int i = 0; i < N_FLIGHT; i++) arrived[i] = false;
  haveAccel = false;

  // Wake exactly as the rebuild will: on, 150 ms, reports back on.
  const uint32_t wakeMs = millis();
  imu.modeOn();
  delay(kWakeSettleMs);
  enableReports();

  uint32_t lastSeen = orientationCount;
  uint32_t lastSampleMs = 0;
  uint32_t lastBadMs = 0;
  bool anyBad = false;
  bool haveSample = false;
  float fastestDps = 0.0f;

  while (millis() - wakeMs < kMeasureMs) {
    sh2_service();
    if (gyroDps > fastestDps) fastestDps = gyroDps;

    if (orientationCount != lastSeen && haveAccel) {
      lastSeen = orientationCount;
      const uint32_t t = millis() - wakeMs;
      const float err = orientationErrorDeg();

      if (!r.gotRead && allFourArrived()) {
        r.gotRead = true;
        r.readMs = t;
        r.errAtRead = err;
        r.accuracyAtRead = accuracyStatus;
        r.headingAtRead = headingDeg();
      }
      if (r.errAt1s < 0.0f && t >= 1000) r.errAt1s = err;
      if (err >= kGoodDeg) {
        anyBad = true;
        lastBadMs = t;
      }
      r.errAtEnd = err;
      lastSampleMs = t;
      haveSample = true;
    }
    delay(2);
  }

  imu.modeSleep();

  r.accuracyAtEnd = accuracyStatus;
  r.headingAtEnd = headingDeg();
  r.tiltDeg = angleBetweenDeg(gravityUp(gravitySign), startUp);
  r.valid = (fastestDps < kMovingDps);
  if (haveSample) {
    if (!anyBad) {
      r.settled = true;
      r.settledMs = r.readMs;
    } else if (lastBadMs < lastSampleMs) {
      r.settled = true;
      r.settledMs = lastBadMs + kReportMs;
    }
  }
  return r;
}

static const char *verdict(const Result &r) {
  if (!r.valid) return "NOT VALID (the board moved while the IMU was awake)";
  if (!r.gotRead) return "NO READING";
  return (r.errAtRead < kGoodDeg) ? "PASS" : "FAIL";
}

static void printResult(int step, const Result &r, float headingBefore) {
  Console.println();
  Console.print("RESULT step ");
  Console.print(step + 1);
  Console.print(": ");
  Console.println(kSteps[step].label);
  Console.print("  measured tilt from the start position: ");
  Console.print(r.tiltDeg, 0);
  Console.println(" deg");
  if (r.gotRead) {
    Console.print("  IMU orientation error when the flight software reads it (+");
    Console.print(r.readMs);
    Console.print(" ms): ");
    Console.print(r.errAtRead, 1);
    Console.println(" deg   <-- the number that matters");
  }
  Console.print("  error after 1 s: ");
  Console.print(r.errAt1s, 1);
  Console.print(" deg    at the end (+3.5 s): ");
  Console.print(r.errAtEnd, 1);
  Console.println(" deg");
  if (r.settled) {
    Console.print("  error stayed under 5 deg from +");
    Console.print(r.settledMs);
    Console.println(" ms");
  } else {
    Console.println("  error did NOT settle under 5 deg within 3.5 s");
  }
  Console.print("  IMU accuracy status: ");
  Console.print(r.accuracyAtRead);
  Console.print(" at read, ");
  Console.print(r.accuracyAtEnd);
  Console.println(" at end");
  Console.print("  heading change since the previous step: at read ");
  Console.print(headingDifferenceDeg(headingBefore, r.headingAtRead), 0);
  Console.print(" deg, at end ");
  Console.print(headingDifferenceDeg(headingBefore, r.headingAtEnd), 0);
  Console.println(" deg");
  Console.print("  verdict: ");
  Console.println(verdict(r));
}

static void waitWithCountdown(uint32_t ms) {
  const uint32_t start = millis();
  uint32_t nextNote = 5000;
  while (millis() - start < ms) {
    delay(20);
    if (millis() - start >= nextNote && nextNote < ms) {
      Console.print("    ");
      Console.print((ms - nextNote) / 1000);
      Console.println(" s left");
      nextNote += 5000;
    }
  }
}

void setup() {
  Console.begin(115200);
  delay(300);
  Console.println();
  Console.println("t02_imu_sleep_tilt v2 boot");

  Wire.setSDA(PA15);
  Wire.setSCL(PB15);
  Wire.begin();

  bool imuOk = false;
  for (int attempt = 0; attempt < 5 && !imuOk; attempt++) {
    imuOk = imu.begin(0x4A, Wire, -1, -1);
    if (!imuOk) delay(500);
  }
  if (!imuOk) {
    while (true) {
      Console.println("IMU not found");
      delay(5000);
    }
  }
  sh2_setSensorCallback(onReport, nullptr);
  imu.wasReset();

  delay(1000);
  if (!enableReports()) Console.println("warning: not every report could be turned on");

  // Part 1: IMU streaming, as V2.5 runs it.
  Console.println();
  Console.println("PART 1 of 2: warm-up with the IMU running continuously");
  Console.println(">>> Lay the board flat on the desk and let go. The side facing up now is TOP.");
  streamAndReport(20000);

  Console.println(">>> Pick it up and SLOWLY turn it in every direction for 30 s:");
  Console.println(">>> tip it forwards, backwards, side to side, and spin it. Keep the cables slack.");
  streamAndReport(30000);

  Console.println(">>> Put it back down flat, TOP up, and let go. Stay hands off.");
  serviceFor(5000);
  if (!calibrateComparison(10000)) Console.println("warning: no readings while flat");

  Console.println();
  Console.println("PART 2 of 2: the IMU sleeps; you move the board; it wakes and is checked");
  Console.print("There are ");
  Console.print(kStepCount);
  Console.println(" steps, 15 s each.");
}

void loop() {
  static int step = 0;
  static float headingBefore = headingDeg();

  if (step >= kStepCount) return;

  imu.modeSleep();
  Console.println();
  Console.print(">>> STEP ");
  Console.print(step + 1);
  Console.print(" of ");
  Console.print(kStepCount);
  Console.print(": ");
  Console.println(kSteps[step].instruction);
  Console.println(">>> Do it now (9 s).");
  waitWithCountdown(kMoveWindowMs);
  Console.println(">>> HANDS OFF - keep completely still until the result appears.");
  delay(kHandsOffMs);

  results[step] = measureWake();
  printResult(step, results[step], headingBefore);
  headingBefore = results[step].headingAtEnd;

  step++;
  if (step < kStepCount) return;

  // Summary.
  Console.println();
  Console.println("SUMMARY (error = IMU orientation vs real gravity, in degrees)");
  Console.println("step  what                        tilt  err@read  read_ms  err@1s  settled_ms  acc  verdict");
  for (int i = 0; i < kStepCount; i++) {
    const Result &r = results[i];
    Console.print(i + 1);
    Console.print("     ");
    Console.print(kSteps[i].label);
    Console.print("  |  ");
    Console.print(r.tiltDeg, 0);
    Console.print("  |  ");
    Console.print(r.errAtRead, 1);
    Console.print("  |  ");
    Console.print(r.readMs);
    Console.print("  |  ");
    Console.print(r.errAt1s, 1);
    Console.print("  |  ");
    if (r.settled) {
      Console.print(r.settledMs);
    } else {
      Console.print("never");
    }
    Console.print("  |  ");
    Console.print(r.accuracyAtRead);
    Console.print("  |  ");
    Console.println(verdict(r));
  }
  Console.println("DONE - copy everything from PART 1 to here and send it over.");
}
