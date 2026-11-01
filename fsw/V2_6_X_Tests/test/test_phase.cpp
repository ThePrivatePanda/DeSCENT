// Phase.cpp on the laptop: a mount, a climb, a hover, a fall and a landing, replayed as the sensors
// would report them, the knocks and dead sensors in between, the ways down that leave a board high
// (a roof, a car ride up a hill), and the real drops of 2026-10-13 and 2026-10-17.
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
    case Phase::Ground: return "ground";
    case Phase::Air: return "air";
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
    runImu(forMs, alt, [accel](uint32_t) { return accel; }, gyro, imu);
  }

  // the same with the acceleration a function of time, for a drone whose vibration comes and goes
  template <typename Alt, typename Accel>
  void runImu(uint32_t forMs, Alt alt, Accel accel, float gyro, bool imu = true)
  {
    const uint32_t until = now + forMs;
    while (now < until) {
      if (imu) {
        f.imuSample(now, accel(now), gyro);
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
    expect(f, Phase::Ground, "on the bench");
    check(f.altitudeSource() == AltitudeSource::Barometer, "the barometer decides");
    s.run(20000, [](uint32_t t) { return 120.0f + 2.0f * sinf(t / 700.0f); }, 4.0f, 2.0f);   // picked up, carried, bumped
    expect(f, Phase::Ground, "handled on the way to the drone");
    s.run(3000000, [](uint32_t t) { return 120.0f + 6.0f * sinf(t / 300000.0f); }, 0.05f, 0.0f);   // weather over 50 min
    expect(f, Phase::Ground, "an hour mounted, with the weather drifting");
    // the drone runs on the ground for ten minutes: vibration, no climb
    s.run(600000, [](uint32_t t) { return 120.0f + 1.0f * sinf(t / 100.0f); }, 6.0f, 1.0f);
    expect(f, Phase::Ground, "a drone running on the ground is not a flight");
    check(!f.changed(), "nothing changed, so nothing to transmit about");
  }

  // The drone takes off and climbs at 3 m/s: the air once it is 15 m up, within a few seconds
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(4000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Ground, "12 m up is not yet the air");
    s.run(2000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "18 m up is the air");
    check(f.changed(), "the change is reported once");
    check(!f.changed(), "and only once");
    check(f.aboveGroundM() > 15.0f && f.aboveGroundM() < 20.0f, "the height above the ground reads");

    // hover at 300 m, the drone's vibration all the while, no release: still the air
    s.run(120000, [](uint32_t t) { return 300.0f + 3.0f * sinf(t / 900.0f); }, 5.0f, 1.0f);
    expect(f, Phase::Air, "hovering on the drone is still the air");

    // a quadcopter holding dead still at 300 m for two minutes, barometer flat: still up there. Under
    // kGroundQuietMs, and a real drone's IMU is never this quiet (see highUp below)
    s.run(120000, [](uint32_t) { return 300.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "two minutes of dead-still hover above half the climb is still the air");

    // the drone descends to 60 m up and hovers there, still: below half the climb, so the ground,
    // and the next climb from there is the air again
    const uint32_t d = s.now;
    s.run(60000, [d](uint32_t t) { return 300.0f - 4.0f * (t - d) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "descending on the drone is still the air");
    s.run(31000, [](uint32_t) { return 60.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Ground, "30 s dead still at 60 m, below half of what it climbed, is the ground");
    check(f.changed(), "reported");
    const uint32_t r = s.now;
    s.run(8000, [r](uint32_t t) { return 60.0f + 3.0f * (t - r) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "climbing again from there is the air again: the ground re-arms");

    // the release and the fall: nothing changes, it is still in the air
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    const uint32_t rel = s.now;
    s.run(20000, [rel](uint32_t t) { return 300.0f - 10.0f * (t - rel) / 1000.0f; }, 9.0f, 8.0f);
    expect(f, Phase::Air, "falling at 10 m/s is the same phase as flying: in the air");
    check(f.verticalRateMps() < -8.0f, "the vertical rate reads the fall");

    // on the ground: a few seconds of tumbling in the grass, then still
    s.run(3000, [](uint32_t) { return 100.0f; }, 3.0f, 1.0f);
    s.run(25000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    expect(f, Phase::Air, "25 s still is not yet the ground");
    s.run(6000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    expect(f, Phase::Ground, "30 s still after a fall is the ground");
    check(f.changed(), "reported");

    // picked up and carried: still landed. Walked up a hill: not a climb of 15 m at once
    s.run(30000, [](uint32_t t) { return 100.0f + 1.5f * sinf(t / 500.0f); }, 4.0f, 2.0f);
    expect(f, Phase::Ground, "carried about after landing is still the ground");
  }

  // The IMU is dead: the barometer alone, and landing takes two minutes still
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "on the ground with no IMU");
    const uint32_t t0 = s.now;
    s.run(10000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "a climb is a climb, IMU or not");
    s.run(20000, [](uint32_t) { return 300.0f; }, 0.0f, 0.0f, false);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 0.0f, 0.0f, false);
    s.run(100000, [](uint32_t) { return 90.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "100 s still on the barometer alone is not enough");
    s.run(25000, [](uint32_t) { return 90.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "120 s still on the barometer alone is the ground");
  }

  // The drone lands with the sat still on it: the ground, and a later take-off is the air again
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(10000, [](uint32_t) { return 120.0f; }, 0.1f, 0.0f);
    const uint32_t t0 = s.now;
    s.run(10000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "up");
    const uint32_t t1 = s.now;
    s.run(15000, [t1](uint32_t t) { return 150.0f - 2.0f * (t - t1) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "a drone descending at 2 m/s is still in the air");
    s.run(35000, [](uint32_t) { return 120.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "landed on the drone, motors off, is the ground");
    const uint32_t t2 = s.now;
    s.run(8000, [t2](uint32_t t) { return 120.0f + 3.0f * (t - t2) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "and the next take-off is the air, from the new baseline");
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
    expect(f, Phase::Ground, "GPS wandering 10 m on the ground is not a flight");
    check(f.altitudeSource() == AltitudeSource::Gps, "GPS decides");
    const uint32_t t0 = s.now;
    s.run(7000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Ground, "21 m up on GPS is not yet the air: 30 is the line");
    s.run(9000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "48 m up on GPS is the air, a second's fix behind");
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(35000, [](uint32_t t) { return 100.0f + 3.0f * sinf(t / 2000.0f); }, 0.8f, 0.1f);   // GPS wanders 6 m lying still
    expect(f, Phase::Ground, "landed on GPS, its wander within 8 m allowed: the ground");
  }

  // The barometer dies in flight: GPS takes over without a false landing
  {
    FlightPhase f;
    Sim s{f, 0};
    s.gps = true;
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(10000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "up on the barometer");
    check(f.altitudeSource() == AltitudeSource::Barometer, "the barometer decides while it is live");
    s.baro = false;
    s.run(120000, [](uint32_t) { return 300.0f; }, 0.5f, 0.05f);   // still hover, barometer gone
    expect(f, Phase::Air, "a still hover with the barometer gone is still in the air");
    check(f.altitudeSource() == AltitudeSource::Gps, "GPS took over");
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(35000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    expect(f, Phase::Ground, "and the landing is seen on GPS");
  }
}

static void blind()
{
  // Neither altitude source: the IMU alone. A running drone is the air, 30 s still is the ground.
  FlightPhase f;
  Sim s{f, 0};
  s.baro = false;
  s.run(60000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Ground, "still on the bench, blind");
  check(f.altitudeSource() == AltitudeSource::None, "nothing decides");
  s.run(5000, [](uint32_t) { return 0.0f; }, 4.0f, 2.0f);   // a knock, carried for 5 s
  s.run(20000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Ground, "5 s of handling in a 10 s window is not a flight");
  s.run(12000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
  expect(f, Phase::Air, "10 s of a running drone is the air, blind");
  s.run(120000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
  s.run(10000, [](uint32_t) { return 0.0f; }, 9.0f, 8.0f);   // the fall
  s.run(25000, [](uint32_t) { return 0.0f; }, 0.8f, 0.1f);
  expect(f, Phase::Air, "25 s still is not yet the ground");
  s.run(6000, [](uint32_t) { return 0.0f; }, 0.8f, 0.1f);
  expect(f, Phase::Ground, "30 s still is the ground, blind");
  s.run(12000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
  expect(f, Phase::Air, "picked up by a drone again: the air again");
}

// The ways down the halfway rule never sees: at rest above half the climb. The IMU quiet for
// kGroundQuietMs is the ground at any height, and only the IMU, so a dead IMU leaves it to altitude
static void highUp()
{
  // Flown up to a roof 130 m above the pad and left there: above half the climb, so 30 s still is
  // not the ground, three minutes is
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);   // to 300 m
    const uint32_t t1 = s.now;
    s.run(20000, [t1](uint32_t t) { return 300.0f - 2.5f * (t - t1) / 1000.0f; }, 5.0f, 1.0f);   // down to 250 m
    expect(f, Phase::Air, "up and down to a roof");
    s.run(60000, [](uint32_t) { return 250.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "a minute at rest on the roof, above half the climb, is still the air");
    s.run(119000, [](uint32_t) { return 250.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "2 min 59 s at rest is not yet the ground");
    s.run(1100, [](uint32_t) { return 250.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Ground, "three minutes at rest on the roof is the ground");
    check(f.changed(), "reported");
    check(fabsf(f.aboveGroundM()) < 1.0f, "the roof is the new ground");
    const uint32_t t2 = s.now;
    s.run(8000, [t2](uint32_t t) { return 250.0f + 3.0f * (t - t2) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "and a take-off from the roof is the air again");
  }

  // Driven up a hill: 60 m in two minutes on a bumpy road is the air by the climb (a slower road is
  // followed by the baseline). Parked at the top it's the ground three minutes later, a knock resets
  // the count
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.runImu(120000, [t0](uint32_t t) { return 120.0f + 0.5f * (t - t0) / 1000.0f; },
             [](uint32_t t) { return (t / 1000) % 3 == 0 ? 3.0f : 1.0f; }, 0.2f);
    expect(f, Phase::Air, "driven 60 m up a hill is the air by the barometer");
    s.run(100000, [](uint32_t) { return 180.0f; }, 0.3f, 0.02f);   // parked
    s.run(1000, [](uint32_t) { return 180.0f; }, 4.0f, 1.0f);      // a door shut
    s.run(170000, [](uint32_t) { return 180.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "the quiet starts again after a knock: 170 s since is not yet the ground");
    s.run(11000, [](uint32_t) { return 180.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "three minutes parked at the top is the ground");
  }

  // A drone hovering ten minutes: its vibration dips under the quiet line now and then, never for
  // long, so it stays in the air however long it hovers
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(30000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    s.runImu(600000, [](uint32_t) { return 210.0f; },
             [](uint32_t t) { return (t / 5000) % 4 == 0 ? 0.5f : 3.0f; }, 0.2f);   // 5 s calm in every 20
    expect(f, Phase::Air, "ten minutes hovering, calm 5 s in every 20, is still the air");
  }

  // With the IMU dead the three minutes do not apply: nothing says it is at rest but the altitude,
  // and the altitude says it is still up there
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    s.run(400000, [](uint32_t) { return 290.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "no IMU, at rest high up for seven minutes: still the air");
  }
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
    Phase at430 = Phase::Ground, at471 = Phase::Ground, at560 = Phase::Ground, at1055 = Phase::Ground;
    int changes = 0;
    unsigned long landed1 = 0, landed2 = 0, rising1 = 0, rising2 = 0;
    while (fscanf(in, "%lu,%f,%f,%d,%f\n", &ms, &accel, &gyro, &fix, &alt) == 5) {
      f.imuSample(ms, accel, gyro);
      if (withGps && fix) {
        f.gpsSample(ms, alt);
      }
      if (f.changed()) {
        changes++;
        if (f.phase() == Phase::Ground) {
          (landed1 == 0 ? landed1 : landed2) = ms;
        } else if (f.phase() == Phase::Air) {
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
    check(rising1 > 150000 && rising1 < 230000, withGps ? "GPS: the air during the first climb" : "blind: the air during the first climb");
    check(at430 == Phase::Air && at471 == Phase::Air, withGps ? "GPS: in the air through release 1 and the fall" : "blind: in the air through release 1 and the fall");
    check(landed1 > 471000 && landed1 < 510000, withGps ? "GPS: landed within 40 s of impact 1" : "blind: landed within 40 s of impact 1");
    check(at560 == Phase::Ground, withGps ? "GPS: still landed on the ground" : "blind: still landed on the ground");
    check(rising2 > 840000 && rising2 < 940000, withGps ? "GPS: the air during the second climb" : "blind: the air during the second climb");
    check(at1055 == Phase::Air, withGps ? "GPS: in the air through release 2" : "blind: in the air through release 2");
    check(landed2 > 1062000, withGps ? "GPS: landed after the second fall" : "blind: landed after the second fall");
  }
}

// The two drops of 2026-10-17 from board 9's flash (2026-10-18 pull 1432-board9): boot 24 is drop 1,
// boot 32 drop 2, barometer live, GPS never fixed. The barometer is filled in on a straight line
// between records, as the board read it every 200 ms; with one reading per 5 s record the baseline
// lags and the drift before drop 2 reads as the climb 20 s early. After Landed the board logged
// every 10 s, so the ground can only show on the next record
// Timelines from the 2026-10-17 field notes: boot 24 air +177 s, release +292.9 s,
// touchdown +309.8 s; boot 32 air +84 s, release +382.5 s, touchdown +393.7 s. Wanted: the air
// from the climb through the fall, and the ground within 40 s of touchdown
static void recordedBoard9()
{
  FILE *in = fopen("test/data/drop-2026-10-17-board9.csv", "r");
  check(in != nullptr, "test/data/drop-2026-10-17-board9.csv opens (run from V2_6_X_Tests)");
  if (in == nullptr) {
    return;
  }
  struct Drop { int boot; unsigned long climb, release, touchdown; };
  const Drop drops[2] = {{24, 177000, 292900, 309800}, {32, 84000, 382500, 393700}};
  FlightPhase f;
  int boot = -1;
  int changes[2] = {0, 0};
  unsigned long air[2] = {0, 0}, ground[2] = {0, 0}, envMs = 0;
  bool airAtRelease[2] = {false, false}, airAtTouchdown[2] = {false, false};
  float lastAlt = NAN;
  int b, imu, baro;
  unsigned long ms;
  float accel, gyro, alt;
  while (fscanf(in, "%d,%lu,%f,%f,%d,%d,%f\n", &b, &ms, &accel, &gyro, &imu, &baro, &alt) == 7) {
    if (b != boot) {
      f = FlightPhase();   // a reset
      boot = b;
      lastAlt = NAN;
    }
    const int d = boot == drops[0].boot ? 0 : 1;
    if (imu) {
      f.imuSample(ms, accel, gyro);
    } else {
      f.imuLost(ms);
    }
    // a record carries the newest barometer reading, which is new every 200 ms
    if (baro && !std::isnan(lastAlt) && ms - envMs > 400) {
      for (unsigned long t = envMs + 200; t + 200 <= ms; t += 200) {
        f.envSample(t, lastAlt + (alt - lastAlt) * static_cast<float>(t - envMs) / static_cast<float>(ms - envMs));
      }
    }
    if (baro && (alt != lastAlt || ms - envMs >= 200)) {
      f.envSample(ms, alt);
      lastAlt = alt;
      envMs = ms;
    }
    if (f.changed()) {
      changes[d]++;
      (f.phase() == Phase::Air ? air[d] : ground[d]) = ms;
    }
    if (ms <= drops[d].release) airAtRelease[d] = f.phase() == Phase::Air;
    if (ms <= drops[d].touchdown) airAtTouchdown[d] = f.phase() == Phase::Air;
  }
  fclose(in);
  for (int d = 0; d < 2; d++) {
    printf("board 9 boot %d: %d changes, air %lu, ground %lu\n", drops[d].boot, changes[d], air[d], ground[d]);
    char what[96];
    snprintf(what, sizeof what, "board 9 drop %d: the air on the climb, then the ground, two changes only", d + 1);
    check(changes[d] == 2 && air[d] > drops[d].climb - 10000 && air[d] < drops[d].climb + 10000, what);
    snprintf(what, sizeof what, "board 9 drop %d: still the air at the release and through the fall", d + 1);
    check(airAtRelease[d] && airAtTouchdown[d], what);
    snprintf(what, sizeof what, "board 9 drop %d: the ground within 30 s of rest after touchdown, and the record gap", d + 1);
    check(ground[d] > drops[d].touchdown && ground[d] < drops[d].touchdown + 45000, what);
  }
}

// After a reset: log in the air, radio on the ground (decided 2026-10-22)
static void bootAir()
{
  // left still
  {
    FlightPhase f;
    Sim s{f, 0};
    check(f.logsAir() && f.phase() == Phase::Ground, "boot: the log in the air, the radio on the ground");
    bool txGround = true;
    for (int i = 0; i < 25; i++) {
      s.run(1000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
      txGround = txGround && f.phase() == Phase::Ground;
    }
    check(f.logsAir() && txGround, "boot, 25 s still: the log still in the air, the radio on the ground");
    check(!f.logChanged(), "boot, 25 s still: the log has not changed");
    s.run(6000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    check(!f.logsAir(), "boot, 30 s still: the log is on the ground");
    check(f.logChanged() && !f.logChanged(), "boot, 30 s still: the log change is reported once");
    check(f.phase() == Phase::Ground && !f.changed(), "boot, 30 s still: the radio stayed on the ground throughout, nothing to transmit about");
    s.run(600000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    check(!f.logsAir() && f.phase() == Phase::Ground, "boot, ten minutes still: both on the ground");
  }

  // handled two minutes
  {
    FlightPhase f;
    Sim s{f, 0};
    bool txGround = true;
    for (int i = 0; i < 120; i++) {
      s.run(1000, [](uint32_t t) { return 120.0f + 2.0f * sinf(t / 700.0f); }, 4.0f, 2.0f);
      txGround = txGround && f.phase() == Phase::Ground;
    }
    check(f.logsAir() && txGround && !f.changed(), "boot, handled two minutes: the log in the air, the radio on the ground");
    s.run(31000, [](uint32_t) { return 120.0f; }, 0.2f, 0.02f);
    check(!f.logsAir() && f.phase() == Phase::Ground, "boot, handled then put down 30 s: the log on the ground");
  }

  // a real climb
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(5000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(4000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    check(f.phase() == Phase::Ground && f.logsAir(), "boot, 12 m up: the radio not yet in the air, the log is");
    s.run(2000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    check(f.phase() == Phase::Air && f.logsAir(), "boot, 18 m up: the air for the radio and the log");
    check(f.changed() && !f.logChanged(), "boot, climbed: the radio's change reported, the log never left the air");
    // and lands
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(31000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    check(f.phase() == Phase::Ground && !f.logsAir() && f.changed() && f.logChanged(), "boot, flown and landed: both on the ground");
  }

  // reset mid-fall at 300 m, 10 m/s
  {
    FlightPhase f;
    Sim s{f, 0};
    const uint32_t t0 = s.now;
    s.run(20000, [t0](uint32_t t) { return 300.0f - 10.0f * (t - t0) / 1000.0f; }, 9.0f, 8.0f);
    check(f.logsAir() && f.phase() == Phase::Ground, "reset mid-fall: the fall is logged as the air, the radio on the ground");
    s.run(3000, [](uint32_t) { return 100.0f; }, 6.0f, 3.0f);   // the impact and tumbling
    s.run(25000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    check(f.logsAir(), "reset mid-fall: the impact and 25 s after it still logged as the air");
    s.run(6000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    check(!f.logsAir() && f.phase() == Phase::Ground && !f.changed(), "reset mid-fall: 30 s at rest is the ground, the radio never changed");
  }

  // no altitude source
  {
    FlightPhase f;
    Sim s{f, 0};
    s.baro = false;
    s.run(25000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
    check(f.logsAir(), "blind boot, 25 s still: still logged as the air");
    s.run(6000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
    check(!f.logsAir() && f.phase() == Phase::Ground, "blind boot, 30 s still: the log on the ground");
  }

  // IMU dead, barometer only
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(100000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    check(f.logsAir(), "boot without an IMU, 100 s still: still logged as the air");
    s.run(25000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    check(!f.logsAir(), "boot without an IMU, 120 s still: the log on the ground");
  }
}

// nothing to judge by: boot-air ends at kBootAirMaxMs anyway
static void bootAirTimeout()
{
  FlightPhase f;
  Sim s{f, 0};
  s.baro = false;
  s.run(299000, [](uint32_t) { return 0.0f; }, 0.0f, 0.0f, false);
  check(f.logsAir() && !f.logChanged(), "no IMU, no altitude, 4:59: still logged as the air");
  s.run(1100, [](uint32_t) { return 0.0f; }, 0.0f, 0.0f, false);
  check(!f.logsAir() && f.logChanged() && f.phase() == Phase::Ground && !f.changed(), "no IMU, no altitude, 5:00: logged as the ground, radio unchanged");
  s.run(30000, [](uint32_t) { return 0.0f; }, 0.0f, 0.0f, false);
  check(!f.logsAir(), "and stays the ground");
  // the barometer comes back at 5:30 and the drone climbs at 6:00
  s.baro = true;
  s.run(30000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
  check(!f.logsAir() && f.phase() == Phase::Ground, "barometer back, still: ground");
  const uint32_t t0 = s.now;
  s.run(8000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
  check(f.phase() == Phase::Air && f.logsAir() && f.changed(), "a climb at 6:00: the air for both");
}

int main()
{
  bootAir();
  bootAirTimeout();
  onTheBarometer();
  onGps();
  blind();
  highUp();
  recorded();
  recordedBoard9();
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
