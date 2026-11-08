// Phase.cpp on the laptop: a mount, a climb, a hover, a fall and a landing, replayed as the sensors
// would report them, the knocks and dead sensors in between, the ways down that leave a board high
// (a roof, a car ride up a hill), and the real drops of 2026-10-13 and 2026-10-17.
// Run from V2_6_X_Tests, so the recorded flight under test/data is found:
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src -I../V2_6_X_Code/src/app test/test_phase.cpp ../V2_6_X_Code/src/app/Phase.cpp -o /tmp/phase && /tmp/phase
#include <cstdio>
#include <cmath>
#include <vector>
#include "Phase.h"
#include "log/SystemEvent.h"

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
// Which sources are alive is set per run: imu false means it is missing, baro and gps likewise.
// paced feeds the IMU at the rate the phase asks for, once a second at 1 Hz, as the board does.
// sawAir and sawGround say whether the phase was ever that since they were last cleared
struct Sim
{
  FlightPhase &f;
  uint32_t now;
  bool baro = true;
  bool gps = false;
  bool paced = false;
  uint32_t nextEnv = 0;
  uint32_t nextGps = 0;
  bool sawAir = false;
  bool sawGround = false;

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
      if (!imu) {
        f.imuLost(now);
      } else if (!paced || f.imuFast() || now % 1000 == 0) {
        f.imuSample(now, accel(now), gyro);
      }
      if (baro && now >= nextEnv) {
        f.envSample(now, alt(now));
        nextEnv = now + 200;
      }
      if (gps && now >= nextGps) {
        f.gpsSample(now, alt(now) + 40.0f);   // sea level differs from the barometer's: only differences count
        nextGps = now + 1000;
      }
      sawAir = sawAir || f.phase() == Phase::Air;
      sawGround = sawGround || f.phase() == Phase::Ground;
      now += 50;
    }
  }
};

