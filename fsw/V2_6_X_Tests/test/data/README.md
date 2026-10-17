# Recorded flights for the host tests

`drop-2026-10-13-board13.csv`: board 13's radio packets from the two drone drops of 2026-10-13,
one row per packet received (about 3 a second, with loss), as `ms,accel,gyro,gpsValid,gpsAltM`:
ms since the first packet, the magnitude of the IMU's linear acceleration in m/s2 and of its
rotation rate in rad/s, whether the GPS had a fix, and its altitude above sea level in m.
The barometer was dead on that board all day, so there is no pressure altitude. Release 1 at about
429 s, impact 1 at 471 s (77 m/s2), release 2 at about 1051 s, last packet of that fall at 1062 s,
then nothing until 1275 s. Made from `~/DeSCENT/descent-ground-2026-10-13T21-47-06.log` with
`ground/web/js/packet.js`.
