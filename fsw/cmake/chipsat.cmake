# Fetches what the build needs into deps/, then builds and flashes. Run through build.sh or build.cmd:
#   build              fetch what's missing, configure, build (the default)
#   setup              fetch everything, flashing tool too, and configure. Nothing is built
#   flash [--probe SN] build, then flash over SWD with the ST-Link, or the one with serial SN
#   probes             list the ST-Links plugged in
#   clean              delete build/
# Only CMake has to be installed. Everything else (Arduino core and CMSIS sources, compiler, ninja,
# OpenOCD) goes in deps/, pinned and checked by sha256, and skipped if already there

cmake_minimum_required(VERSION 3.21)

get_filename_component(ROOT "${CMAKE_CURRENT_LIST_DIR}/.." ABSOLUTE)
set(DEPS "${ROOT}/deps")
set(BUILD "${ROOT}/build")

# arguments after the script name
set(ACTION build)
set(PROBE "")
set(SKETCH_NAME "")
set(DEFINES "")
set(i 3)
while(i LESS CMAKE_ARGC)
  set(arg "${CMAKE_ARGV${i}}")
  if(arg STREQUAL "--probe")
    math(EXPR i "${i} + 1")
    set(PROBE "${CMAKE_ARGV${i}}")
  elseif(arg STREQUAL "--sketch")
    math(EXPR i "${i} + 1")
    set(SKETCH_NAME "${CMAKE_ARGV${i}}")
  elseif(arg MATCHES "^-D")
    list(APPEND DEFINES "${arg}")   # passed to the compiler, e.g. -DCHIPSAT_ID=12
  elseif(arg MATCHES "^(build|setup|flash|reset|probes|clean|help)$")
    set(ACTION "${arg}")
  elseif(NOT arg STREQUAL "--")
    message(FATAL_ERROR "unknown argument ${arg}, try: build, setup, flash [--probe SN] [--sketch NAME] [-DNAME=VALUE], probes, clean")
  endif()
  math(EXPR i "${i} + 1")
endwhile()

if(ACTION STREQUAL "help")
  message("build              fetch what's missing, configure, build")
  message("setup              fetch everything, flashing tool too, and configure. Nothing is built")
  message("flash [--probe SN] build, then flash with the ST-Link (the one with serial SN if there are several)")
  message("reset [--probe SN] restart the board over the ST-Link, without building or flashing")
  message("probes             list the ST-Links plugged in")
  message("clean              delete build/")
  message("")
  message("--sketch NAME      build a bench sketch from V2_6_X_Tests/bench instead of the flight software,")
  message("                   e.g. ./build.sh flash --sketch flash_dump. Leave it off to go back to flight")
  message("-DNAME=VALUE       passed to the compiler, for a board's own settings:")
  message("                   ./build.sh flash -DCHIPSAT_ID=12 -DCHIPSAT_SPREADING_FACTOR=9")
  return()
endif()

if(ACTION STREQUAL "clean")
  file(REMOVE_RECURSE "${BUILD}")
  message("removed ${BUILD}")
  return()
endif()

# ---------------------------------------------------------------------------------------------------
# which downloads fit this computer

cmake_host_system_information(RESULT HOST_OS QUERY OS_NAME)
cmake_host_system_information(RESULT HOST_CPU QUERY OS_PLATFORM)
string(TOLOWER "${HOST_CPU}" HOST_CPU)
if(HOST_CPU MATCHES "^(x86_64|amd64)$")
  set(HOST_CPU x64)
elseif(HOST_CPU MATCHES "^(aarch64|arm64)$")
  set(HOST_CPU arm64)
else()
  message(FATAL_ERROR "no prebuilt compiler for a ${HOST_CPU} computer")
endif()

