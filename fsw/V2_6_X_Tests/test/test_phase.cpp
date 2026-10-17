// Phase.cpp on the laptop: a mount, a climb, a hover, a fall and a landing, replayed as the sensors
// would report them, the knocks and dead sensors in between, and the real drops of 2026-10-13.
// Run from V2_6_X_Tests, so the recorded flight under test/data is found:
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src -I../V2_6_X_Code/src/app test/test_phase.cpp ../V2_6_X_Code/src/app/Phase.cpp -o /tmp/phase && /tmp/phase
#include <cstdio>
#include <cmath>
#include "Phase.h"

using ChipSatApp::AltitudeSource;
using ChipSatApp::FlightPhase;
using ChipSatApp::Phase;

static int checks = 0;
static int failures = 0;

static void check(bool ok, const char *what)
{
  ++checks;
  if (!ok) {
    ++failures;
    printf("FAIL %s\n", what);
  }
}

static const char *name(Phase p)
{
  switch (p) {
    case Phase::Waiting: return "waiting";
    case Phase::Rising: return "rising";
    case Phase::Falling: return "falling";
    case Phase::Landed: return "landed";
  }
  return "?";
}

static void expect(const FlightPhase &f, Phase want, const char *what)
{
  ++checks;
  if (f.phase() != want) {
    ++failures;
    printf("FAIL %s: is %s, wanted %s\n", what, name(f.phase()), name(want));
  }
}

// The sensors as the controller feeds them: the IMU every 50 ms, the barometer every 200 ms, GPS
// every second. alt is a function of time; accel and gyro are what the IMU reads throughout.
// Which sources are alive is set per run: imu false means it is dead, baro and gps likewise
struct Sim
{
  FlightPhase &f;
  uint32_t now;
  bool baro = true;
  bool gps = false;
  uint32_t nextEnv = 0;
  uint32_t nextGps = 0;

  template <typename Alt>
  void run(uint32_t forMs, Alt alt, float accel, float gyro, bool imu = true)
  {
    const uint32_t until = now + forMs;
    while (now < until) {
      if (imu) {
        f.imuSample(now, accel, gyro);
      } else {
        f.imuLost(now);
      }
      if (baro && now >= nextEnv) {
        f.envSample(now, alt(now));
        nextEnv = now + 200;
      }
      if (gps && now >= nextGps) {
        f.gpsSample(now, alt(now) + 40.0f);   // sea level differs from the barometer's: only differences count
        nextGps = now + 1000;
      }
      now += 50;
    }
  }
};

