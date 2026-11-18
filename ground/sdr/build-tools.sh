#!/bin/sh
# Builds the two programs the app's built-in SDR runs, for the OS this runs on, into OUT:
# file_lora from rxx_lora with rxx_lora-ldro.patch and rxx_lora-stdin.patch, and rtl_sdr from
# osmocom rtl-sdr with rtl-sdr-stdout.patch, each linked so that it runs with nothing installed,
# and THIRD_PARTY/ with their licences and where every source came from.
#
#   ground/sdr/build-tools.sh OUT [SOURCES.tar.gz]
#
# CI runs it (.github/workflows/release.yml): on Linux inside Alpine, so that musl links both
# fully statically, with fftw and libusb built here; on Windows in an MSYS2 UCRT64 shell against
# its static fftw and libusb; on macOS against Homebrew's static libraries. Given a second path it
# also writes the sources of everything it built there, as a .tar.gz.
set -eu

RXX_LORA=f32079fdd0b190c58dbe07c385eb3f625d5f2301
RTL_SDR=2.0.3
FFTW=3.3.10
LIBUSB=1.0.30

here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$1/THIRD_PARTY/patches"
out=$(cd "$1" && pwd)
work=$(mktemp -d)
cd "$work"

case $(uname -s) in
    MINGW*|MSYS*) os=windows exe=.exe ;;
    Linux) os=linux exe= ;;
    Darwin) os=macos exe= ;;
    *) echo "build-tools.sh: no recipe for $(uname -s)" >&2; exit 1 ;;
esac
# a CPU from the last fifteen years, not the one CI happens to run on
case $(uname -m) in
    x86_64) arch=-march=x86-64-v2 ;;
    *) arch= ;;
esac

# as committed, so the patches apply on Windows too
git -c core.autocrlf=false clone -q https://github.com/xdab/rxx_lora
git -C rxx_lora checkout -q $RXX_LORA
git -C rxx_lora apply "$here/rxx_lora-ldro.patch" "$here/rxx_lora-stdin.patch"
curl -fsSL -o rtl-sdr-$RTL_SDR.tar.gz https://github.com/osmocom/rtl-sdr/archive/refs/tags/v$RTL_SDR.tar.gz
curl -fsSL -o fftw-$FFTW.tar.gz https://www.fftw.org/fftw-$FFTW.tar.gz
curl -fsSL -o libusb-$LIBUSB.tar.bz2 https://github.com/libusb/libusb/releases/download/v$LIBUSB/libusb-$LIBUSB.tar.bz2
tar xzf rtl-sdr-$RTL_SDR.tar.gz
tar xzf fftw-$FFTW.tar.gz
tar xjf libusb-$LIBUSB.tar.bz2
(cd rtl-sdr-$RTL_SDR && patch -p1 < "$here/rtl-sdr-stdout.patch")

rtl=rtl-sdr-$RTL_SDR/src
rtl_src="$rtl/rtl_sdr.c $rtl/convenience/convenience.c $rtl/librtlsdr.c $rtl/tuner_e4k.c $rtl/tuner_fc0012.c $rtl/tuner_fc0013.c $rtl/tuner_fc2580.c $rtl/tuner_r82xx.c"
case $os in
    linux)
        prefix=$work/prefix
        (cd fftw-$FFTW && ./configure -q --prefix="$prefix" --enable-float --enable-sse2 --enable-avx \
            --disable-shared --enable-static --disable-fortran && make -s -j2 && make -s install)
        (cd libusb-$LIBUSB && ./configure -q --prefix="$prefix" --disable-udev --disable-shared \
            --enable-static && make -s -j2 && make -s install)
        inc="-I$prefix/include" fftw_libs="-static -L$prefix/lib -lfftw3f"
        usb_inc="-I$prefix/include/libusb-1.0" usb_libs="-static -L$prefix/lib -lusb-1.0"
        # takes the dongle from the kernel's TV driver, which otherwise holds it on most distros
        rtl_src="-DDETACH_KERNEL_DRIVER=1 $rtl_src"
        built="fftw $FFTW and libusb $LIBUSB built from the source here, with musl, in Alpine $(cat /etc/alpine-release)"
        ;;
    windows)
        inc= fftw_libs="-static -lfftw3f"
        usb_inc="-I$MINGW_PREFIX/include/libusb-1.0" usb_libs="-static -lusb-1.0"
        rtl_src="$rtl_src $rtl/getopt/getopt.c"
        built="fftw, libusb and the compiler's runtime from MSYS2 UCRT64 (https://packages.msys2.org): $(pacman -Q mingw-w64-ucrt-x86_64-fftw mingw-w64-ucrt-x86_64-libusb mingw-w64-ucrt-x86_64-gcc | tr '\n' ',' | sed 's/,$//; s/,/, /g')"
        ;;
    macos)
        prefix=$(brew --prefix)
        inc="-I$prefix/include" fftw_libs="$prefix/lib/libfftw3f.a"
        usb_inc="-I$prefix/include/libusb-1.0"
        usb_libs="$prefix/lib/libusb-1.0.a -lobjc -framework IOKit -framework CoreFoundation -framework Security"
        built="fftw and libusb from Homebrew: $(brew list --versions fftw libusb | tr '\n' ',' | sed 's/,$//; s/,/, /g')"
        ;;
