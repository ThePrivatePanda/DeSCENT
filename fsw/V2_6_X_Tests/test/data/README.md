# Recorded flights for the host tests

`drop-2026-10-13-board13.csv`: board 13's radio packets from the two drone drops of 2026-10-13,
one row per packet received (about 3 a second, with loss), as `ms,accel,gyro,gpsValid,gpsAltM`:
ms since the first packet, the magnitude of the IMU's linear acceleration in m/s2 and of its
rotation rate in rad/s, whether the GPS had a fix, and its altitude above sea level in m.
The barometer was dead on that board all day, so there is no pressure altitude. Release 1 at about
429 s, impact 1 at 471 s (77 m/s2), release 2 at about 1051 s, last packet of that fall at 1062 s,
then nothing until 1275 s. Made from `~/DeSCENT/descent-ground-2026-10-13T21-47-06.log` with
`ground/web/js/packet.js`.

`drop-2026-10-17-board9.csv`: board 9's own flash records from the two drops of 2026-10-17, boot
24 (drop 1) and boot 32 (drop 2), one row per packet record, as
`boot,ms,accel,gyro,imuValid,baroValid,baroAltM`: the boot count, the record's ms since boot, the
magnitudes of the linear acceleration in m/s2 and of the rotation rate in rad/s, whether both IMU
reports were valid, whether the barometer was, and its altitude in m. A record every 5 s on the
ground, about 18 a second in the air, one per 10 s after the board's own Landed. GPS never fixed
on that board, so there is no GPS column. Made from
`~/DeSCENT/2026-10-18/pulls/1432-board9/dump.txt` (the raw slots, CRC checked). The timelines are
in `docs/internals/09-drops-2026-10-17.md`.
