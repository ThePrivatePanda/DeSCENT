// What the firmware would make of an image's settings block. Takes a .bin, finds the block the way
// the firmware does, and runs the firmware's own parse() over it - so a tool that writes blocks and
// the board that reads them can be checked against each other without a board.
//   g++ -std=gnu++17 -Itest/mock -I../V2_6_X_Code/src -DCHIPSAT_RADIO_MODULE=1 \
//     test/read_block.cpp ../V2_6_X_Code/src/Settings.cpp -o /tmp/readblock && /tmp/readblock image.bin
#include <cstdio>
#include <cstring>
#include <vector>

#include "Settings.h"

using namespace ChipSatSettings;

int main(int argc, char** argv)
{
  if (argc < 2) {
    printf("usage: read_block <image.bin>\n");
    return 2;
  }
  FILE* f = fopen(argv[1], "rb");
  if (f == nullptr) {
    printf("cannot open %s\n", argv[1]);
    return 2;
  }
  std::vector<unsigned char> image;
  unsigned char buf[4096];
  size_t n;
  while ((n = fread(buf, 1, sizeof buf, f)) > 0) {
    image.insert(image.end(), buf, buf + n);
  }
  fclose(f);

  int found = 0;
  size_t at = 0;
  for (size_t i = 0; i + kBlockBytes <= image.size(); i++) {
    if (memcmp(&image[i], kMagic, sizeof kMagic) == 0) {
      found++;
      at = i;
    }
  }
  printf("%s: %d block(s)\n", argv[1], found);
  if (found != 1) {
    return 1;
  }

  Settings s;
  const char* why = nullptr;
  const Parse result = parse(&image[at], &s, &why);
  printf("  at 0x%zX\n", at);
  switch (result) {
    case Parse::kGood:
      printf("  the board would run: csid %u  module %s  sf %u  dbm %d  set by %s\n",
             s.csid, s.module == kModuleHp ? "hp" : "le", s.spreadingFactor, s.targetDbm,
             s.setBy == kSetByTool ? "tool" : "build");
      for (size_t p = 0; p < kPhases; p++) {
        static const char* names[4] = {"waiting", "rising", "falling", "landed"};
        printf("    %-8s every %u ms%s", names[p], s.rate[p].fixedMs, s.rate[p].fromSoc ? ", by battery:" : "\n");
        if (s.rate[p].fromSoc) {
          for (size_t i = 0; i < kSocBands; i++) {
            if (i < kSocBands - 1) printf(" %u ms >= %u %%,", s.rate[p].socIntervalMs[i], s.rate[p].socPercent[i]);
            else printf(" then %u ms\n", s.rate[p].socIntervalMs[i]);
          }
        }
      }
      return 0;
    case Parse::kDamaged:
      printf("  REFUSED, running the values it was built with: %s\n", why);
      return 1;
    case Parse::kRejected:
      printf("  REFUSED, running the values it was built with: %s\n", why);
      return 1;
  }
  return 1;
}
