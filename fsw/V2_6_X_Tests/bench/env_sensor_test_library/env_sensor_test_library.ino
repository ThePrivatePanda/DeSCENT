// Same as env_sensor_test but through the Adafruit BME280 library (2.3.0), set up the way the old
// flight code had it: begin() with its defaults. Prints the same columns so the two can be compared.
// Console PB7/PB6 at 115200.

#include <Wire.h>
#include <Adafruit_BME280.h>

HardwareSerial console(PB7, PB6);
Adafruit_BME280 bme;

constexpr float kSeaLevelHpa = 1013.25f;

void setup()
{
  console.begin(115200);
  delay(300);
  console.println();

  Wire.setSDA(PA15);
  Wire.setSCL(PB15);
  Wire.begin();

  const bool found = bme.begin(0x77, &Wire);
  console.print("chip_id=0x");
  console.println(bme.sensorID(), HEX);
  console.print("env found=");
  console.println(found);
  console.println("seconds  celsius  hpa  humidity  metres  ms");
}

void loop()
{
  const uint32_t start = millis();
  const float celsius = bme.readTemperature();
  const float hpa = bme.readPressure() / 100.0f;
  const float humidity = bme.readHumidity();
  const float metres = bme.readAltitude(kSeaLevelHpa);
  const uint32_t took = millis() - start;

  console.print(millis() / 1000);
  console.print("  ");
  console.print(celsius, 2);
  console.print("  ");
  console.print(hpa, 2);
  console.print("  ");
  console.print(humidity, 2);
  console.print("  ");
  console.print(metres, 2);
  console.print("  ");
  console.println(took);

  delay(1000);
}