static void onTheBarometer()
{
  // Switched on, carried to the drone and handled: an hour of waiting, and nothing a hand does
  // climbs 15 m, so none of it looks like a flight
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(600000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Waiting, "on the bench");
    check(f.altitudeSource() == AltitudeSource::Barometer, "the barometer decides");
    s.run(20000, [](uint32_t t) { return 120.0f + 2.0f * sinf(t / 700.0f); }, 4.0f, 2.0f);   // picked up, carried, bumped
    expect(f, Phase::Waiting, "handled on the way to the drone");
    s.run(3000000, [](uint32_t t) { return 120.0f + 6.0f * sinf(t / 300000.0f); }, 0.05f, 0.0f);   // weather over 50 min
    expect(f, Phase::Waiting, "an hour mounted, with the weather drifting");
    // the drone runs on the ground for ten minutes: vibration, no climb
    s.run(600000, [](uint32_t t) { return 120.0f + 1.0f * sinf(t / 100.0f); }, 6.0f, 1.0f);
    expect(f, Phase::Waiting, "a drone running on the ground is not a flight");
    check(!f.changed(), "nothing changed, so nothing to transmit about");
  }

  // The drone takes off and climbs at 3 m/s: rising once it is 15 m up, within a few seconds
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(4000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Waiting, "12 m up is not yet rising");
    s.run(2000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "18 m up is rising");
    check(f.changed(), "the change is reported once");
    check(!f.changed(), "and only once");
    check(f.aboveGroundM() > 15.0f && f.aboveGroundM() < 20.0f, "the height above the ground reads");

    // hover at 300 m, the drone's vibration all the while, no release: still rising
    s.run(120000, [](uint32_t t) { return 300.0f + 3.0f * sinf(t / 900.0f); }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "hovering on the drone is still rising");

    // a quadcopter holding dead still at 300 m for two minutes, barometer flat: still up there
    s.run(120000, [](uint32_t) { return 300.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Rising, "a dead-still hover above half the climb is not landed");

    // the drone descends to 60 m up and hovers there, still: below half the climb, so landed, and
    // the next climb from there is rising again
    const uint32_t d = s.now;
    s.run(60000, [d](uint32_t t) { return 300.0f - 4.0f * (t - d) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "descending on the drone is still rising");
    s.run(31000, [](uint32_t) { return 60.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Landed, "30 s dead still at 60 m, below half of what it climbed, is landed");
    check(f.changed(), "reported");
    const uint32_t r = s.now;
    s.run(8000, [r](uint32_t t) { return 60.0f + 3.0f * (t - r) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "climbing again from there is rising again: landed re-arms");

    // the release and the fall: nothing changes, it is still in the air
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    const uint32_t rel = s.now;
    s.run(20000, [rel](uint32_t t) { return 300.0f - 10.0f * (t - rel) / 1000.0f; }, 9.0f, 8.0f);
    expect(f, Phase::Rising, "falling at 10 m/s is the same phase as flying: in the air");
    check(f.verticalRateMps() < -8.0f, "the vertical rate reads the fall");

    // on the ground: a few seconds of tumbling in the grass, then still
    s.run(3000, [](uint32_t) { return 100.0f; }, 3.0f, 1.0f);
    s.run(25000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    expect(f, Phase::Rising, "25 s still is not yet landed");
    s.run(6000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    expect(f, Phase::Landed, "30 s still after a fall is landed");
    check(f.changed(), "reported");

    // picked up and carried: still landed. Walked up a hill: not a climb of 15 m at once
    s.run(30000, [](uint32_t t) { return 100.0f + 1.5f * sinf(t / 500.0f); }, 4.0f, 2.0f);
    expect(f, Phase::Landed, "carried about after landing is still landed");
  }

  // The IMU is dead: the barometer alone, and landing takes two minutes still
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Waiting, "waiting with no IMU");
    const uint32_t t0 = s.now;
    s.run(10000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Rising, "a climb is a climb, IMU or not");
    s.run(20000, [](uint32_t) { return 300.0f; }, 0.0f, 0.0f, false);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 0.0f, 0.0f, false);
    s.run(100000, [](uint32_t) { return 90.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Rising, "100 s still on the barometer alone is not enough");
    s.run(25000, [](uint32_t) { return 90.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Landed, "120 s still on the barometer alone is landed");
  }

  // The drone lands with the sat still on it: landed, and a later take-off is rising again
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(10000, [](uint32_t) { return 120.0f; }, 0.1f, 0.0f);
    const uint32_t t0 = s.now;
    s.run(10000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "up");
    const uint32_t t1 = s.now;
    s.run(15000, [t1](uint32_t t) { return 150.0f - 2.0f * (t - t1) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "a drone descending at 2 m/s is still in the air");
    s.run(35000, [](uint32_t) { return 120.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Landed, "landed on the drone, motors off, is landed");
    const uint32_t t2 = s.now;
    s.run(8000, [t2](uint32_t t) { return 120.0f + 3.0f * (t - t2) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "and the next take-off is rising, from the new baseline");
  }
}

static void onGps()
{
  // No barometer: GPS altitude decides, with its own wider margins. The first fix comes late
  {
    FlightPhase f;
    Sim s{f, 0};
    s.baro = false;
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    check(f.altitudeSource() == AltitudeSource::None, "no fix yet: nothing decides");
    s.gps = true;
    s.run(60000, [](uint32_t t) { return 120.0f + 5.0f * sinf(t / 3000.0f); }, 0.1f, 0.01f);   // GPS wander, 10 m peak to peak
    expect(f, Phase::Waiting, "GPS wandering 10 m on the ground is not a flight");
    check(f.altitudeSource() == AltitudeSource::Gps, "GPS decides");
    const uint32_t t0 = s.now;
    s.run(7000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Waiting, "21 m up on GPS is not yet rising: 30 is the line");
    s.run(9000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "48 m up on GPS is rising, a second's fix behind");
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(35000, [](uint32_t t) { return 100.0f + 3.0f * sinf(t / 2000.0f); }, 0.8f, 0.1f);   // GPS wanders 6 m lying still
    expect(f, Phase::Landed, "landed on GPS, its wander within 8 m allowed");
  }

  // The barometer dies in flight: GPS takes over without a false landing
  {
    FlightPhase f;
    Sim s{f, 0};
    s.gps = true;
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(10000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Rising, "up on the barometer");
    check(f.altitudeSource() == AltitudeSource::Barometer, "the barometer decides while it is live");
    s.baro = false;
    s.run(120000, [](uint32_t) { return 300.0f; }, 0.5f, 0.05f);   // still hover, barometer gone
    expect(f, Phase::Rising, "a still hover with the barometer gone is still in the air");
    check(f.altitudeSource() == AltitudeSource::Gps, "GPS took over");
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(35000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    expect(f, Phase::Landed, "and the landing is seen on GPS");
  }
}

static void blind()
{
  // Neither altitude source: the IMU alone. A running drone is the air, 30 s still is the ground.
  // The controller logs at the blind gap whatever this says; this only sets the transmit gap
  FlightPhase f;
  Sim s{f, 0};
  s.baro = false;
  s.run(60000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Waiting, "still on the bench, blind");
  check(f.altitudeSource() == AltitudeSource::None, "nothing decides");
  s.run(5000, [](uint32_t) { return 0.0f; }, 4.0f, 2.0f);   // a knock, carried for 5 s
  s.run(20000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Waiting, "5 s of handling in a 10 s window is not a flight");
  s.run(12000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
  expect(f, Phase::Rising, "10 s of a running drone is the air, blind");
  s.run(120000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
  s.run(10000, [](uint32_t) { return 0.0f; }, 9.0f, 8.0f);   // the fall
  s.run(25000, [](uint32_t) { return 0.0f; }, 0.8f, 0.1f);
  expect(f, Phase::Rising, "25 s still is not yet landed");
  s.run(6000, [](uint32_t) { return 0.0f; }, 0.8f, 0.1f);
  expect(f, Phase::Landed, "30 s still is landed, blind");
  s.run(12000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
  expect(f, Phase::Rising, "picked up by a drone again: rising again");
}

// The two drops of 2026-10-13 as board 13 reported them over the radio, about three samples a
// second with loss, GPS altitude as the only altitude (its barometer was dead all day). What is
// wanted: in the air through both releases and falls, landed 30 s after each impact, never
// landed on the drone, and the same with no altitude at all
static void recorded()
{
  for (int withGps = 1; withGps >= 0; withGps--) {
    FILE *in = fopen("test/data/drop-2026-10-13-board13.csv", "r");
    check(in != nullptr, "test/data/drop-2026-10-13-board13.csv opens (run from V2_6_X_Tests)");
    if (in == nullptr) {
      return;
    }
    FlightPhase f;
    unsigned long ms;
    float accel, gyro, alt;
    int fix;
    Phase at430 = Phase::Waiting, at471 = Phase::Waiting, at560 = Phase::Waiting, at1055 = Phase::Waiting;
    int changes = 0;
    unsigned long landed1 = 0, landed2 = 0, rising1 = 0, rising2 = 0;
    while (fscanf(in, "%lu,%f,%f,%d,%f\n", &ms, &accel, &gyro, &fix, &alt) == 5) {
      f.imuSample(ms, accel, gyro);
      if (withGps && fix) {
        f.gpsSample(ms, alt);
      }
      if (f.changed()) {
        changes++;
        if (f.phase() == Phase::Landed) {
          (landed1 == 0 ? landed1 : landed2) = ms;
        } else if (f.phase() == Phase::Rising) {
          (rising1 == 0 ? rising1 : rising2) = ms;
        }
      }
      if (ms <= 430000) at430 = f.phase();
      if (ms <= 471000) at471 = f.phase();
      if (ms <= 560000) at560 = f.phase();
      if (ms <= 1055000) at1055 = f.phase();
    }
    fclose(in);
    const char *how = withGps ? "on GPS" : "blind";
    printf("%s: %d changes, rising %lu/%lu, landed %lu/%lu\n", how, changes, rising1, rising2, landed1, landed2);
    check(changes == 4, withGps ? "GPS: four changes, no flip-flop on the drone" : "blind: four changes, no flip-flop on the drone");
    check(rising1 > 150000 && rising1 < 230000, withGps ? "GPS: rising during the first climb" : "blind: rising during the first climb");
    check(at430 == Phase::Rising && at471 == Phase::Rising, withGps ? "GPS: in the air through release 1 and the fall" : "blind: in the air through release 1 and the fall");
    check(landed1 > 471000 && landed1 < 510000, withGps ? "GPS: landed within 40 s of impact 1" : "blind: landed within 40 s of impact 1");
    check(at560 == Phase::Landed, withGps ? "GPS: still landed on the ground" : "blind: still landed on the ground");
    check(rising2 > 840000 && rising2 < 940000, withGps ? "GPS: rising during the second climb" : "blind: rising during the second climb");
    check(at1055 == Phase::Rising, withGps ? "GPS: in the air through release 2" : "blind: in the air through release 2");
    check(landed2 > 1062000, withGps ? "GPS: landed after the second fall" : "blind: landed after the second fall");
  }
}

int main()
{
  onTheBarometer();
  onGps();
  blind();
  recorded();
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
