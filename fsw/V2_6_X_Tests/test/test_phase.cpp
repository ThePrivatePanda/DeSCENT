// Phase.cpp on the laptop: a mount, a climb, a hover, a fall and a landing, replayed as the sensors
// would report them, the knocks and dead sensors in between, the ways down that leave a board high
// (a roof, a car ride up a hill), and the real drops of 2026-10-13 and 2026-10-17.
// Run from V2_6_X_Tests, so the recorded flight under test/data is found:
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src -I../V2_6_X_Code/src/app test/test_phase.cpp ../V2_6_X_Code/src/app/Phase.cpp -o /tmp/phase && /tmp/phase
#include <algorithm>
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
  float felt = NAN;   // what the IMU feels, gravity in. NAN: no orientation
  float yaw = NAN;      // the orientation, a turn about one axis, rad, plus yawRate a second. NAN: none
  float yawRate = 0.0f;

  ChipSatSensors::Quaternionf orientation() const
  {
    if (std::isnan(yaw)) {
      return {0.0f, 0.0f, 0.0f, 0.0f};
    }
    const float a = yaw + yawRate * static_cast<float>(now) / 1000.0f;
    return {0.0f, 0.0f, sinf(0.5f * a), cosf(0.5f * a)};
  }

  template <typename Alt>
  void run(uint32_t forMs, Alt alt, float accel, float gyro, bool imu = true)
  {
    runImu(forMs, alt, [accel](uint32_t) { return accel; }, gyro, imu);
  }

  // the same with the acceleration a function of time, for a drone whose vibration comes and goes
  template <typename Alt, typename Accel>
  void runImu(uint32_t forMs, Alt alt, Accel accel, float gyro, bool imu = true)
  {
    runMotion(forMs, alt, accel, [gyro](uint32_t) { return gyro; }, imu);
  }

  // and the rotation too, for a tumble
  template <typename Alt, typename Accel, typename Gyro>
  void runMotion(uint32_t forMs, Alt alt, Accel accel, Gyro gyro, bool imu = true)
  {
    const uint32_t until = now + forMs;
    while (now < until) {
      if (!imu) {
        f.imuLost(now);
      } else if (!paced || f.imuFast() || now % 1000 == 0) {
        f.imuSample(now, accel(now), gyro(now), felt, orientation());
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
  // Switched on, carried to the drone and handled: an hour of waiting. The air at the slightest hint,
  // so handling is the air until 30 s after it stops, and so is a drone running on the ground
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
    expect(f, Phase::Air, "handled 5 s at a time on the way to the drone: the air");
    s.run(24000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Air, "put down 29 s: still the air");
    s.run(2000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Ground, "put down 30 s: the ground");
    f.changed();
    s.run(3000000, [](uint32_t t) { return 120.0f + 6.0f * sinf(t / 300000.0f); }, 0.05f, 0.0f);   // weather over 50 min
    expect(f, Phase::Ground, "an hour mounted, with the weather drifting");
    check(!f.changed(), "nothing changed, so nothing to transmit about");
    s.run(600000, [](uint32_t t) { return 120.0f + 1.0f * sinf(t / 100.0f); }, 6.0f, 1.0f);
    expect(f, Phase::Air, "a drone running on the ground ten minutes is the air by the IMU");
    s.run(31000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Ground, "and the ground 30 s after its motors stop");
  }

  // The drone takes off and climbs at 3 m/s: the air on the hint, 20 moving readings in
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(900, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Ground, "0.9 s of a running drone is not yet the air");
    s.run(6100, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "1 s of it is the air");
    check(f.changed(), "the change is reported once");
    check(!f.changed(), "and only once");
    check(f.aboveGroundM() > 20.0f && f.aboveGroundM() < 30.0f, "the height above the ground reads");

    // hover at 300 m, the drone's vibration all the while, no release: still the air
    s.run(120000, [](uint32_t t) { return 300.0f + 3.0f * sinf(t / 900.0f); }, 5.0f, 1.0f);
    expect(f, Phase::Air, "hovering on the drone is still the air");

    // a quadcopter holding dead still at 300 m, which a real drone's IMU never gives (see highUp
    // below): the ground 30 s after its last movement, like a roof. Moving again is the air
    s.run(29000, [](uint32_t) { return 300.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Air, "29 s of dead-still hover at 180 m up is still the air");
    s.run(2000, [](uint32_t) { return 300.0f; }, 0.5f, 0.05f);
    expect(f, Phase::Ground, "31 s of it, quiet and flat, is the ground at any height");
    s.run(9000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "and the drone moving again is the air at once");

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

    // picked up and carried back: the air, and the ground again 30 s after it is put down
    s.run(1500, [](uint32_t t) { return 100.0f + 1.5f * sinf(t / 500.0f); }, 4.0f, 2.0f);
    expect(f, Phase::Air, "carried after landing: the air within a second and a half");
    s.run(60000, [](uint32_t t) { return 100.0f + 1.5f * sinf(t / 500.0f); }, 4.0f, 2.0f);
    expect(f, Phase::Air, "carried a minute: still the air");
    s.run(31000, [](uint32_t) { return 100.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "put down, the ground 30 s later");
  }

  // The IMU is dead: a climb is the air as it is with the IMU, and no reading is quiet, so it lands
  // as a still board does, 30 s after the fall stops and the barometer reads flat
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "on the ground with no IMU");
    const uint32_t t0 = s.now;
    s.run(4000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "12 m up with the IMU missing: not yet");
    s.run(31000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "15 m up is the air, the IMU missing or not");
    s.run(20000, [](uint32_t) { return 300.0f; }, 0.0f, 0.0f, false);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 0.0f, 0.0f, false);
    s.run(25000, [](uint32_t) { return 90.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "no IMU, 25 s still after the fall: not yet the ground");
    s.run(9000, [](uint32_t) { return 90.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "no IMU, 34 s still after the fall: the ground, as with a quiet IMU");
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
    s.run(7000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Ground, "a smooth climb on GPS, the IMU quiet: 21 m is not yet the air");
    s.run(9000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Air, "48 m is: GPS's 30 m");
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
  expect(f, Phase::Air, "carried 5 s blind is the air, and with no altitude the ground takes 3 min");
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
// height, so they land like any other, and so does a board whose IMU is dead
static void highUp()
{
  // Flown up to a roof 130 m above the pad and left there: the ground 30 s after the last movement,
  // and a knock starts it again
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
    expect(f, Phase::Ground, "31 s quiet and flat on the roof is the ground");
    check(f.changed(), "reported");
    check(fabsf(f.aboveGroundM()) < 1.0f, "the roof is the new ground");
    const uint32_t t2 = s.now;
    s.run(8000, [t2](uint32_t t) { return 250.0f + 3.0f * (t - t2) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "and a take-off from the roof is the air again");
  }

  // Driven 60 m up a hill in two minutes on a bumpy road: the barometer climbs and the IMU moves a
  // third of the time, which is a climb with movement, so the air at 15 m. A known cost of taking a
  // smooth drone climb: the ground 30 s after the last bump
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.runImu(120000, [t0](uint32_t t) { return 120.0f + 0.5f * (t - t0) / 1000.0f; },
             [](uint32_t t) { return (t / 1000) % 3 == 0 ? 3.0f : 1.0f; }, 0.2f);
    check(s.sawAir, "driven 60 m up a hill: the air on the way");
    s.run(27000, [](uint32_t) { return 180.0f; }, 0.3f, 0.02f);   // parked, the last bump 2 s before
    expect(f, Phase::Air, "parked 27 s: not yet the ground");
    s.run(2000, [](uint32_t) { return 180.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "parked 29 s, 31 s after the last bump: the ground");
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

  // With the IMU dead no reading is quiet, and the quiet runs from take-off: high up it is the ground
  // as anywhere, and a climb past the minute is the ground at the minute and the air again 15 m on
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "no IMU, climbed 180 m: the air");
    s.run(31000, [](uint32_t) { return 290.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "no IMU, at rest high up 31 s: the ground");
    const uint32_t t1 = s.now;
    auto climb = [t1](uint32_t t) { return 290.0f + 3.0f * (t - t1) / 1000.0f; };
    s.run(8000, climb, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "no IMU, climbing off the roof: the air at 15 m");
    s.sawGround = false;
    s.run(112000, climb, 0.0f, 0.0f, false);
    check(s.sawGround && f.phase() == Phase::Air, "no IMU, a 2 min climb: the ground at the minute, then the air again");
  }
}

// A desk board, IMU quiet, its barometer reading 46 m high on every other reading as boards did
// during 22 dBm transmits on 2026-11-01: never the air. Stepping up 46 m and staying there is a
// climb to the phase, which takes the barometer at its word: keeping transmit readings out is the
// controller's part (test_flight_loop's desk boards)
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
    check(sb.sawAir, gap ? "desk, barometer stepping up 46 m for whole seconds, 9 s IMU gap: a climb to the phase"
                         : "desk, barometer stepping up 46 m for whole seconds: a climb to the phase");
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
  // going: neither is a fall. The offset reads flat, so the ground at 30 s quiet; the step starts
  // the flat again, so the ground 30 s after it
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
    if (offset) {
      expect(f, Phase::Ground, "31 s quiet with the offset is: it reads flat and is no fall");
      continue;
    }
    expect(f, Phase::Air, "31 s quiet, 21 s after the step: not yet");
    s.run(8000, alt, 0.3f, 0.02f);
    expect(f, Phase::Air, "39 s quiet, 29 s after the step: not yet");
    s.run(2000, alt, 0.3f, 0.02f);
    expect(f, Phase::Ground, "41 s quiet, 31 s flat after the step: the ground, a step is no fall");
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
    s.run(5000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
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

// Handled with the barometer live and flat: the air at the hint, the ground 30 s after it stops
static void handling()
{
  FlightPhase f;
  Sim s{f, 0};
  s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  s.run(900, [](uint32_t) { return 120.0f; }, 4.0f, 2.0f);
  expect(f, Phase::Ground, "handled 0.9 s: not yet");
  s.run(100, [](uint32_t) { return 120.0f; }, 4.0f, 2.0f);
  expect(f, Phase::Air, "handled 1 s: the air");
  s.run(15000, [](uint32_t) { return 120.0f; }, 4.0f, 2.0f);
  s.run(29000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Air, "put down 29 s: still the air");
  s.run(2000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  expect(f, Phase::Ground, "put down 30 s: the ground");
}

// A gap in the IMU is quiet and keeps the window, and doesn't hold off the ground
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
    check(f.phase() == Phase::Air && !s.sawGround, "a 9 s IMU gap mid-flight: 9 s quiet, still the air");
  }
  // no altitude, so the hint decides: a brown-out right after the start costs only the readings
  {
    FlightPhase f;
    Sim s{f, 0};
    s.baro = false;
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    s.run(500, [](uint32_t) { return 120.0f; }, 5.0f, 1.0f);
    s.run(3000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "0.5 s of a running drone and a 3 s brown-out: not yet the air");
    s.run(1000, [](uint32_t) { return 120.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "1 s after it is");
  }
  // On 2026-11-01 the HP boards' IMU slept through every transmit, and each gap started the quiet
  // over, so a board up high never landed. The gaps don't start it over now
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    for (int i = 0; i < 7; i++) {
      s.run(2800, [](uint32_t) { return 300.0f; }, 0.3f, 0.02f);
      s.run(1200, [](uint32_t) { return 300.0f; }, 0.0f, 0.0f, false);
    }
    expect(f, Phase::Air, "high up, quiet, the IMU gone 1.2 s in every 4: 28 s is not yet the ground");
    s.run(4000, [](uint32_t) { return 300.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "and 32 s after the last movement is: the gaps don't start it over");
  }
}

// The IMU dies in the air: no reading is quiet, so it lands on the rules a quiet IMU does. A fall the
// barometer sees holds the air, and a barometer that won't read flat lands it at the minute
static void imuDead()
{
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    s.run(20000, [](uint32_t) { return 300.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "the IMU gone 20 s at the top: still the air");
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "IMU dead, 40 s without a reading but falling 20 of them: the air");
    s.run(25000, [](uint32_t) { return 100.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "IMU dead, 25 s still after the fall: not yet the ground");
    s.run(8000, [](uint32_t) { return 100.0f; }, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "IMU dead, 33 s still and flat: the ground");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    auto jumpy = [](uint32_t t) { return 300.0f + ((t / 1000) % 2 ? 4.0f : 0.0f); };
    s.run(58000, jumpy, 0.0f, 0.0f, false);
    expect(f, Phase::Air, "IMU dead, the barometer 4 m apart every other second, 58 s: not yet the ground");
    s.run(3000, jumpy, 0.0f, 0.0f, false);
    expect(f, Phase::Ground, "and 61 s: the ground on the minute");
  }
}

// The boards read the IMU every 50 ms and both recordings carry fewer: board 13's radio a few a
// second, board 9's flash about 12 a second in the air and one per 5 s on the ground. A window by
// time can't be judged on that, so each reading stands for the slots after it up to the next, for a
// second at most; the rest of a longer gap was lost. A copy is no measured rotation, so it stays
// under the tumble line: one handling reading in board 9's 10 s trickle isn't a second of tumbling
static void imuHeld(FlightPhase &f, unsigned long fromMs, unsigned long toMs, float accel, float gyro)
{
  for (unsigned long t = fromMs + 50; t < toMs && t <= fromMs + 1000; t += 50) {
    f.imuSample(t, accel, std::min(gyro, 5.0f), NAN, {0.0f, 0.0f, 0.0f, 0.0f});
  }
}

// The two drops of 2026-10-13 as board 13 reported them over the radio, GPS altitude as the only
// altitude (its barometer was dead all day). On GPS: in the air through both releases and falls,
// landed 30 s after impact 1, never landed on the drone. Handled between the drops it goes up on
// the hint and comes down after. With no altitude at all the ground needs 3 min quiet, which it
// never had between the drops
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
      f.imuSample(ms, accel, gyro, NAN, {0.0f, 0.0f, 0.0f, 0.0f});
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
      check(airs == 3 && grounds == 3, "GPS: three spells in the air, the second the handling between the drops");
      check(air[0] < 230000, "GPS: the air by the first climb");
      check(at430 == Phase::Air && at471 == Phase::Air, "GPS: in the air through release 1 and the fall");
      check(ground[0] > 471000 && ground[0] < 510000, "GPS: landed within 40 s of impact 1");
      check(air[2] < 940000, "GPS: the air by the second climb");
      check(at1055 == Phase::Air, "GPS: in the air through release 2");
      check(ground[2] > 1062000, "GPS: landed after the second fall");
    } else {
      check(airs == 1 && grounds == 1, "blind: one spell in the air");
      check(air[0] < 230000, "blind: the air by the first climb");
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
// ground within 40 s of touchdown. Handled before and after, each moving record held a second is a
// hint, so the boots go up then too
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
  unsigned long firstAir[2] = {0, 0}, landed[2] = {0, 0};   // the first air of the boot, the first ground after touchdown
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
    // the held IMU copies and the barometer filled in between two records, in time order as a board
    // would see them
    std::vector<std::pair<unsigned long, float>> between;   // when, and the altitude or NAN for the IMU
    for (unsigned long t = imuMs + 50; imuMs != 0 && t < ms && t <= imuMs + 1000; t += 50) {
      between.push_back({t, NAN});
    }
    if (baro && !std::isnan(lastAlt) && ms - envMs > 400) {
      for (unsigned long t = envMs + 200; t + 200 <= ms; t += 200) {
        between.push_back({t, lastAlt + (alt - lastAlt) * static_cast<float>(t - envMs) / static_cast<float>(ms - envMs)});
      }
    }
    std::sort(between.begin(), between.end(), [](const auto &x, const auto &y) { return x.first < y.first; });
    for (const auto &[t, a] : between) {
      if (std::isnan(a)) {
        f.imuSample(t, lastAccel, std::min(lastGyro, 5.0f), NAN, {0.0f, 0.0f, 0.0f, 0.0f});
      } else {
        f.envSample(t, a);
      }
    }
    if (imu) {
      f.imuSample(ms, accel, gyro, NAN, {0.0f, 0.0f, 0.0f, 0.0f});
      imuMs = ms;
      lastAccel = accel;
      lastGyro = gyro;
    } else {
      f.imuLost(ms);
      imuMs = 0;
    }
    // a record carries the newest barometer reading, which is new every 200 ms
    if (baro && (alt != lastAlt || ms - envMs >= 200)) {
      f.envSample(ms, alt);
      lastAlt = alt;
      envMs = ms;
    }
    if (f.changed()) {
      changes[d]++;
      (f.phase() == Phase::Air ? air[d] : ground[d]) = ms;
      if (f.phase() == Phase::Air && firstAir[d] == 0) {
        firstAir[d] = ms;
      }
      if (f.phase() == Phase::Ground && landed[d] == 0 && ms > drops[d].touchdown) {
        landed[d] = ms;
      }
    }
    if (ms <= drops[d].release) airAtRelease[d] = f.phase() == Phase::Air;
    if (ms <= drops[d].touchdown) airAtTouchdown[d] = f.phase() == Phase::Air;
  }
  fclose(in);
  for (int d = 0; d < 2; d++) {
    printf("board 9 boot %d: %d changes, air %lu, ground %lu\n", drops[d].boot, changes[d], air[d], ground[d]);
    char what[96];
    snprintf(what, sizeof what, "board 9 drop %d: the air by 30 s into the climb", d + 1);
    check(firstAir[d] != 0 && firstAir[d] < drops[d].climb + 30000, what);
    snprintf(what, sizeof what, "board 9 drop %d: still the air at the release and through the fall", d + 1);
    check(airAtRelease[d] && airAtTouchdown[d], what);
    snprintf(what, sizeof what, "board 9 drop %d: the ground within 30 s of rest after touchdown, and the record gap", d + 1);
    check(landed[d] > drops[d].touchdown && landed[d] < drops[d].touchdown + 45000, what);
  }
}

// After a reset: logged as the air, the phase on the ground (decided 2026-10-22). The controller's
// transmit gap follows logsAir() too (2026-11-16)
static void bootAir()
{
  // left still
  {
    FlightPhase f;
    Sim s{f, 0};
    check(f.logsAir() && f.phase() == Phase::Ground, "boot: the log in the air, the phase on the ground");
    bool ground = true;
    for (int i = 0; i < 25; i++) {
      s.run(1000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
      ground = ground && f.phase() == Phase::Ground;
    }
    check(f.logsAir() && ground, "boot, 25 s still: the log still in the air, the phase on the ground");
    check(!f.logChanged(), "boot, 25 s still: the log has not changed");
    s.run(6000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    check(!f.logsAir(), "boot, 30 s still: the log is on the ground");
    check(f.logChanged() && !f.logChanged(), "boot, 30 s still: the log change is reported once");
    check(f.phase() == Phase::Ground && !f.changed(), "boot, 30 s still: the phase stayed on the ground throughout, no phase change to report");
    s.run(600000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    check(!f.logsAir() && f.phase() == Phase::Ground, "boot, ten minutes still: both on the ground");
  }

  // handled on and off for two minutes: the air for the phase too
  {
    FlightPhase f;
    Sim s{f, 0};
    for (int i = 0; i < 120; i++) {
      s.run(1000, [](uint32_t t) { return 120.0f + 2.0f * sinf(t / 700.0f); }, i % 10 < 5 ? 4.0f : 0.1f, i % 10 < 5 ? 2.0f : 0.01f);
    }
    check(f.logsAir() && f.phase() == Phase::Air && f.changed(), "boot, handled 5 s in 10 for two minutes: the air for the phase and the log");
    s.run(31000, [](uint32_t) { return 120.0f; }, 0.2f, 0.02f);
    check(!f.logsAir() && f.phase() == Phase::Ground, "boot, handled then put down 30 s: the log on the ground");
  }

  // a real take-off
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(5000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(900, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    check(f.phase() == Phase::Ground && f.logsAir(), "boot, 0.9 s of a running drone: the phase not yet in the air, the log is");
    s.run(6100, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    check(f.phase() == Phase::Air && f.logsAir(), "boot, 7 s: the air for the phase and the log");
    check(f.changed() && !f.logChanged(), "boot, climbed: the phase change reported, the log never left the air");
    // and lands
    s.run(60000, [](uint32_t) { return 300.0f; }, 5.0f, 1.0f);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(31000, [](uint32_t) { return 100.0f; }, 0.8f, 0.1f);
    check(f.phase() == Phase::Ground && !f.logsAir() && f.changed() && f.logChanged(), "boot, flown and landed: both on the ground");
  }

  // reset mid-fall at 300 m, 10 m/s, tumbling at 460 deg/s: logged as the air from the reset, and the
  // phase in the air a second in, though a fall is no climb
  {
    FlightPhase f;
    Sim s{f, 0};
    const uint32_t t0 = s.now;
    s.run(900, [t0](uint32_t t) { return 300.0f - 10.0f * (t - t0) / 1000.0f; }, 9.0f, 8.0f);
    check(f.logsAir() && f.phase() == Phase::Ground, "reset mid-fall: the fall is logged as the air, the phase not yet");
    s.run(200, [t0](uint32_t t) { return 300.0f - 10.0f * (t - t0) / 1000.0f; }, 9.0f, 8.0f);
    check(f.logsAir() && f.phase() == Phase::Air && f.changed() && !f.logChanged(), "reset mid-fall: tumbling a second is the air for the phase too");
    s.run(18900, [t0](uint32_t t) { return 300.0f - 10.0f * (t - t0) / 1000.0f; }, 9.0f, 8.0f);
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

  // IMU dead, barometer only: no reading is quiet, so the barometer flat 30 s ends it
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(29000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    check(f.logsAir(), "boot without an IMU, 29 s still: still logged as the air");
    s.run(3000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    check(!f.logsAir(), "boot without an IMU, 30 s still and flat: the log on the ground");
  }
}

// Boot-air ends at kBootAirMaxMs whatever the sensors say: a quiet IMU and a barometer 4 m apart every
// other second, never flat. With no IMU and no altitude nothing moves, so 30 s ends it
static void bootAirTimeout()
{
  {
    FlightPhase f;
    Sim s{f, 0};
    auto jumpy = [](uint32_t t) { return 120.0f + ((t / 1000) % 2 ? 4.0f : 0.0f); };
    s.run(299000, jumpy, 0.1f, 0.01f);
    check(f.logsAir() && !f.logChanged(), "quiet, the barometer never flat, 4:59: still logged as the air");
    s.run(1100, jumpy, 0.1f, 0.01f);
    check(!f.logsAir() && f.logChanged() && f.phase() == Phase::Ground && !f.changed(), "quiet, the barometer never flat, 5:00: logged as the ground, the phase unchanged");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.baro = false;
    s.run(29000, [](uint32_t) { return 0.0f; }, 0.0f, 0.0f, false);
    check(f.logsAir() && !f.logChanged(), "no IMU, no altitude, 29 s: still logged as the air");
    s.run(1100, [](uint32_t) { return 0.0f; }, 0.0f, 0.0f, false);
    check(!f.logsAir() && f.logChanged() && f.phase() == Phase::Ground && !f.changed(), "no IMU, no altitude, 30 s: logged as the ground, the phase unchanged");
    s.run(30000, [](uint32_t) { return 0.0f; }, 0.0f, 0.0f, false);
    check(!f.logsAir(), "and stays the ground");
    // the barometer comes back at 1:00 and the drone climbs at 1:30
    s.baro = true;
    s.run(30000, [](uint32_t) { return 120.0f; }, 0.0f, 0.0f, false);
    check(!f.logsAir() && f.phase() == Phase::Ground, "barometer back, still: ground");
    const uint32_t t0 = s.now;
    s.run(8000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 0.0f, 0.0f, false);
    check(f.phase() == Phase::Air && f.logsAir() && f.changed(), "a climb with no IMU: the air for both");
  }
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

// On the ground the IMU runs at 1 Hz: one moving reading puts it at 20 Hz, and kWakeQuietMs without
// moving puts it back, both as events
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
  s.run(20000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  check(!f.imuFast(), "20 s more quiet at 1 Hz: still 1 Hz");
  s.run(50, [](uint32_t) { return 120.0f; }, 4.0f, 2.0f);
  check(f.imuFast() && f.phase() == Phase::Ground, "one moving reading: 20 Hz, still the ground");
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

  // then a take-off from 1 Hz: woken on the first moving reading, the hint at 20 Hz
  s.run(5000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  const uint32_t t0 = s.now;
  s.run(2100, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
  expect(f, Phase::Air, "a running drone from 1 Hz is the air within 2.1 s");
  setSystemSink(nullptr, nullptr);
}

// Any one of the hints is the air: the IMU moving half of 2 s whatever the altitude says (a running
// drone, a pressurised cabin), a 15 m climb whatever the IMU says
static void takeOff()
{
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    s.run(1100, [](uint32_t) { return 120.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "a running drone with the barometer flat: the air in a second");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.baro = false;
    s.run(60000, [](uint32_t) { return 0.0f; }, 0.1f, 0.01f);
    s.run(1100, [](uint32_t) { return 0.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "no altitude source: the same");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(7000, [t0](uint32_t t) { return 120.0f + 2.0f * (t - t0) / 1000.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Ground, "the barometer climbing 14 m with the IMU quiet: not yet");
    s.run(3000, [t0](uint32_t t) { return 120.0f + 2.0f * (t - t0) / 1000.0f; }, 0.1f, 0.01f);
    expect(f, Phase::Air, "15 m is the air, the IMU quiet or not");
  }
  // a few stray moving readings, as a bad report or a knock gives, are not a hint
  {
    FlightPhase f;
    Sim s{f, 0};
    s.runImu(600000, [](uint32_t) { return 120.0f; }, [](uint32_t t) { return t % 1000 < 250 ? 4.0f : 0.1f; }, 0.01f);
    check(!s.sawAir, "five moving readings in every second for ten minutes: never the air");
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
    s.run(29000, [](uint32_t) { return 300.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "a hover, quiet and flat at the top: still the air at 29 s");
    s.run(2000, [](uint32_t) { return 300.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "and the ground at 31 s, at any height");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    flown(s);
    const uint32_t r = s.now;
    s.run(20000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(29000, [](uint32_t) { return 100.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "down, 29 s quiet: not yet");
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
    s.run(30000, [](uint32_t) { return 220.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "caught in a tree 100 m up, 30 s quiet, the flat a second short: still the air");
    s.run(2000, [](uint32_t) { return 220.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "the tree is the ground at 32 s, quiet and flat");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    flown(s);
    const uint32_t r = s.now;
    s.sawGround = false;
    s.run(59000, [r](uint32_t t) { return 300.0f - 3.0f * (t - r) / 1000.0f; }, 0.3f, 0.02f);
    check(f.phase() == Phase::Air && !s.sawGround, "a steady 3 m/s fall, not tumbling, below the descent's 5 m/s: the air for 59 s");
    s.run(2000, [r](uint32_t t) { return 300.0f - 3.0f * (t - r) / 1000.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Ground, "and the ground at a minute: too slow for the descent to hold, a known limit");
  }
  {
    FlightPhase f;
    check(!f.landed(), "boot: not landed");
    Sim s{f, 0};
    s.run(40000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    check(!f.logsAir() && !f.landed(), "boot-air ending is not a landing");
  }
}

// Lying still after a flight while the barometer jumps every second, as board 17's did for the
// 8.5 min it went on transmitting back to back after landing on 2026-11-08: never flat, so only the
// IMU can land it, a minute after its last movement
static void stillJumpyBarometer()
{
  using namespace ChipSatLog;
  static std::vector<SystemEvent> seen;
  seen.clear();
  setSystemSink([](void *, const SystemEvent &e) { seen.push_back(e); }, nullptr);
  FlightPhase f;
  Sim s{f, 0};
  s.run(60000, [](uint32_t) { return 555.0f; }, 0.1f, 0.01f);
  const uint32_t t0 = s.now;
  s.run(30000, [t0](uint32_t t) { return 555.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
  const uint32_t r = s.now;
  s.run(9000, [r](uint32_t t) { return 645.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
  expect(f, Phase::Air, "up and dropped");
  auto jumpy = [](uint32_t t) { return 555.0f + ((t / 200) % 5 == 0 ? ((t / 1000) % 2 ? 34.0f : -40.0f) : 0.0f); };
  s.run(59000, jumpy, 0.04f, 0.009f);
  expect(f, Phase::Air, "still, the barometer 34 m up or 40 m down every second: 59 s is not yet the ground");
  s.run(2000, jumpy, 0.04f, 0.009f);
  expect(f, Phase::Ground, "a minute still is the ground whatever the barometer does");
  bool imuRule = false;
  for (const SystemEvent &e : seen) {
    imuRule = imuRule || (e.what == kPhaseToGround && e.part == kPartImu && e.reason == kWhyQuietImu);
  }
  check(imuRule, "landed on the IMU's own rule, said as quietimu");
  setSystemSink(nullptr, nullptr);
}

// A fall is the air at once at 20 Hz, from the ground or boot-air, with no climb. The numbers are the
// pulls': every drop tumbled at a median 448-663 deg/s for 7 s and more, handling stayed under
// 190 deg/s in bursts of a second or two, and some releases read near 0 g for 250-370 ms first
static float dps(float d) { return d * 3.14159265f / 180.0f; }

static uint32_t untilAir(Sim &s, uint32_t forMs, float accel, float gyroLow, float gyroHigh)
{
  const uint32_t start = s.now;
  while (s.now - start < forMs && s.f.phase() != Phase::Air) {
    s.runMotion(50, [](uint32_t) { return 600.0f; }, [accel](uint32_t) { return accel; },
                [gyroLow, gyroHigh](uint32_t t) { return (t / 250) % 2 ? gyroHigh : gyroLow; });
  }
  return s.now - start;
}

static void falls()
{
  using namespace ChipSatLog;
  static std::vector<SystemEvent> seen;
  setSystemSink([](void *, const SystemEvent &e) { seen.push_back(e); }, nullptr);
  auto toAirFor = [](uint8_t why) {
    for (const SystemEvent &e : seen) {
      if (e.what == kPhaseToAir && e.reason == why) {
        return true;
      }
    }
    return false;
  };
  auto pad = [](uint32_t) { return 600.0f; };

  // on the ground at 20 Hz, woken by a knock, the barometer flat: the drop board 15 missed
  {
    seen.clear();
    FlightPhase f;
    Sim s{f, 0};
    s.paced = true;
    s.run(60000, pad, 0.1f, 0.01f);
    s.run(50, pad, 4.0f, 1.0f);
    s.run(5000, pad, 0.1f, 0.01f);
    check(f.imuFast() && f.phase() == Phase::Ground && !f.logsAir(), "on the ground, the IMU at 20 Hz after a knock");
    const uint32_t ms = untilAir(s, 7000, 9.0f, dps(450.0f), dps(650.0f));
    printf("tumbling from the ground at 20 Hz: the air %u ms in\n", static_cast<unsigned int>(ms));
    check(f.phase() == Phase::Air && ms <= 1100 && (toAirFor(kWhyTumbling) || toAirFor(kWhyMotionHint)),
          "a tumble at 450-650 deg/s is the air within about a second, its readings a hint first");
    check(f.changed(), "and the radio follows at once");
  }

  // boot-air
  {
    seen.clear();
    FlightPhase f;
    Sim s{f, 0};
    s.run(10000, pad, 0.1f, 0.01f);
    const uint32_t ms = untilAir(s, 7000, 9.0f, dps(450.0f), dps(650.0f));
    check(f.phase() == Phase::Air && ms <= 1100 && f.changed() && !f.logChanged(),
          "boot-air: a tumble is the air for the phase within about a second, the log never left it");
  }

  // the IMU at 1 Hz when it starts, 50 ms after a reading, the worst case: the next reading wakes
  // it to 20 Hz, then the second of tumbling
  {
    seen.clear();
    FlightPhase f;
    Sim s{f, 0};
    s.paced = true;
    s.run(60050, pad, 0.1f, 0.01f);
    check(!f.imuFast(), "at rest on the ground: 1 Hz");
    const uint32_t ms = untilAir(s, 7000, 9.0f, dps(450.0f), dps(650.0f));
    printf("tumbling from the ground at 1 Hz: the air %u ms in\n", static_cast<unsigned int>(ms));
    check(f.phase() == Phase::Air && ms <= 2100, "a tumble from 1 Hz: the wake on the next reading, then the air within about 2 s");
  }

  // handled for three minutes at 20 Hz: bursts of 1-2 s at 100-190 deg/s, a flick of 0.6 s at
  // 350 deg/s, felt acceleration 5-15 m/s2: the air on the hint, never a tumble or a free fall
  {
    seen.clear();
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, pad, 0.1f, 0.01f);
    s.sawAir = false;
    for (int i = 0; i < 20; i++) {
      s.felt = i % 2 ? 5.0f : 15.0f;
      s.run(1000, pad, 4.0f, dps(100.0f));
      s.run(2000, pad, 4.0f, dps(190.0f));
      s.run(600, pad, 4.0f, i % 4 == 0 ? dps(350.0f) : dps(120.0f));
      s.felt = 9.81f;
      s.run(5400, pad, 0.1f, 0.01f);
    }
    check(s.sawAir && !toAirFor(kWhyTumbling) && !toAirFor(kWhyFreeFall),
          "handled three minutes, bursts to 190 deg/s and flicks to 350: the hint, never a tumble or a free fall");
  }

  // released into a free fall at 20 Hz, the tumble not started yet: 250 ms near 0 g is the air
  {
    seen.clear();
    FlightPhase f;
    Sim s{f, 0};
    s.paced = true;
    s.felt = 9.81f;
    s.run(60000, pad, 0.1f, 0.01f);
    s.run(50, pad, 4.0f, 1.0f);
    s.run(5000, pad, 0.1f, 0.01f);
    s.felt = 0.3f;
    s.run(100, pad, 9.5f, 0.3f);
    expect(f, Phase::Ground, "100 ms near 0 g is not yet a free fall");
    s.run(150, pad, 9.5f, 0.3f);
    check(f.phase() == Phase::Air && toAirFor(kWhyFreeFall), "250 ms near 0 g is the air, said as freefall");
    FlightPhase g;
    Sim t{g, 0};
    t.felt = 9.81f;
    t.run(60000, pad, 0.1f, 0.01f);
    t.felt = 0.3f;
    t.run(100, pad, 1.0f, 0.1f);
    t.felt = 9.81f;
    t.run(100, pad, 0.1f, 0.01f);
    t.felt = 0.3f;
    t.run(100, pad, 1.0f, 0.1f);
    t.felt = NAN;
    t.run(1000, pad, 1.0f, 0.1f);
    check(!t.sawAir, "two 100 ms dips with a reading at 1 g between, then no orientation, the IMU quiet otherwise: never the air");
  }

  // A drone hovering dead still is the ground after a minute and the IMU goes to 1 Hz. Moving again,
  // it wakes to 20 Hz on its next reading and a release from there tumbles into the air within a
  // second. A release straight out of the still hover, 50 ms after a 1 Hz reading, costs the wait
  // for the next one as well
  for (int stillToRelease = 0; stillToRelease < 2; stillToRelease++) {
    seen.clear();
    FlightPhase f;
    Sim s{f, 0};
    s.paced = true;
    s.run(60000, pad, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(30000, [t0](uint32_t t) { return 600.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
    expect(f, Phase::Air, "the drone up 90 m");
    s.run(61000, [](uint32_t) { return 690.0f; }, 0.3f, 0.02f);
    check(f.phase() == Phase::Ground && !f.imuFast(), "a minute of dead-still hover: the ground, the IMU at 1 Hz");
    if (!stillToRelease) {
      s.run(10000, [](uint32_t) { return 690.0f; }, 5.0f, 1.0f);
      check(f.phase() == Phase::Air, "the drone moving again: the air on the hint before any release");
    }
    s.run(50, [](uint32_t) { return 690.0f; }, 0.3f, 0.02f);
    const uint32_t ms = untilAir(s, 7000, 9.0f, dps(450.0f), dps(650.0f));
    printf("released from a hover that landed, %s: the air %u ms in\n",
           stillToRelease ? "straight out of the still hover" : "after the drone moved again", static_cast<unsigned int>(ms));
    check(f.phase() == Phase::Air && ms <= (stillToRelease ? 2100u : 1100u),
          stillToRelease ? "released from the still hover: the air within about 2 s" : "released after moving again: the air within about a second");
  }
  setSystemSink(nullptr, nullptr);
}

// From the ground or boot-air a climb on the live altitude is the air whatever the IMU says: board
// 15's drone climbed 45 m on 2026-11-08 reading quiet most of the way and the drop was missed
static void climbs()
{
  using namespace ChipSatLog;
  static std::vector<SystemEvent> seen;
  seen.clear();
  setSystemSink([](void *, const SystemEvent &e) { seen.push_back(e); }, nullptr);
  auto knocks = [](uint32_t t) { return t % 3000 < 50 ? 3.0f : 0.5f; };   // one moving reading in 3 s
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 555.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    auto climb = [t0](uint32_t t) { return 555.0f + 2.0f * (t - t0) / 1000.0f; };
    s.runImu(7000, climb, knocks, 0.05f);
    expect(f, Phase::Ground, "a smooth 2 m/s climb with a moving reading every 3 s: 14 m up, not yet");
    s.runImu(3000, climb, knocks, 0.05f);
    expect(f, Phase::Air, "and the air once a whole second's lowest is 15 m up");
    bool climbed = false;
    for (const SystemEvent &e : seen) {
      climbed = climbed || (e.what == kPhaseToAir && e.reason == kWhyClimb);
    }
    check(climbed, "said as climb");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 555.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(10000, [t0](uint32_t t) { return 555.0f + 2.0f * (t - t0) / 1000.0f; }, 0.3f, 0.02f);
    expect(f, Phase::Air, "the same climb with the IMU still: the air at 15 m too");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 555.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.runImu(30000, [t0](uint32_t t) { return 555.0f + 2.0f * (t - t0) / 1000.0f; }, [](uint32_t) { return 0.5f; }, 0.05f);
    s.runImu(20000, [](uint32_t) { return 615.0f; }, knocks, 0.05f);
    expect(f, Phase::Air, "60 m up still, then moving: the air");
  }
  // a rise with a 3 s brown-out of the IMU, bad readings and a few stray moving ones in it
  {
    FlightPhase f;
    Sim s{f, 0};
    s.run(60000, [](uint32_t) { return 555.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    auto climb = [t0](uint32_t t) { return 555.0f + 2.0f * (t - t0) / 1000.0f; };
    s.run(3000, climb, 0.3f, 0.02f);
    s.run(3000, climb, 0.0f, 0.0f, false);
    for (int i = 0; i < 3; i++) {
      f.imuSample(s.now, NAN, NAN, NAN, {0.0f, 0.0f, 0.0f, 0.0f});
      s.run(50, climb, 3.0f, 0.02f);
    }
    s.run(3700, climb, 0.3f, 0.02f);
    expect(f, Phase::Air, "a 2 m/s rise through a 3 s brown-out, bad and stray readings: the air at 15 m");
  }
  // a desk board knocked now and then, its barometer 46 m high on every other reading: the lowest
  // of each second is the desk
  {
    FlightPhase f;
    Sim s{f, 0};
    s.runImu(600000, [](uint32_t t) { return 340.0f + ((t / 200) % 2 ? 46.0f : 0.0f); }, knocks, 0.05f);
    check(!s.sawAir, "a desk, knocked every 3 s, the barometer 46 m high every other reading: never the air");
  }
  setSystemSink(nullptr, nullptr);
}

// The orientation turning more than kTurnedRad in kTurnSpanMs counts as movement for the quiet
// timers, not for the air: a slow fall that keeps turning never lands, a heading snap on a still
// board only holds the landing off
static void turning()
{
  auto flown = [](Sim &s) {
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
    const uint32_t t0 = s.now;
    s.run(60000, [t0](uint32_t t) { return 120.0f + 3.0f * (t - t0) / 1000.0f; }, 5.0f, 1.0f);
  };
  for (int turns = 0; turns < 2; turns++) {
    FlightPhase f;
    Sim s{f, 0};
    s.yaw = 0.0f;
    flown(s);
    s.yawRate = turns ? 0.1745f : 0.0f;   // 10 deg/s, under the gyro's moving line
    const uint32_t r = s.now;
    s.sawGround = false;
    s.run(90000, [r](uint32_t t) { return 300.0f - 3.0f * (t - r) / 1000.0f; }, 0.3f, 0.1745f);
    check(turns ? f.phase() == Phase::Air && !s.sawGround : f.phase() == Phase::Ground,
          turns ? "a steady 3 m/s fall turning 10 deg/s: still the air after 90 s"
                : "the same fall not turning: the ground at a minute");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.yaw = 0.0f;
    flown(s);
    const uint32_t r = s.now;
    s.run(8000, [r](uint32_t t) { return 300.0f - 10.0f * (t - r) / 1000.0f; }, 9.0f, 8.0f);
    s.run(20000, [](uint32_t) { return 220.0f; }, 0.04f, 0.009f);
    s.yaw = 1.2217f;   // 70 deg, as board 17 boot 4 jumped lying still
    s.sawGround = false;
    s.run(34000, [](uint32_t) { return 220.0f; }, 0.04f, 0.009f);
    check(f.phase() == Phase::Air && !s.sawGround, "a 70 deg snap 20 s into rest: not yet the ground 34 s after it");
    s.run(2000, [](uint32_t) { return 220.0f; }, 0.04f, 0.009f);
    expect(f, Phase::Ground, "the ground 30 s after the snap's 5 s it is compared across");
  }
  {
    FlightPhase f;
    Sim s{f, 0};
    s.yaw = 0.0f;
    s.run(120000, [](uint32_t) { return 120.0f; }, 0.04f, 0.009f);
    s.yaw = 1.2217f;
    s.run(60000, [](uint32_t) { return 120.0f; }, 0.04f, 0.009f);
    check(!s.sawAir, "a 70 deg snap on a board on the ground: never the air");
  }
}

// Asleep after landing the controller hands the phase one barometer reading a wake and no IMU
// readings, a minute apart. The gap is no movement, the ground follows weather of 6 m an hour, one
// reading 16 m up is the climb, and tilted() is a moving reading at 1 Hz
static void asleepWakes()
{
  using namespace ChipSatLog;
  static std::vector<SystemEvent> seen;
  seen.clear();
  setSystemSink([](void *, const SystemEvent &e) { seen.push_back(e); }, nullptr);
  FlightPhase f;
  Sim s{f, 0};
  s.paced = true;
  s.run(40000, [](uint32_t) { return 120.0f; }, 0.1f, 0.01f);
  float weather = 120.0f;
  for (int wake = 0; wake < 60; ++wake) {
    s.now += 60000;
    weather += 0.1f;
    f.imuLost(s.now);
    f.envSample(s.now, weather);
  }
  check(f.phase() == Phase::Ground && !f.imuFast() && !f.logsAir(), "an hour of wakes a minute apart, the weather 6 m up: the ground at 1 Hz");
  s.nextEnv = s.now;
  s.run(10000, [weather](uint32_t) { return weather; }, 0.1f, 0.01f);
  check(f.phase() == Phase::Ground && !f.imuFast(), "readings again after the hour: nothing moved in it");
  f.tilted(s.now);
  check(f.imuFast() && f.phase() == Phase::Ground, "tilted: 20 Hz, still the ground");
  s.run(31000, [weather](uint32_t) { return weather; }, 0.1f, 0.01f);
  check(!f.imuFast() && !s.sawAir, "30 s quiet after it: 1 Hz again, never the air");
  bool tilt = false;
  for (const SystemEvent &e : seen) {
    tilt = tilt || (e.source == kSourcePhase && e.what == kPhaseImuFast && e.part == kPartImu && e.reason == kWhyTilted);
  }
  check(tilt, "the step to 20 Hz says tilted");
  s.now += 60000;
  f.imuLost(s.now);
  f.envSample(s.now, weather + 16.0f);
  expect(f, Phase::Air, "a wake a minute later 16 m up: the air on that one reading");
  setSystemSink(nullptr, nullptr);
}

int main()
{
  bootAir();
  bootAirFlat();
  waking();
  asleepWakes();
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
  stillJumpyBarometer();
  falls();
  climbs();
  turning();
  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