static void onTheBarometer()
{
  // Switched on, carried to the drone and handled: an hour of waiting. Handled a few seconds at a
  // time it stays on the ground. A drone running its motors on the ground can't be told from a
  // flight by the IMU, so that is the air until 30 s after they stop
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(600000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Ground, "on the bench");
    check(f.altitudeSource() == AltitudeSource::Barometer, "the barometer is live");
    for (int i = 0; i < 4; i++) {   // picked up, looked at, put down
      s.run(5000, [](uint32_t t) { return 120.0f + 2.0f * sinf(t / 700.0f); }, 4.0f, 2.0f);
      s.run(5000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    }
    expect(f, Phase::Ground, "handled 5 s at a time on the way to the drone");
    s.run(3000000, [](uint32_t t) { return 120.0f + 6.0f * sinf(t / 300000.0f); }, 0.05f, 0.0f);   // weather over 50 min
    expect(f, Phase::Ground, "an hour mounted, with the weather drifting");
    check(!f.changed(), "nothing changed, so nothing to transmit about");
    s.run(600000, [](uint32_t t) { return 120.0f + 1.0f * sinf(t / 100.0f); }, 6.0f, 1.0f);
    expect(f, Phase::Air, "a drone running on the ground ten minutes is the air by the IMU");
    s.run(31000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Ground, "and the ground 30 s after its motors stop");
  }

  // The drone takes off and climbs at 3 m/s: the air 8 s into its motors, whatever the height
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(7000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Ground, "7 s of a running drone is not yet the air");
    s.run(2000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "8 s of it is the air");
    check(f.changed(), "the change is reported once");
    check(!f.changed(), "and only once");
    check(f.aboveGroundM() > 20.0f && f.aboveGroundM() < 30.0f, "the height above the ground reads");

    // hover at 300 m, the drone's vibration all the while, no release: still the air
    s.run(120000, [](uint32_t t) { return 300.0f + 3.0f * sinf(t / 900.0f); }, 5.0f, 1.0f);
    expect(f, Phase::Air, "hovering on the drone is still the air");

    // a quadcopter holding dead still at 300 m is not the ground: above half the climb that takes
    // kLandedHighMs quiet and flat, which a real drone's IMU never gives (see highUp below)
    s.run(31000, [](uint32_t) { return 300.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "30 s of dead-still hover at 180 m up is still the air");
    s.run(259000, [](uint32_t) { return 300.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "4 min 50 s of it is still the air");
    s.run(9000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "and the drone moving again is still the air");

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

    // picked up and carried back: a hand reads like a drone, but with the barometer live and no climb
    // it takes the window held kMotionHeldMs, and then the ground again once it is put down
    s.run(30000, [](uint32_t t) { return 100.0f + 1.5f * sinf(t / 500.0f); }, 4.0f, 2.0f);
    expect(f, Phase::Ground, "carried 30 s after landing with no climb is still the ground");
    s.run(40000, [](uint32_t t) { return 100.0f + 1.5f * sinf(t / 500.0f); }, 4.0f, 2.0f);
    expect(f, Phase::Air, "carried 70 s is the air: the window held a minute");
    s.run(31000, [](uint32_t) { return 100.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "put down, the ground 30 s later");
  }

  // The IMU is dead: the barometer alone once the IMU has been missing kImuDeadMs, and landing takes
  // two minutes still
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "on the ground with no IMU");
    const uint32_t t0 = s.now;
    s.run(20000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "a climb with the IMU missing 50 s holds the ground: it may come back");
    s.run(15000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "missing over a minute the IMU is dead, and a climb is the air");
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
  // No barometer, and the first GPS fix comes late. The IMU decides; GPS only says an altitude was
  // live, so the landing takes 30 s and not 3 min
  {
    FlightPhase f;
    Sim s{f, 0};
    s.baro = false;
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    check(f.altitudeSource() == AltitudeSource::None, "no fix yet: no altitude");
    s.gps = true;
    s.run(60000, [](uint32_t t) { return 120.0f + 5.0f * sinf(t / 3000.0f); }, 0.1f, 0.01f);   // GPS wander, 10 m peak to peak
    expect(f, Phase::Ground, "GPS wandering 10 m on the ground is not a flight");
    check(f.altitudeSource() == AltitudeSource::Gps, "GPS is live");
    const uint32_t t0 = s.now;
    s.run(7000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Ground, "7 s of a running drone is not yet the air");
    s.run(9000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "16 s of it is");
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(35000, [](uint32_t t) { return 100.0f + 3.0f * sinf(t / 2000.0f); }, 0.8f, 0.1f);   // GPS wanders 6 m lying still
    expect(f, Phase::Ground, "landed, GPS live in the air: the ground after 30 s");
  }

  // The barometer dies in flight: GPS takes over as the live altitude
  {
    FlightPhase f;
    Sim s{f, 0};
    s.gps = true;
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(10000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "up on the barometer");
    check(f.altitudeSource() == AltitudeSource::Barometer, "the barometer first while it is live");
    s.baro = false;
    s.run(120000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);   // hover, barometer gone
    expect(f, Phase::Air, "a hover with the barometer gone is still in the air");
    check(f.altitudeSource() == AltitudeSource::Gps, "GPS took over");
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(35000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    expect(f, Phase::Ground, "and the landing is seen on GPS");
  }
}

static void blind()
{
  // Neither altitude source all flight: a running drone is the air, and 3 min still the ground, as a
  // fall above 9 km that doesn't tumble has nothing else to hold it up
  FlightPhase f;
  Sim s{f, 0};
  s.baro = false;
  s.run(60000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Ground, "still on the bench, blind");
  check(f.altitudeSource() == AltitudeSource::None, "no altitude");
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
  expect(f, Phase::Air, "30 s still is not the ground with no altitude all flight");
  s.run(148000, [](uint32_t) { return 0.0f; }, 0.8f, 0.1f);
  expect(f, Phase::Air, "2 min 59 s still is not yet");
  s.run(2000, [](uint32_t) { return 0.0f; }, 0.8f, 0.1f);
  expect(f, Phase::Ground, "3 min still is the ground, blind");
  s.run(12000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
  expect(f, Phase::Air, "picked up by a drone again: the air again");
}

// The ways down that leave a board high: at rest above where it took off. The IMU decides at any
// height, so they land like any other. A dead IMU leaves it to altitude, which only lands low
static void highUp()
{
  // Flown up to a roof 130 m above the pad and left there: above half the climb, so the ground
  // kLandedHighMs after the last movement, and a knock starts it again
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);   // to 300 m
    const uint32_t t1 = s.now;
    s.run(20000, [t1](uint32_t t) { return 300.0f - 2.5f * (t - t1) / 1000.0f; }, 5.0f, 1.0f);   // down to 250 m
    expect(f, Phase::Air, "up and down to a roof");
    s.run(20000, [](uint32_t) { return 250.0f; }, 0.5f, 0.05f);
    s.run(1000, [](uint32_t) { return 250.0f; }, 4.0f, 1.0f);   // a gust knocks it over
    s.run(29000, [](uint32_t) { return 250.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "the quiet starts again after a knock: 29 s since is not yet the ground");
    s.run(2000, [](uint32_t) { return 250.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "30 s at rest on the roof, above half the climb, is still the air: it could be a hover");
    s.run(268000, [](uint32_t) { return 250.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "4 min 59 s after the knock: not yet");
    s.run(2000, [](uint32_t) { return 250.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Ground, "5 min quiet and flat on the roof is the ground");
    check(f.changed(), "reported");
    check(fabsf(f.aboveGroundM()) < 1.0f, "the roof is the new ground");
    const uint32_t t2 = s.now;
    s.run(8000, [t2](uint32_t t) { return 250.0f + 3.0f * (t - t2) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "and a take-off from the roof is the air again");
  }

  // Driven 60 m up a hill in two minutes on a bumpy road: the barometer climbs and the IMU moves a
  // third of the time, which is neither
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.runImu(120000, [t0](uint32_t t) { return 120.0f + 0.5f * (t - t0) / 1000.0f; },
             [](uint32_t t) { return (t / 1000) % 3 == 0 ? 3.0f : 1.0f; }, 0.2f);
    s.run(100000, [](uint32_t) { return 180.0f; }, 0.3f, 0.02f);   // parked
    check(!s.sawAir, "driven 60 m up a hill and parked is never the air");
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

// A desk board, IMU quiet, its barometer reading 46 m high on every other reading as boards did
// during 22 dBm transmits on 2026-11-01, or stepping up 46 m once. Ten minutes of either is never
// the air, with a 9 s IMU gap in the middle or without
static void barometerOffsets()
{
  for (int gap = 0; gap < 2; gap++) {
    auto alternating = [](uint32_t t) { return 340.0f + ((t / 200) % 2 ? 46.0f : 0.0f); };
    auto step = [](uint32_t t) { return t < 300000 ? 340.0f : 386.0f; };
    FlightPhase a;
    Sim sa{a, 0};
    sa.run(300000, alternating, 0.1f, 0.01f);
    if (gap) {
      sa.run(9000, alternating, 0.0f, 0.0f, false);
    }
    sa.run(300000, alternating, 0.1f, 0.01f);
    check(!sa.sawAir, gap ? "desk, barometer 46 m high every other reading, 9 s IMU gap: never the air"
                          : "desk, barometer 46 m high every other reading: never the air");
    FlightPhase b;
    Sim sb{b, 0};
    sb.run(295000, step, 0.1f, 0.01f);
    if (gap) {
      sb.run(9000, step, 0.0f, 0.0f, false);
    }
    sb.run(305000, step, 0.1f, 0.01f);
    check(!sb.sawAir, gap ? "desk, barometer stepping up 46 m, 9 s IMU gap: never the air"
                          : "desk, barometer stepping up 46 m: never the air");
  }
}

// A fall that doesn't tumble: the IMU quiet from the release, the barometer falling 60 s. Held in the
// air while it falls, at 10, 50 and 200 m/s and with the transmit offset on top, the ground 30 s
// after it stops
static void flatFall()
{
  const float rates[3] = {10.0f, 50.0f, 200.0f};
  for (int offset = 0; offset < 2; offset++) {
    for (float rate : rates) {
      const float off = offset ? 46.0f : 0.0f;
      const float top = 100.0f + rate * 60.0f;
      FlightPhase f;
      Sim s{f, 0};
      s.run(30000, [off](uint32_t t) { return 100.0f + ((t / 200) % 2 ? off : 0.0f); }, 0.1f, 0.01f);
      const uint32_t t0 = s.now;
      s.run(20000, [=](uint32_t t) { return 100.0f + (top - 100.0f) * (t - t0) / 20000.0f + ((t / 200) % 2 ? off : 0.0f); },
            5.0f, 1.0f);   // carried up
      const uint32_t rel = s.now;
      s.sawGround = false;
      s.run(60000, [=](uint32_t t) { return top - rate * (t - rel) / 1000.0f + ((t / 200) % 2 ? off : 0.0f); }, 0.3f, 0.02f);
      char what[96];
      snprintf(what, sizeof what, "flat fall at %.0f m/s%s: the air all the way down", rate, offset ? " with the offset" : "");
      check(f.phase() == Phase::Air && !s.sawGround, what);
      s.run(29000, [off](uint32_t t) { return 100.0f + ((t / 200) % 2 ? off : 0.0f); }, 0.3f, 0.02f);
      snprintf(what, sizeof what, "flat fall at %.0f m/s%s: 29 s after it stops, still the air", rate, offset ? " with the offset" : "");
      expect(f, Phase::Air, what);
      s.run(4000, [off](uint32_t t) { return 100.0f + ((t / 200) % 2 ? off : 0.0f); }, 0.3f, 0.02f);
      snprintf(what, sizeof what, "flat fall at %.0f m/s%s: the ground 30 s after it stops", rate, offset ? " with the offset" : "");
      expect(f, Phase::Ground, what);
    }
  }

  // Up on the drone and dead still, the barometer stepping down 46 m once or the offset coming and
  // going: neither is a fall, so the ground once it has been quiet and flat kLandedHighMs, above half
  // the climb
  for (int offset = 0; offset < 2; offset++) {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 100.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 100.0f + 5.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    const uint32_t still = s.now;
    auto alt = [offset, still](uint32_t t) {
      return offset ? 400.0f + ((t / 200) % 2 ? 46.0f : 0.0f) : (t < still + 10000 ? 400.0f : 354.0f);
    };
    s.run(29000, alt, 0.3f, 0.02f);
    expect(f, Phase::Air, offset ? "still up there, the offset coming and going: 29 s is not yet the ground"
                                 : "still up there, the barometer stepping down 46 m: 29 s is not yet the ground");
    s.run(2000, alt, 0.3f, 0.02f);
    expect(f, Phase::Air, offset ? "nor 30 s, high up" : "nor 30 s, high up");
    s.run(285000, alt, 0.3f, 0.02f);
    expect(f, Phase::Ground, offset ? "and 5 min quiet and flat is: the offset is no fall"
                                    : "and 5 min quiet and flat after the step is: a step is no fall");
  }
}

// No altitude live in the air: the barometer dead from the take-off, no GPS. 3 min quiet is the
// ground. A source live at any point of the air makes it 30 s
static void noAltitude()
{
  for (int comesBack = 0; comesBack < 2; comesBack++) {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    s.baro = false;
    s.run(20000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "no altitude: a running drone is the air");
    if (comesBack) {
      s.baro = true;
      s.run(5000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
      s.baro = false;
      s.run(35000, [](uint32_t) { return 0.0f; }, 0.3f, 0.02f);
      expect(f, Phase::Ground, "the barometer live 5 s of the air: the ground after 30 s quiet");
      continue;
    }
    s.run(179000, [](uint32_t) { return 0.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "no altitude all through the air: 2 min 59 s quiet is still the air");
    s.run(2000, [](uint32_t) { return 0.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "no altitude all through the air: 3 min quiet is the ground");
  }
}

// Handled with the barometer live and flat: 5 s is never the air, nor 15 s with no climb, a minute
// more is, and the ground 30 s after
static void handling()
{
  FlightPhase f;
  Sim s{f, 0};
  s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  s.run(5000, [](uint32_t) { return 120.0f; }, 4.0f, 2.0f);
  s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  check(!s.sawAir, "handled 5 s is never the air");
  s.run(15000, [](uint32_t) { return 120.0f; }, 4.0f, 2.0f);
  check(!s.sawAir, "handled 15 s with the barometer flat is not the air: no climb");
  s.run(55000, [](uint32_t) { return 120.0f; }, 4.0f, 2.0f);
  expect(f, Phase::Air, "handled 70 s is the air: the window held a minute");
  s.run(29000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Air, "put down 29 s: still the air");
  s.run(2000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Ground, "put down 30 s: the ground");
}

// A gap in the IMU holds the phase and keeps the window, and doesn't hold off the ground
static void imuGaps()
{
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    s.sawGround = false;
    s.run(9000, [](uint32_t) { return 300.0f; }, 0.0f, 0.0f, false);
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    check(f.phase() == Phase::Air && !s.sawGround, "a 9 s IMU gap mid-flight holds the air");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    auto climb = [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; };
    s.run(6000, climb, 5.0f, 1.0f);
    s.run(2000, climb, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "6 s of a running drone and a 2 s IMU gap: not yet the air");
    s.run(1500, climb, 5.0f, 1.0f);
    expect(f, Phase::Air, "1.5 s more after the gap is the air: the window kept the 6 s");
  }
  // On 2026-11-01 the HP boards' IMU slept through every transmit, and each gap started the quiet
  // over, so a board up high never landed. Up high it takes kLandedHighMs now, gaps and all
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    for (int i = 0; i < 74; i++) {
      s.run(2800, [](uint32_t) { return 300.0f; }, 0.3f, 0.02f);
      s.run(1200, [](uint32_t) { return 300.0f; }, 0.0f, 0.0f, false);
    }
    expect(f, Phase::Air, "high up, quiet, the IMU gone 1.2 s in every 4: 296 s is not yet the ground");
    s.run(6000, [](uint32_t) { return 300.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "and 5 min after the last movement is: the gaps don't start it over");
  }
}

// The IMU dies in the air: held for kImuDeadMs, then altitude lands it as before, two minutes still
// below half the climb
static void imuDead()
{
  FlightPhase f;
  Sim s{f, 0};
  s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  const uint32_t t0 = s.now;
  s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
  s.run(50000, [](uint32_t) { return 300.0f; }, 0.0f, 0.0f, false);
  expect(f, Phase::Air, "the IMU missing 50 s holds the air");
  const uint32_t r = s.now;
  s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 0.0f, 0.0f, false);
  s.run(100000, [](uint32_t) { return 100.0f; }, 0.0f, 0.0f, false);
  expect(f, Phase::Air, "IMU dead, 100 s still below half the climb: not yet the ground");
  s.run(25000, [](uint32_t) { return 100.0f; }, 0.0f, 0.0f, false);
  expect(f, Phase::Ground, "IMU dead, 120 s still below half the climb: the ground on altitude");
}

// The boards read the IMU every 50 ms and both recordings carry fewer: board 13's radio a few a
// second, board 9's flash about 12 a second in the air and one per 5 s on the ground. A window by
// time can't be judged on that, so each reading stands for the slots after it up to the next, for a
// second at most; the rest of a longer gap was lost
static void imuHeld(FlightPhase &f, unsigned long fromMs, unsigned long toMs, float accel, float gyro)
{
  for (unsigned long t = fromMs + 50; t < toMs && t <= fromMs + 1000; t += 50) {
    f.imuSample(t, accel, gyro);
  }
}

// The two drops of 2026-10-13 as board 13 reported them over the radio, GPS altitude as the only
// altitude (its barometer was dead all day). On GPS: in the air through both releases and falls,
// landed 30 s after impact 1, never landed on the drone. Between the drops it was handled for most
// of 38 s (631 to 669 s) with no climb, so that stays the ground. With no altitude at all the IMU
// alone takes it up, and the ground needs 3 min quiet, which it never had between the drops
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
    Phase at430 = Phase::Ground, at471 = Phase::Ground, at560 = Phase::Ground, at800 = Phase::Ground, at1055 = Phase::Ground;
    int airs = 0, grounds = 0;
    unsigned long air[4] = {}, ground[4] = {};
    unsigned long lastMs = 0;
    float lastAccel = 0.0f, lastGyro = 0.0f;
    while (fscanf(in, "%lu,%f,%f,%d,%f\n", &ms, &accel, &gyro, &fix, &alt) == 5) {
      if (lastMs != 0) {
        imuHeld(f, lastMs, ms, lastAccel, lastGyro);
      }
      f.imuSample(ms, accel, gyro);
      lastMs = ms;
      lastAccel = accel;
      lastGyro = gyro;
      if (withGps && fix) {
        f.gpsSample(ms, alt);
      }
      if (f.changed()) {
        if (f.phase() == Phase::Air && airs < 4) {
          air[airs++] = ms;
        } else if (f.phase() == Phase::Ground && grounds < 4) {
          ground[grounds++] = ms;
        }
      }
      if (ms <= 430000) at430 = f.phase();
      if (ms <= 471000) at471 = f.phase();
      if (ms <= 560000) at560 = f.phase();
      if (ms <= 800000) at800 = f.phase();
      if (ms <= 1055000) at1055 = f.phase();
    }
    fclose(in);
    printf("%s: air %lu %lu %lu, ground %lu %lu %lu\n", withGps ? "on GPS" : "blind", air[0], air[1], air[2],
           ground[0], ground[1], ground[2]);
    if (withGps) {
      check(airs == 2 && grounds == 2, "GPS: two spells in the air, no flip-flop on the drone");
      check(air[0] > 150000 && air[0] < 230000, "GPS: the air during the first climb");
      check(at430 == Phase::Air && at471 == Phase::Air, "GPS: in the air through release 1 and the fall");
      check(ground[0] > 471000 && ground[0] < 510000, "GPS: landed within 40 s of impact 1");
      check(at560 == Phase::Ground, "GPS: still landed on the ground");
      check(at800 == Phase::Ground, "GPS: handled between the drops with no climb stays the ground");
      check(air[1] > 840000 && air[1] < 940000, "GPS: the air during the second climb");
      check(at1055 == Phase::Air, "GPS: in the air through release 2");
      check(ground[1] > 1062000, "GPS: landed after the second fall");
    } else {
      check(airs == 1 && grounds == 1, "blind: one spell in the air");
      check(air[0] > 150000 && air[0] < 230000, "blind: the air during the first climb");
      check(at430 == Phase::Air && at471 == Phase::Air && at560 == Phase::Air && at800 == Phase::Air && at1055 == Phase::Air,
            "blind: never 3 min quiet, so the air through both drops");
      check(ground[0] > 1062000, "blind: landed after the second fall");
    }
  }
}

// The two drops of 2026-10-17 from board 9's flash (2026-10-18 pull 1432-board9): boot 24 is drop 1,
// boot 32 drop 2, barometer live, GPS never fixed. The barometer is filled in on a straight line
// between records, as the board read it every 200 ms; with one reading per 5 s record the baseline
// lags and the drift before drop 2 reads as the climb 20 s early. After Landed the board logged
// every 10 s, so the ground can only show on the next record
// Timelines from the 2026-10-17 field notes: boot 24 air +177 s, release +292.9 s,
// touchdown +309.8 s; boot 32 air +84 s, release +382.5 s, touchdown +393.7 s. Wanted: the air
// within the ring's 30 s of the climb, so the take-off is on the chip, through the fall, and the
// ground within 40 s of touchdown. On drop 2 the drone read quiet for a quarter to a half of its
// first 20 s, so the IMU takes about 20 s there
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
  unsigned long imuMs = 0;
  float lastAccel = 0.0f, lastGyro = 0.0f;
  int b, imu, baro;
  unsigned long ms;
  float accel, gyro, alt;
  while (fscanf(in, "%d,%lu,%f,%f,%d,%d,%f\n", &b, &ms, &accel, &gyro, &imu, &baro, &alt) == 7) {
    if (b != boot) {
      f = FlightPhase();   // a reset
      boot = b;
      lastAlt = NAN;
      imuMs = 0;
    }
    const int d = boot == drops[0].boot ? 0 : 1;
    if (imuMs != 0) {
      imuHeld(f, imuMs, ms, lastAccel, lastGyro);
    }
    if (imu) {
      f.imuSample(ms, accel, gyro);
      imuMs = ms;
      lastAccel = accel;
      lastGyro = gyro;
    } else {
      f.imuLost(ms);
      imuMs = 0;
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
    snprintf(what, sizeof what, "board 9 drop %d: the air within 30 s of the climb, then the ground, two changes only", d + 1);
    check(changes[d] == 2 && air[d] > drops[d].climb - 10000 && air[d] < drops[d].climb + 30000, what);
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

  // handled on and off for two minutes, never long enough for the air
  {
    FlightPhase f;
    Sim s{f, 0};
    bool txGround = true;
    for (int i = 0; i < 120; i++) {
      s.run(1000, [](uint32_t t) { return 120.0f + 2.0f * sinf(t / 700.0f); }, i % 10 < 5 ? 4.0f : 0.1f, i % 10 < 5 ? 2.0f : 0.01f);
      txGround = txGround && f.phase() == Phase::Ground;
    }
    check(f.logsAir() && txGround && !f.changed(), "boot, handled 5 s in 10 for two minutes: the log in the air, the radio on the ground");
    s.run(31000, [](uint32_t) { return 120.0f; }, 0.2f, 0.02f);
    check(!f.logsAir() && f.phase() == Phase::Ground, "boot, handled then put down 30 s: the log on the ground");
  }

  // a real take-off
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(5000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(7000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    check(f.phase() == Phase::Ground && f.logsAir(), "boot, 7 s of a running drone: the radio not yet in the air, the log is");
    s.run(2000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    check(f.phase() == Phase::Air && f.logsAir(), "boot, 9 s: the air for the radio and the log");
    check(f.changed() && !f.logChanged(), "boot, climbed: the radio's change reported, the log never left the air");
    // and lands
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(31000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    check(f.phase() == Phase::Ground && !f.logsAir() && f.changed() && f.logChanged(), "boot, flown and landed: both on the ground");
  }

  // reset mid-fall at 300 m, 10 m/s, tumbling: logged as the air. A fall is no climb and 20 s is short
  // of kMotionHeldMs, so the radio stays on the ground
  {
    FlightPhase f;
    Sim s{f, 0};
    const uint32_t t0 = s.now;
    s.run(7000, [t0](uint32_t t) { return 300.0f - 10.0f * (t - t0) / 1000.0f; }, 9.0f, 8.0f);
    check(f.logsAir() && f.phase() == Phase::Ground, "reset mid-fall: the fall is logged as the air, the radio not yet");
    s.run(13000, [t0](uint32_t t) { return 300.0f - 10.0f * (t - t0) / 1000.0f; }, 9.0f, 8.0f);
    check(f.logsAir() && f.phase() == Phase::Ground && !f.changed(), "reset mid-fall: tumbling is logged as the air, the radio stays on the ground");
    s.run(3000, [](uint32_t) { return 100.0f; }, 6.0f, 3.0f);   // the impact and tumbling
    s.run(25000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    check(f.logsAir(), "reset mid-fall: the impact and 25 s after it still logged as the air");
    s.run(6000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    check(!f.logsAir() && f.phase() == Phase::Ground && f.logChanged(), "reset mid-fall: 30 s at rest is the ground for the log");
  }

  // reset mid-fall, not tumbling: the barometer falling holds boot-air until it stops
  {
    FlightPhase f;
    Sim s{f, 0};
    const uint32_t t0 = s.now;
    s.run(40000, [t0](uint32_t t) { return 500.0f - 10.0f * (t - t0) / 1000.0f; }, 0.5f, 0.05f);
    check(f.logsAir() && f.phase() == Phase::Ground, "reset in a flat fall: 40 s quiet but falling, still logged as the air");
    s.run(29000, [](uint32_t) { return 100.0f; }, 0.5f, 0.05f);
    check(f.logsAir(), "reset in a flat fall: 29 s after it stops, still logged as the air");
    s.run(4000, [](uint32_t) { return 100.0f; }, 0.5f, 0.05f);
    check(!f.logsAir() && f.phase() == Phase::Ground, "reset in a flat fall: the log on the ground 30 s after it stops");
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

// Boot-air ends at rest only once a live altitude is flat as well: a reset under a slow parachute
// is quiet but still coming down. The transmit offset on every other reading is flat
static void bootAirFlat()
{
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(90000, [](uint32_t t) { return 500.0f - 2.0f * t / 1000.0f; }, 0.3f, 0.02f);
    check(f.logsAir(), "boot in a 2 m/s fall, quiet: still logged as the air after 90 s");
    s.run(29000, [](uint32_t) { return 320.0f; }, 0.3f, 0.02f);
    check(f.logsAir(), "boot, down and 29 s flat: still the air");
    s.run(3000, [](uint32_t) { return 320.0f; }, 0.3f, 0.02f);
    check(!f.logsAir(), "boot, down and 30 s flat: the log on the ground");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    auto alternating = [](uint32_t t) { return 340.0f + ((t / 200) % 2 ? 46.0f : 0.0f); };
    s.run(29000, alternating, 0.1f, 0.01f);
    check(f.logsAir(), "boot on a desk, the offset every other reading: 29 s is still the air");
    s.run(3000, alternating, 0.1f, 0.01f);
    check(!f.logsAir(), "and 31 s is the ground: the offset reads as flat");
  }
}

// On the ground the IMU runs at 1 Hz: kWakeMoving of the last kWakeReadings moving puts it at 20 Hz,
// and kWakeQuietMs without moving puts it back, both as events
static void waking()
{
  using namespace ChipSatLog;
  static std::vector<SystemEvent> seen;
  seen.clear();
  setSystemSink([](void *, const SystemEvent &e) { seen.push_back(e); }, nullptr);
  FlightPhase f;
  Sim s{f, 0};
  s.paced = true;
  check(f.imuFast(), "boot-air: the IMU at 20 Hz");
  s.run(40000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  check(!f.logsAir() && !f.imuFast(), "boot-air over at rest: the IMU at 1 Hz");
  s.runImu(20000, [](uint32_t) { return 120.0f; }, [](uint32_t t) { return (t / 1000) % 5 < 2 ? 4.0f : 0.1f; }, 0.01f);
  check(!f.imuFast(), "two of every five readings at 1 Hz moving: still 1 Hz");
  s.run(3000, [](uint32_t) { return 120.0f; }, 4.0f, 2.0f);
  check(f.imuFast() && f.phase() == Phase::Ground, "three of the last five moving: 20 Hz, still the ground");
  s.run(29000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  check(f.imuFast(), "29 s quiet at 20 Hz: still 20 Hz");
  s.run(2000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  check(!f.imuFast() && !s.sawAir, "30 s quiet: 1 Hz again, never the air");
  bool fast = false, slow = false;
  for (const SystemEvent &e : seen) {
    fast = fast || (e.source == kSourcePhase && e.what == kPhaseImuFast && e.part == kPartImu && e.reason == kWhyMoving);
    slow = slow || (e.source == kSourcePhase && e.what == kPhaseImuSlow && e.part == kPartImu && e.reason == kWhyQuiet);
  }
  check(fast && slow, "the step to 20 Hz and back are system events");

  // then a take-off from 1 Hz: woken, the window filled at 20 Hz, the climb
  s.run(5000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  const uint32_t t0 = s.now;
  s.run(12000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
  expect(f, Phase::Air, "a running drone from 1 Hz is the air within 12 s");
  setSystemSink(nullptr, nullptr);
}

// With an altitude live the IMU needs the climb too, or the window held kMotionHeldMs. Blind, the
// IMU alone. The barometer alone never
static void takeOff()
{
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    s.run(20000, [](uint32_t) { return 120.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Ground, "20 s of a running drone with the barometer flat: not the air without a climb");
    const uint32_t t0 = s.now;
    s.run(7000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "and once it has climbed 15 m it is");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    s.run(66000, [](uint32_t t) { return 120.0f + 0.5f * sinf(t / 900.0f); }, 5.0f, 1.0f);
    expect(f, Phase::Ground, "66 s of motion, altitude flat: not yet");
    s.run(4000, [](uint32_t t) { return 120.0f + 0.5f * sinf(t / 900.0f); }, 5.0f, 1.0f);
    expect(f, Phase::Air, "the window held a minute is the air whatever the altitude says");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.baro = false;
    s.run(60000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
    s.run(9000, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "no altitude source: the IMU alone, 8 s of a running drone");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(120000, [t0](uint32_t t) { return 120.0f + 2.0f * (t - t0) / 1000.0f; }, 0.1f, 0.01f);
    check(!s.sawAir, "the barometer climbing 240 m with the IMU quiet is never the air");
  }
}

// Up to 300 m and down again, quiet from the release: where it stops and how flat decides
static void landingRules()
{
  auto flown = [](Sim &s) {
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
  };
  {
    FlightPhase f;
    Sim s{f, 0};
    flown(s);
    s.run(299000, [](uint32_t) { return 300.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "a hover, quiet and flat at the top: still the air at 4 min 59 s");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    flown(s);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(29000, [](uint32_t) { return 100.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "down below half the climb, 29 s quiet: not yet");
    s.run(3000, [](uint32_t) { return 100.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "and 31 s is the ground");
    check(f.landed(), "after a flight: landed");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    flown(s);
    const uint32_t r = s.now;
    s.run(8000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(31000, [](uint32_t) { return 220.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "caught in a tree 100 m up, 31 s quiet and flat: still the air");
    s.run(268000, [](uint32_t) { return 220.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "and at 4 min 59 s");
    s.run(3000, [](uint32_t) { return 220.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "the tree is the ground after 5 min quiet and flat");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    flown(s);
    const uint32_t r = s.now;
    s.sawGround = false;
    s.run(60000, [r](uint32_t t) { return 300.0f - 3.0f * (t - r) / 1000.0f; }, 0.3f, 0.02f);
    check(f.phase() == Phase::Air && !s.sawGround, "a steady 3 m/s fall, not tumbling, below the descent's 5 m/s: the air");
  }
  {
    FlightPhase f;
    check(!f.landed(), "boot: not landed");
    Sim s{f, 0};
    s.run(40000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    check(!f.logsAir() && !f.landed(), "boot-air ending is not a landing");
  }
}

int main()
{
  bootAir();
  bootAirFlat();
  waking();
  takeOff();
  landingRules();
  bootAirTimeout();
  onTheBarometer();
  onGps();
  blind();
  highUp();
  barometerOffsets();
  flatFall();
  noAltitude();
  handling();
  imuGaps();
  imuDead();
  recorded();
  recordedBoard9();
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
