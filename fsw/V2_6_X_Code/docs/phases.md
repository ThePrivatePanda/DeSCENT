# Phases, power states and what decides them

How a board decides it is on the ground, in the air or landed, what each sensor does in each
state, and what happens when a sensor is missing. The code is app/Phase.cpp (the phase),
app/FlightController.cpp (the landed states and power) and app/FlashLog.cpp (what reaches the chip).
Names in brackets are the constants in config.h; the numbers are what they are set to today.

As of 2026-11-07 only boot-air's end and the 1 Hz ground IMU stepping up have been seen on a board
(pull 1813-board15). Take-off, landing and the landed states are host-tested only, and every
threshold below was picked, not measured.

## Inputs

- A moving IMU reading: linear acceleration at least 2 m/s2 [kQuietAccelMps2] or rotation at least
  0.35 rad/s [kQuietGyroRadPerSec]. Anything under both is quiet. Readings come every 50 ms at
  20 Hz or every 1 s at 1 Hz.
- The motion window: the last 10 s [kMotionWindowMs] in 50 ms slots. It says "moving" when at least
  75 % of the slots got a reading [kMotionFilled] and at least 80 % of those moved [kMotionFraction].
  At 1 Hz it never fills, which is why the ground steps the IMU up first.
- The altitude source: the barometer while it is live (a reading in the last 3 s,
  [kAltitudeStaleMs]), else the GPS, else none. Each keeps its own ground baseline, which follows the
  altitude slowly while the sat is still (vertical rate under 0.3 m/s), so weather and a walk to the
  drone don't count as a climb, and its own peak in the air.
- A climb: the lowest reading of the last whole second at least 15 m above the baseline on the
  barometer [kRiseGainM], 30 m on GPS [kRiseGainGpsM]. The lowest, because the barometer read up to
  46 m high during 22 dBm transmits on 2026-11-01 and that offset only ever lifts readings.
- Flat: over a span of whole seconds, every second's lowest reading within 3 m of the others'
  lowest and its highest within 3 m of their highest [kFlatAltitudeM], 15 m on GPS
  [kFlatAltitudeGpsM]. An offset that comes and goes inside every second leaves both alone; one that
  lifts some seconds and not others breaks the span.
- Falling: four whole seconds in a row, each lower than the last by 5 m on both its lowest and its
  highest reading [kDescentBins, kDescentStepM], barometer only.
- The IMU dead: no valid reading for 60 s [kImuDeadMs]. A shorter gap holds the phase and counts as
  no movement.

## States

Boot-air. Every reset starts here, since a reset can come mid-fall. The radio is on the ground
rate, the flash log as in the air, the IMU at 20 Hz. It ends when
- the IMU has been quiet 30 s [kLandedMs] and, if an altitude source is live, it has been flat the
  same 30 s; or
- with the IMU dead, the altitude has stayed within 4 m (8 m on GPS) for 120 s [kLandedNoImuMs]; or
- uptime reaches 5 min [kBootAirMaxMs], whatever else.
With the IMU dead and no altitude, only the 5 min ends it. A real take-off during boot-air goes
straight to the air.

Ground. The IMU at 1 Hz [kImuGroundIntervalMs], GPS on, barometer every 200 ms. When 3 of the last
5 readings move [kWakeMoving, kWakeReadings] the IMU goes to 20 Hz, still on the ground; 30 s with no
moving reading [kWakeQuietMs] puts it back to 1 Hz.

Ground to air needs the motion window moving at 20 Hz and
- no altitude source live, or
- a climb on the live one, or
- the window moving without a break for 60 s [kMotionHeldMs], whatever the altitude says, for a
  pressurised cabin. A dip under 80 % starts the 60 s again.
With the IMU dead, a climb alone is the air, judged on the latest reading rather than the second's
lowest, so a barometer offset can do it then. With the IMU working the barometer alone never puts a
board in the air.

Air. Everything at full rate. Air to ground needs the IMU quiet and
- with an altitude source live: quiet 30 s, flat 30 s and low, meaning below half the climb or the
  climb was under 15 m; or quiet and flat 5 min at any height [kLandedHighMs], for a tree or a roof.
  A hover is quiet, flat and high, so it waits the 5 min;
- with an altitude seen during the flight but none live now: quiet 30 s;
- with no altitude live at any point in the air: quiet 3 min [kGroundQuietMs]. The BME280 reads
  nothing below 300 hPa, about 9 km, so a high fall that stops tumbling has nothing else to hold it;
- with the IMU dead: below half the climb and within 4 m (8 m on GPS) for 120 s.
Falling resets the quiet timer, so a steady fall that stopped tumbling stays in the air.

Landed, looking. Ground after a real air spell this boot (boot-air doesn't count). The ring keeps
writing until everything from the air is on the chip, then the console says `I FLASH ... landed`.
The IMU is at 1 Hz with the same step to 20 Hz. The GPS looks for a fix to hold: the receiver's
trusted 3D fix with its horizontal accuracy under 10 m [kFixGoodMm], or under 20 m
[kFixFairMm] once 5 min have passed since landing [kFixRelaxMs], five in a row [kFixRun] with no gap
between them, the first no sooner than 30 s after landing [kFixAfterLandingMs]. The best of the five
is kept. With no fix it keeps looking for as long as the battery lasts.

Landed, asleep. Five good fixes and the fall on the chip. The GPS goes to software standby and the
driver writes nothing more to it. Packets go out at the ground transmit gap, moved up to 2 s either
way each time [kLandedJitterMs], carrying the held fix with the GPS valid bit set, a battery reading
and one barometer reading taken just before. Records go out at the ground log gap and are written at
once. The flash chip is powered down between records. Between work the MCU is in Stop2
[kDeepSleepWhenLanded] until the next IMU reading, packet or record, never longer than 4 s
[kSleepMaxMs] because the 5 s watchdog keeps counting. The console says `asleep nexts=` once per
packet. Three moving readings at 1 Hz, or going to the air on altitude with the IMU dead, wake
everything and start the fix search again.

The radio sleeps (warm start) after every transmission in every state. The MCU idles on WFI between
loop passes in every state but asleep [kSleepWhenIdle].

## What reaches the chip

Every record goes into a 600 record RAM ring first, 30 s at 20 Hz. In boot-air and the air every
record is written as fast as the chip takes it. On the ground records are still made at the faster
of the ground and air log gaps so the ring always holds the last 30 s, but only one packet record
per ground log gap is kept (every one with a gap of 0), and kept records wait in the ring until 580
are pending, so a take-off can write the 30 s before it in time order. A ground gap of 0 writes
everything at once. Time anchors and events are always kept. Going to the air writes the whole ring
first. Asleep, records are made at the ground log gap and written at once.

Every change above is a system event on the chip (phase source: toair, toground, descent,
bootairend, imufast, imuslow, asleep, awake, with the reason), so a pull shows when and why.

## Known limits

- An offset that lifts some seconds and not others breaks "flat", which holds boot-air's end and a
  low landing off until their 5 min limits. Not seen against real offset data yet.
- Asleep with the IMU dead the barometer is read once a packet, so it isn't live and can't take the
  board back to the air.
- A board in water keeps moving and stays in the air. Grass in wind against the 2 m/s2 line hasn't
  been measured on these boards.
- The ground app's Live screen doesn't show the landed states or the held fix; its Analysis tab
  does, from the system events in a pull.