# The Arduino STM32 core and CMSIS, as sources. Same files the Arduino IDE installs for core 2.12.0,
# hashes from ST's package index (package_stmicroelectronics_index.json)
set(CORE_VERSION 2.12.0)
set(CORE_URL https://github.com/stm32duino/Arduino_Core_STM32/releases/download/${CORE_VERSION}/STM32-${CORE_VERSION}.tar.bz2)
set(CORE_SHA 8d1ff26959c4cef55996f25b8403b029234264dc962793cff832e9ddc27e7544)
set(CMSIS_VERSION 6.2.0)
set(CMSIS_URL https://github.com/stm32duino/ArduinoModule-CMSIS/releases/download/${CMSIS_VERSION}/CMSIS-${CMSIS_VERSION}.tar.bz2)
set(CMSIS_SHA e9dcf458a333cda8d4332bb3a4058e8c3de1626c4a567250c60151b590a1d8ce)

set(GCC_VERSION 14.2.1-1.1)
set(OPENOCD_VERSION 0.12.0-7)
set(NINJA_VERSION 1.13.2)
set(GCC_URL https://github.com/xpack-dev-tools/arm-none-eabi-gcc-xpack/releases/download/v${GCC_VERSION})
set(OPENOCD_URL https://github.com/xpack-dev-tools/openocd-xpack/releases/download/v${OPENOCD_VERSION})
set(NINJA_URL https://github.com/ninja-build/ninja/releases/download/v${NINJA_VERSION})

# sha256 of each file, from the .sha files on the release pages (ninja: hashed on download)
if(HOST_OS STREQUAL "Linux" AND HOST_CPU STREQUAL "x64")
  set(TAG linux-x64)
  set(EXT tar.gz)
  set(GCC_SHA ed8c7d207a85d00da22b90cf80ab3b0b2c7600509afadf6b7149644e9d4790a6)
  set(OPENOCD_SHA 94b3790983beaf8ed57e646c0620dd66d705fddae03d290823a6ed3b439468d6)
  set(NINJA_FILE ninja-linux.zip)
  set(NINJA_SHA 5749cbc4e668273514150a80e387a957f933c6ed3f5f11e03fb30955e2bbead6)
elseif(HOST_OS STREQUAL "Linux" AND HOST_CPU STREQUAL "arm64")
  set(TAG linux-arm64)
  set(EXT tar.gz)
  set(GCC_SHA a1ac95c8d9347020d61e387e644a2c1806556b77162958a494d2f5f3d5fe7053)
  set(OPENOCD_SHA db73a3ab91c556ecec2405a7e02d404b11139df6aba1031cad94a7e6766d06cc)
  set(NINJA_FILE ninja-linux-aarch64.zip)
  set(NINJA_SHA fd2cacc8050a7f12a16a2e48f9e06fca5c14fc4c2bee2babb67b58be17a607fc)
elseif(HOST_OS STREQUAL "macOS" AND HOST_CPU STREQUAL "x64")
  set(TAG darwin-x64)
  set(EXT tar.gz)
  set(GCC_SHA b5bf8d5af099fd464d1543e5b8901308fb64116fa7a244426cacf4ff1b882fc7)
  set(OPENOCD_SHA 668ad25350103a4357e11629ec833eae5982e973889ce25bad0c2963e37fa8bf)
  set(NINJA_FILE ninja-mac.zip)
  set(NINJA_SHA c99048673aa765960a99cf10c6ddb9f1fad506099ff0a0e137ad8960a88f321b)
elseif(HOST_OS STREQUAL "macOS" AND HOST_CPU STREQUAL "arm64")
  set(TAG darwin-arm64)
  set(EXT tar.gz)
  set(GCC_SHA f52ea3760c53b25d726a7345be60a210736293db85f92daa39d1d22d34e2c995)
  set(OPENOCD_SHA 667342c086984f3e5a55b4e0d5f711add13fb04de040fca493303000e6c19327)
  set(NINJA_FILE ninja-mac.zip)   # universal binary
  set(NINJA_SHA c99048673aa765960a99cf10c6ddb9f1fad506099ff0a0e137ad8960a88f321b)
elseif(HOST_OS STREQUAL "Windows" AND HOST_CPU STREQUAL "x64")
  set(TAG win32-x64)
  set(EXT zip)
  set(GCC_SHA 0b2d496b383ba578182eb57b3f7d35ff510e36eda56257883b902fa07c3bba55)
  set(OPENOCD_SHA 6bfd3c97135aafef8affc9af1acf34fd0e2b9ca26044506f6abd7f95b7630052)
  set(NINJA_FILE ninja-win.zip)
  set(NINJA_SHA 07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65)
else()
  message(FATAL_ERROR "no prebuilt compiler for ${HOST_OS} on ${HOST_CPU}")
endif()

if(HOST_OS STREQUAL "Windows")
  set(EXE .exe)
endif()

# Downloads url into deps/<name>, unless deps/<name>/.ok already says this version is there.
# The archive's own top folder is dropped when it has one. Anything after the sha is a list of
# patterns, then only matching paths are unpacked
function(fetch name url sha)
  set(patterns ${ARGN})
  set(dest "${DEPS}/${name}")
  if(EXISTS "${dest}/.ok")
    file(READ "${dest}/.ok" have)
    if(have STREQUAL "${sha}")
      return()
    endif()
  endif()
  get_filename_component(file "${url}" NAME)
  set(archive "${DEPS}/${file}")
  message("fetching ${file}")
  file(DOWNLOAD "${url}" "${archive}" EXPECTED_HASH SHA256=${sha} SHOW_PROGRESS STATUS status TLS_VERIFY ON)
  list(GET status 0 code)
  if(NOT code EQUAL 0)
    file(REMOVE "${archive}")
    message(FATAL_ERROR "download of ${url} failed: ${status}")
  endif()
  file(REMOVE_RECURSE "${dest}" "${DEPS}/_unpack")
  if(patterns)
    file(ARCHIVE_EXTRACT INPUT "${archive}" DESTINATION "${DEPS}/_unpack" PATTERNS ${patterns})
  else()
    file(ARCHIVE_EXTRACT INPUT "${archive}" DESTINATION "${DEPS}/_unpack")
  endif()
  file(GLOB top LIST_DIRECTORIES true "${DEPS}/_unpack/*")
  list(LENGTH top count)
  if(count EQUAL 1 AND IS_DIRECTORY "${top}")
    file(RENAME "${top}" "${dest}")
  else()
    file(RENAME "${DEPS}/_unpack" "${dest}")
  endif()
  file(REMOVE_RECURSE "${DEPS}/_unpack")
  file(REMOVE "${archive}")
  file(WRITE "${dest}/.ok" "${sha}")
endfunction()


file(MAKE_DIRECTORY "${DEPS}")

# ---------------------------------------------------------------------------------------------------
# probes: list the ST-Links (USB vendor 0483) with the tools each OS already has

# Undoes the UTF-8 the kernel wrapped the raw serial bytes in: c2 XX is XX, c3 XX is XX plus 0x40.
# Everything else is already the byte it looks like. Trailing newline dropped
# The settings block is a const object that nothing but the firmware's own startup reads, and the
# link runs with --gc-sections. Stop calling it and the linker quietly deletes it from the image, so
# a build still succeeds and a tool flashing that image finds no block to configure. That happened
# once; this is here so it cannot happen quietly again
function(check_settings_block bin)
  if(NOT EXISTS "${bin}")
    return()
  endif()
  execute_process(COMMAND python3 -c
    "import sys;m=bytes([0x5A,0xA5,0x43,0x48,0x49,0x50,0x53,0x41,0x54,0x43,0x46,0x47,0x5A,0xA5,0x7B,0x3C]);b=open(sys.argv[1],'rb').read();n=0;i=b.find(m)
while i!=-1:
 n+=1;i=b.find(m,i+1)
print(n)"
    "${bin}" OUTPUT_VARIABLE found OUTPUT_STRIP_TRAILING_WHITESPACE RESULT_VARIABLE rc)
  if(NOT rc EQUAL 0)
    message("could not check the settings block (python3 missing?), carrying on")
    return()
  endif()
  if(NOT found EQUAL 1)
    message(FATAL_ERROR
      "the image holds ${found} copies of the settings block and it must hold exactly one. "
      "None means the linker dropped it because nothing reads it any more - check that "
      "ChipSatSettings::begin() is still called. More than one means something compares against "
      "the magic instead of validating the CRC, and a tool would not know which copy to write")
  endif()
endfunction()


function(probe_serial_hex bytes out)
  string(LENGTH "${bytes}" length)
  set(text "")
  set(i 0)
  while(i LESS length)
    string(SUBSTRING "${bytes}" ${i} 2 pair)
    math(EXPR value "0x${pair}")
    math(EXPR i "${i} + 2")
    if((value EQUAL 194 OR value EQUAL 195) AND i LESS length)
      string(SUBSTRING "${bytes}" ${i} 2 second)
      math(EXPR low "0x${second}")
      math(EXPR i "${i} + 2")
      math(EXPR value "${low} + (${value} - 194) * 64")
    endif()
    if(NOT value EQUAL 10)
      string(TOUPPER "${pair}" pair)
      math(EXPR value "${value}" OUTPUT_FORMAT HEXADECIMAL)
      string(SUBSTRING "${value}" 2 -1 value)
      string(TOUPPER "${value}" value)
      string(LENGTH "${value}" width)
      if(width EQUAL 1)
        set(value "0${value}")
      endif()
      string(APPEND text "${value}")
    endif()
  endwhile()
  set(${out} "${text}" PARENT_SCOPE)
endfunction()

if(ACTION STREQUAL "probes")
  set(found "")
  if(HOST_OS STREQUAL "Linux")
    file(GLOB vendors "/sys/bus/usb/devices/*/idVendor")
    foreach(v ${vendors})
      file(READ "${v}" vid)
      string(STRIP "${vid}" vid)
      if(vid STREQUAL "0483")
        get_filename_component(dev "${v}" DIRECTORY)
        set(serial "?")
        set(product "?")
        if(EXISTS "${dev}/serial")
          file(READ "${dev}/serial" serial)
          string(STRIP "${serial}" serial)
          # an ST-Link V2's serial is 12 raw bytes, not text. The kernel hands them over as UTF-8, so
          # they come out as mojibake and OpenOCD won't match them. It wants the hex, which is also
          # what STM32CubeProgrammer shows
          if(NOT serial MATCHES "^[A-Za-z0-9]*$")
            file(READ "${dev}/serial" bytes HEX)
            probe_serial_hex("${bytes}" serial)
          endif()
        endif()
        if(EXISTS "${dev}/product")
          file(READ "${dev}/product" product)
          string(STRIP "${product}" product)
        endif()
        list(APPEND found "${serial}  ${product}")
      endif()
    endforeach()
  elseif(HOST_OS STREQUAL "macOS")
    execute_process(COMMAND ioreg -p IOUSB -l -w 0 OUTPUT_VARIABLE io)
    string(REGEX MATCHALL "\"idVendor\" = 1155[^+]*" blocks "${io}")
    foreach(b ${blocks})
      string(REGEX MATCH "\"USB Serial Number\" = \"([^\"]*)\"" m "${b}")
      set(serial "${CMAKE_MATCH_1}")
      string(REGEX MATCH "\"USB Product Name\" = \"([^\"]*)\"" m "${b}")
      list(APPEND found "${serial}  ${CMAKE_MATCH_1}")
    endforeach()
  elseif(HOST_OS STREQUAL "Windows")
    execute_process(COMMAND powershell -NoProfile -Command
      "Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'USB\\VID_0483*' } | ForEach-Object { $_.InstanceId.Split('\\')[-1] + '  ' + $_.FriendlyName }"
      OUTPUT_VARIABLE out)
    string(REPLACE "\r" "" out "${out}")
    string(REPLACE "\n" ";" found "${out}")
  endif()
  list(FILTER found EXCLUDE REGEX "^ *$")
  if(found)
    message("ST-Links plugged in (serial, name), flash one with: flash --probe <serial>")
    foreach(f ${found})
      message("  ${f}")
    endforeach()
  else()
    message("no ST-Link found")
  endif()
  return()
endif()

# ---------------------------------------------------------------------------------------------------
# build

# only the parts of the core this chip uses, the rest is every other STM32 family (645 MB, and paths too
# long for Windows)
fetch(STM32-${CORE_VERSION} "${CORE_URL}" ${CORE_SHA}
  "*/cores/arduino/*" "*/libraries/SrcWrapper/*" "*/libraries/Wire/*" "*/libraries/IWatchdog/*" "*/libraries/SPI/*"
  "*/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/*"
  "*/system/Drivers/STM32WLxx_HAL_Driver/*" "*/system/Drivers/CMSIS/Device/ST/STM32WLxx/*"
  "*/system/STM32WLxx/*" "*/system/ldscript.ld" "*/License.md")
fetch(CMSIS-${CMSIS_VERSION} "${CMSIS_URL}" ${CMSIS_SHA})
fetch(xpack-arm-none-eabi-gcc-${GCC_VERSION} "${GCC_URL}/xpack-arm-none-eabi-gcc-${GCC_VERSION}-${TAG}.${EXT}" ${GCC_SHA})
fetch(ninja-${NINJA_VERSION} "${NINJA_URL}/${NINJA_FILE}" ${NINJA_SHA})
set(NINJA "${DEPS}/ninja-${NINJA_VERSION}/ninja${EXE}")
if(NOT HOST_OS STREQUAL "Windows")
  file(CHMOD "${NINJA}" PERMISSIONS OWNER_READ OWNER_WRITE OWNER_EXECUTE GROUP_READ GROUP_EXECUTE WORLD_READ WORLD_EXECUTE)
endif()

if(ACTION STREQUAL "reset")
  # Restarts the board so bench/flash_dump prints its log from the first record. Nothing is built:
  # the console only has to be listened to, and a reset is the one trigger that can't be missed
  fetch(xpack-openocd-${OPENOCD_VERSION} "${OPENOCD_URL}/xpack-openocd-${OPENOCD_VERSION}-${TAG}.${EXT}" ${OPENOCD_SHA})
  set(OPENOCD_DIR "${DEPS}/xpack-openocd-${OPENOCD_VERSION}")
  set(select "")
  if(PROBE)
    set(select -c "adapter serial ${PROBE}")
  endif()
  execute_process(COMMAND "${OPENOCD_DIR}/bin/openocd${EXE}"
                          -s "${OPENOCD_DIR}/openocd/scripts"
                          -f interface/stlink.cfg ${select}
                          -f target/stm32wlx.cfg
                          -c "init; reset run; exit"
                  RESULT_VARIABLE rc)
  if(NOT rc EQUAL 0)
    message(FATAL_ERROR "reset failed. Is the ST-Link plugged in and the board powered?")
  endif()
  message("reset")
  return()
endif()

set(BENCH "${ROOT}/V2_6_X_Tests/bench")
set(SKETCH_INO "")
if(SKETCH_NAME)
  set(SKETCH_INO "${BENCH}/${SKETCH_NAME}/${SKETCH_NAME}.ino")
  if(NOT EXISTS "${SKETCH_INO}")
    file(GLOB found RELATIVE "${BENCH}" "${BENCH}/*")
    list(JOIN found ", " found)
    message(FATAL_ERROR "no bench sketch called ${SKETCH_NAME}. There is: ${found}")
  endif()
endif()

# one build/ for everything: the core objects don't depend on which sketch is on top, so swapping
# between the flight software and a bench sketch only rebuilds the sketch. The stamp says what's in there
set(stamp "")
if(EXISTS "${BUILD}/sketch.txt")
  file(READ "${BUILD}/sketch.txt" stamp)
endif()
set(want "${SKETCH_INO}|${DEFINES}")
if(NOT EXISTS "${BUILD}/build.ninja" OR NOT stamp STREQUAL "${want}")
  execute_process(COMMAND "${CMAKE_COMMAND}" -S "${ROOT}" -B "${BUILD}" -G Ninja
                          "-DCMAKE_MAKE_PROGRAM=${NINJA}"
                          "-DCMAKE_TOOLCHAIN_FILE=${ROOT}/cmake/arm-none-eabi.cmake"
                          "-DCHIPSAT_SKETCH=${SKETCH_INO}"
                          "-DCHIPSAT_DEFINES=${DEFINES}"
                  RESULT_VARIABLE rc)
  if(NOT rc EQUAL 0)
    message(FATAL_ERROR "configure failed")
  endif()
  file(WRITE "${BUILD}/sketch.txt" "${want}")
endif()

if(ACTION STREQUAL "setup")
  fetch(xpack-openocd-${OPENOCD_VERSION} "${OPENOCD_URL}/xpack-openocd-${OPENOCD_VERSION}-${TAG}.${EXT}" ${OPENOCD_SHA})
  message("ready, deps/ has the compiler, ninja and openocd")
  return()
endif()

set(jobs "")
if(DEFINED ENV{CHIPSAT_JOBS})
  set(jobs -j $ENV{CHIPSAT_JOBS})
endif()
execute_process(COMMAND "${CMAKE_COMMAND}" --build "${BUILD}" ${jobs} RESULT_VARIABLE rc)
if(NOT rc EQUAL 0)
  message(FATAL_ERROR "build failed")
endif()
if(SKETCH_NAME)
  message("built ${BUILD}/chipsat.elf, which is the ${SKETCH_NAME} bench sketch, not the flight software")
else()
  check_settings_block("${BUILD}/chipsat.bin")
  message("built ${BUILD}/chipsat.elf, the flight software")
endif()

if(NOT ACTION STREQUAL "flash")
  return()
endif()

# ---------------------------------------------------------------------------------------------------
# flash, with OpenOCD over the ST-Link

fetch(xpack-openocd-${OPENOCD_VERSION} "${OPENOCD_URL}/xpack-openocd-${OPENOCD_VERSION}-${TAG}.${EXT}" ${OPENOCD_SHA})
set(OPENOCD_DIR "${DEPS}/xpack-openocd-${OPENOCD_VERSION}")

set(select "")
if(PROBE)
  set(select -c "adapter serial ${PROBE}")
endif()
execute_process(COMMAND "${OPENOCD_DIR}/bin/openocd${EXE}"
                        -s "${OPENOCD_DIR}/openocd/scripts"
                        -f interface/stlink.cfg ${select}
                        -f target/stm32wlx.cfg
                        -c "program {${BUILD}/chipsat.elf} verify reset exit"
                RESULT_VARIABLE rc)
if(NOT rc EQUAL 0)
  message(FATAL_ERROR "flash failed. Is the ST-Link plugged in and the board powered? 'probes' lists the ST-Links")
endif()
message("flashed and reset")
