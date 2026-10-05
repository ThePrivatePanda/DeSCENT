// Same thing as gps_test, but through the SparkFun library the old flight code used, so the two can be
// compared side by side on the same receiver. Console PB7/PB6 at 115200.

#include <SparkFun_u-blox_GNSS_Arduino_Library.h>

HardwareSerial console(PB7, PB6);
HardwareSerial gps(PC1, PC0);
SFE_UBLOX_GNSS gnss;

void setup()
{
  console.begin(115200);
  gps.begin(9600);
  delay(500);
  console.println();
  console.println("gps_test_library");

  if (!gnss.begin(gps)) {
    console.println("gnss.begin failed");
    return;
  }
  console.print("uart ubx only: ");
  console.println(gnss.setUART1Output(COM_TYPE_UBX));
  console.print("airborne dynamic model: ");
  console.println(gnss.setVal8(0x20110021, 8, 0x03));
  console.print("measurement rate 1000 ms: ");
  console.println(gnss.setVal16(0x30210001, 1000, 0x03));
  console.print("navigation rate 1: ");
  console.println(gnss.setVal16(0x30210002, 1, 0x03));
  console.print("full power mode: ");
  console.println(gnss.setVal8(0x20D00001, 0, 0x03));

  console.println();
  console.println("seconds  fix  sats  lat  lon  alt_m  invalid_llh  h_acc_m");
}

void loop()
{
  console.print(millis() / 1000);
  console.print("  ");

  if (!gnss.getPVT(1200)) {
    console.println("no position came back");
    delay(1000);
    return;
  }

  console.print(gnss.getFixType());
  console.print("  ");
  console.print(gnss.getSIV());
  console.print("  ");
  console.print(gnss.getLatitude() / 1e7, 7);
  console.print("  ");
  console.print(gnss.getLongitude() / 1e7, 7);
  console.print("  ");
  console.print(gnss.getAltitudeMSL() / 1000.0, 2);
  console.print("  ");
  console.print(gnss.getInvalidLlh());
  console.print("  ");
  console.println(gnss.getHorizontalAccEst() / 1000.0, 1);

  delay(1000);
}
