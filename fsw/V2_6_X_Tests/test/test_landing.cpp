// Landing.cpp on the laptop: does it hold off during free fall, and does it notice the ground
//   g++ -std=gnu++17 -Wall -Wextra -Itest/mock -I../V2_6_X_Code/src/app test/test_landing.cpp ../V2_6_X_Code/src/app/Landing.cpp -o /tmp/land && /tmp/land
#include <cstdio>
#include "Landing.h"

using ChipSatApp::Landing;

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

// feeds the same reading every 50 ms for as long as asked
static void hold(Landing &l, uint32_t &nowMs, uint32_t forMs, float accel, float pressure)
{
  const uint32_t until = nowMs + forMs;
  while (nowMs < until) {
    l.update(nowMs, accel, pressure, true);
    nowMs += 50;
  }
}

int main()
{
  // on the pad: quiet and steady, but too early to call it landed
  {
    Landing l;
    uint32_t now = 0;
    hold(l, now, 50000, 0.05f, 993.0f);
    check(!l.landed(), "quiet on the pad for 50 s is not landed");
  }

  // free fall: the IMU reads near zero, but the pressure climbs the whole way down
  {
    Landing l;
    uint32_t now = 70000;
    float pressure = 700.0f;
    for (int i = 0; i < 400; ++i) {   // 20 s of falling, about 1 hPa a second
      l.update(now, 0.02f, pressure, true);
      now += 50;
      pressure += 0.05f;
    }
    check(!l.landed(), "free fall is not landed, however quiet the IMU is");
  }

  // on the ground: quiet and the pressure stops moving
  {
    Landing l;
    uint32_t now = 70000;
    hold(l, now, 9000, 0.05f, 993.0f);
    check(!l.landed(), "9 s of quiet is not enough");
    hold(l, now, 2000, 0.05f, 993.0f);
    check(l.landed(), "landed after the full quiet time");

    // picked up
    hold(l, now, 200, 3.0f, 993.0f);
    check(!l.landed(), "movement means not landed any more");
    hold(l, now, 11000, 0.05f, 993.0f);
    check(l.landed(), "and it settles again");
  }

  // weather drifting slowly shouldn't unset it
  {
    Landing l;
    uint32_t now = 70000;
    hold(l, now, 11000, 0.05f, 993.0f);
    check(l.landed(), "landed");
    float pressure = 993.0f;
    for (int i = 0; i < 200; ++i) {   // 0.4 hPa over 10 s, under the limit
      l.update(now, 0.05f, pressure, true);
      now += 50;
      pressure += 0.002f;
    }
    check(l.landed(), "a slow drift keeps it landed");
  }

  // readings we can't believe leave the decision alone
  {
    Landing l;
    uint32_t now = 70000;
    hold(l, now, 11000, 0.05f, 993.0f);
    check(l.landed(), "landed");
    for (int i = 0; i < 100; ++i) {
      l.update(now, 50.0f, 500.0f, false);
      now += 50;
    }
    check(l.landed(), "invalid readings change nothing");
  }

  printf("%s: %d checks, %d failed\n", failures == 0 ? "PASSED" : "FAILED", checks, failures);
  return failures == 0 ? 0 : 1;
}
