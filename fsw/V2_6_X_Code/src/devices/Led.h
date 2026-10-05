#pragma once

namespace ChipSatDevices
{

// Status LED. Nothing in the constructor, it would run before setup()
class Led
{
public:
  void begin();   // first thing in setup
  void transmitStarted();
  void transmitEnded();
};

} // namespace ChipSatDevices