esac

make -s -C rxx_lora file CFLAGS="-O3 $arch -Wall -Wextra -I./include $inc" LIBS="$fftw_libs -lpthread -lm"
cp rxx_lora/bin/file_lora$exe "$out/"
# gnu17: rtl-sdr's getopt.c is K&R C, which C23 (GCC 15's default) refuses
# shellcheck disable=SC2086
${CC:-gcc} -std=gnu17 -O2 $arch -Drtlsdr_STATIC -I rtl-sdr-$RTL_SDR/include $usb_inc -o "$out/rtl_sdr$exe" $rtl_src $usb_libs -lpthread -lm

case $os in
    linux) file "$out/file_lora" "$out/rtl_sdr" 2>/dev/null || true ;;
    windows) objdump -p "$out/file_lora.exe" "$out/rtl_sdr.exe" | grep 'DLL Name' | sort -u ;;
    macos) otool -L "$out/file_lora" "$out/rtl_sdr" ;;
esac

t=$out/THIRD_PARTY
cp rxx_lora/LICENSE.md "$t/rxx_lora-LICENSE.md"
cp rtl-sdr-$RTL_SDR/COPYING "$t/rtl-sdr-COPYING"
cp fftw-$FFTW/COPYING "$t/fftw-COPYING"
cp libusb-$LIBUSB/COPYING "$t/libusb-COPYING"
cp "$here/rxx_lora-ldro.patch" "$here/rxx_lora-stdin.patch" "$here/rtl-sdr-stdout.patch" "$t/patches/"
if [ $os = linux ]; then
    curl -fsSL -o "$t/musl-COPYRIGHT" https://git.musl-libc.org/cgit/musl/plain/COPYRIGHT
fi
if [ $os = windows ]; then
    cp "$MINGW_PREFIX/share/licenses/winpthreads/COPYING" "$t/winpthreads-COPYING"
fi
cat > "$t/SOURCES.txt" <<EOF
The SDR programs DeSCENT Ground carries, and where their sources are

file_lora$exe is rxx_lora (https://github.com/xdab/rxx_lora, GPL-3.0, rxx_lora-LICENSE.md) at
commit $RXX_LORA, with patches/rxx_lora-ldro.patch and patches/rxx_lora-stdin.patch, linked
statically with fftw (https://www.fftw.org, GPL-2.0-or-later, fftw-COPYING).

rtl_sdr$exe is osmocom rtl-sdr $RTL_SDR (https://github.com/osmocom/rtl-sdr, GPL-2.0-or-later,
rtl-sdr-COPYING) with patches/rtl-sdr-stdout.patch, linked statically with libusb
(https://libusb.info, LGPL-2.1-or-later, libusb-COPYING).

Built for $os by ground/sdr/build-tools.sh in https://github.com/ThePrivatePanda/DeSCENT, which
CI runs from .github/workflows/release.yml. $built.

Every release attaches descent-sdr-sources.tar.gz: rxx_lora at that commit, the rtl-sdr, fftw and
libusb source tarballs, the patches and the build script. Unpack it and run build-tools.sh from it
to build these programs again.
EOF

if [ $# -ge 2 ]; then
    s=$work/descent-sdr-sources
    mkdir -p "$s"
    git -C rxx_lora archive --prefix=rxx_lora/ -o "$s/rxx_lora-$RXX_LORA.tar.gz" $RXX_LORA
    cp rtl-sdr-$RTL_SDR.tar.gz fftw-$FFTW.tar.gz libusb-$LIBUSB.tar.bz2 "$s/"
    cp "$here/build-tools.sh" "$here/rxx_lora-ldro.patch" "$here/rxx_lora-stdin.patch" "$here/rtl-sdr-stdout.patch" "$s/"
    tar czf "$2" -C "$work" descent-sdr-sources
fi
rm -rf "$work"
