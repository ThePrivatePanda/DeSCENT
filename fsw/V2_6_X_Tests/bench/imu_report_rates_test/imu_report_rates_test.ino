// What report intervals the BNO085 allows, and what it actually does.
// Prints each report's metadata, then turns the four reports on at a range of intervals and counts what arrives.
// The metadata reads have no timeout, reset if it hangs. Takes about a minute, ends with DONE.

#include <Wire.h>
#include <SparkFun_BNO08x_Arduino_Library.h>

HardwareSerial console(PB7, PB6);
BNO08x bno;

struct Sensor
{
  sh2_SensorId_t id;
  const char *name;
};

const Sensor kSensors[] = {
  {SH2_LINEAR_ACCELERATION, "linaccel"},
  {SH2_GYROSCOPE_CALIBRATED, "gyro"},
  {SH2_MAGNETIC_FIELD_CALIBRATED, "mag"},
  {SH2_ROTATION_VECTOR, "quat"},
};
const int kSensorCount = 4;

// requested report intervals in microseconds. flight uses 50 ms. 2.5 ms = the datasheet's 400 Hz
const uint32_t kIntervalsUs[] = {2500, 10000, 50000, 200000, 1000000, 5000000, 10000000};
const int kIntervalCount = sizeof(kIntervalsUs) / sizeof(kIntervalsUs[0]);

uint32_t counts[kSensorCount];

// our own callback so every report is counted. the lib's handler keeps only the last one per transfer
void onEvent(void *, sh2_SensorEvent_t *event)
{
  for (int i = 0; i < kSensorCount; ++i) {
    if (event->reportId == kSensors[i].id) {
      ++counts[i];
      return;
    }
  }
}

void serviceFor(uint32_t ms)
{
  const uint32_t start = millis();
  while (millis() - start < ms) {
    bno.serviceBus();
  }
}

void printMetadata()
{
  console.println("== metadata");
  console.println("sensor    rev  minPeriod_us  maxPeriod_us  power_mA  vendor");
  for (int i = 0; i < kSensorCount; ++i) {
    sh2_SensorMetadata_t meta;
    const int rc = sh2_getMetadata(kSensors[i].id, &meta);
    console.print(kSensors[i].name);
    console.print("  ");
    if (rc != SH2_OK) {
      console.print("read failed, code ");
      console.println(rc);
      continue;
    }
    meta.vendorId[sizeof(meta.vendorId) - 1] = '\0';
    console.print(meta.revision);
    console.print("  ");
    console.print(meta.minPeriod_uS);
    console.print("  ");
    console.print(meta.maxPeriod_uS);
    console.print("  ");
    console.print(meta.power_mA / 1024.0f, 3);   // 16Q10 fixed point
    console.print("  ");
    console.println(meta.vendorId);
  }
}

void runStep(uint32_t intervalUs)
{
  // everything off, then all four on at this interval
  for (int i = 0; i < kSensorCount; ++i) {
    bno.enableReport(kSensors[i].id, 0);
  }
  serviceFor(200);
  for (int i = 0; i < kSensorCount; ++i) {
    bno.enableReport(kSensors[i].id, intervalUs);
    delay(50);
  }
  serviceFor(300);

  for (int i = 0; i < kSensorCount; ++i) {
    counts[i] = 0;
  }
  uint32_t windowMs = 3 * (intervalUs / 1000);   // at least 3 intervals, and at least 3 s
  if (windowMs < 3000) {
    windowMs = 3000;
  }
  serviceFor(windowMs);

  uint32_t got[kSensorCount];
  for (int i = 0; i < kSensorCount; ++i) {
    got[i] = counts[i];
  }

  console.print("== requested ");
  console.print(intervalUs / 1000.0f, 1);
  console.print(" ms, counted over ");
  console.print(windowMs);
  console.println(" ms");
  console.println("sensor    set_interval_us  reports  measured_period_ms");
  for (int i = 0; i < kSensorCount; ++i) {
    sh2_SensorConfig_t cfg;
    const int rc = sh2_getSensorConfig(kSensors[i].id, &cfg);
    console.print(kSensors[i].name);
    console.print("  ");
    if (rc == SH2_OK) {
      console.print(cfg.reportInterval_us);
    } else {
      console.print("readback failed ");
      console.print(rc);
    }
    console.print("  ");
    console.print(got[i]);
    console.print("  ");
    if (got[i] > 0) {
      console.println((float)windowMs / got[i], 1);
    } else {
      console.println("none");
    }
  }
}

void setup()
{
  console.begin(115200);
  delay(500);
  console.println("t06 imu report rates start");

  // same bus setup as the flight code: pins before begin, or Wire takes PA9 (the led)
  Wire.setSDA(PA15);
  Wire.setSCL(PB15);
  Wire.begin();

  if (!bno.begin(0x4A, Wire, -1, -1)) {
    console.println("imu not found");
    while (true) {
      delay(1000);
    }
  }
  sh2_setSensorCallback(onEvent, nullptr);
  serviceFor(300);

  printMetadata();
  for (int i = 0; i < kIntervalCount; ++i) {
    runStep(kIntervalsUs[i]);
  }

  for (int i = 0; i < kSensorCount; ++i) {
    bno.enableReport(kSensors[i].id, 0);
  }
  console.println("DONE");
}

void loop()
{
}
