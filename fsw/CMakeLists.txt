# ChipSat flight software, built with the Arduino STM32 core 2.12.0 sources.
# Same compiler flags, sources and link as arduino-cli for Generic WLE5JCIx. Use build.sh / build.cmd,
# they fetch the core, CMSIS and the compiler into deps/ and run this

cmake_minimum_required(VERSION 3.21)
project(chipsat C CXX ASM)

set(CORE ${CMAKE_CURRENT_SOURCE_DIR}/deps/STM32-2.12.0)
set(CMSIS ${CMAKE_CURRENT_SOURCE_DIR}/deps/CMSIS-6.2.0)
set(SKETCH ${CMAKE_CURRENT_SOURCE_DIR}/V2_6_X_Code)
set(VARIANT "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I")

set(CMAKE_EXECUTABLE_SUFFIX .elf)

# from platform.txt and boards.txt, pnum GENERIC_WLE5JCIX
set(CHIP_FLAGS -mcpu=cortex-m4 -mthumb)
set(CHIP_DEFINES
  VECT_TAB_OFFSET=0x0 USE_HAL_DRIVER USE_FULL_LL_DRIVER
  STM32WLxx ARDUINO=10607 ARDUINO_GENERIC_WLE5JCIX ARDUINO_ARCH_STM32
  BOARD_NAME="GENERIC_WLE5JCIX" VARIANT_H="variant_generic.h"
  STM32WLE5xx USE_CM4_STARTUP_FILE HAL_UART_MODULE_ENABLED)

set(COMMON_INCLUDES
  ${SKETCH}
  "${CORE}/cores/arduino/avr"
  "${CORE}/cores/arduino/stm32"
  "${CORE}/libraries/SrcWrapper/inc"
  "${CORE}/libraries/SrcWrapper/inc/LL"
  "${CORE}/system/Drivers/STM32WLxx_HAL_Driver/Inc"
  "${CORE}/system/Drivers/STM32WLxx_HAL_Driver/Src"
  "${CORE}/system/STM32WLxx"
  ${CMSIS}/CMSIS/Core/Include
  "${CORE}/system/Drivers/CMSIS/Device/ST/STM32WLxx/Include"
  "${CORE}/system/Drivers/CMSIS/Device/ST/STM32WLxx/Source/Templates/gcc"
  "${CORE}/cores/arduino"
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I")

add_compile_options(
  ${CHIP_FLAGS}
  "-fmacro-prefix-map=${CORE}=."
  "$<$<COMPILE_LANGUAGE:C,CXX>:-Os;-Wall;-Wextra;-ffunction-sections;-fdata-sections;--param;max-inline-insns-single=500>"
  "$<$<COMPILE_LANGUAGE:C>:-std=gnu17>"
  "$<$<COMPILE_LANGUAGE:CXX>:-std=gnu++17;-fno-threadsafe-statics;-fno-rtti;-fno-exceptions;-fno-use-cxa-atexit>"
  "$<$<COMPILE_LANGUAGE:ASM>:-x;assembler-with-cpp>")
add_compile_definitions(${CHIP_DEFINES} "$<$<COMPILE_LANGUAGE:C,CXX>:NDEBUG>")

# per-board settings from the build command, e.g. ./build.sh flash -DCHIPSAT_ID=12. Passed straight
# through to the compiler; config.h guards every one of them with #ifndef
set(CHIPSAT_DEFINES "" CACHE STRING "extra -D flags for a board's own settings")
if(CHIPSAT_DEFINES)
  add_compile_options(${CHIPSAT_DEFINES})
endif()

# core.a: the core and the variant, linked as a normal archive like arduino-cli does
add_library(arduino_core STATIC
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/PeripheralPins.c"
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/variant_ELV_BM_TRX1.cpp"
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/PeripheralPins_ELV_BM_TRX1.c"
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/variant_NUCLEO_WL55JC1.cpp"
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/variant_generic.cpp"
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/generic_clock.c"
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/variant_LORA_E5_MINI.cpp"
  "${CORE}/variants/STM32WLxx/WL54JCI_WL55JCI_WLE4J(8-B-C)I_WLE5J(8-B-C)I/PeripheralPins_LORA_E5_MINI.c"
  "${CORE}/cores/arduino/HardwareSerial.cpp"
  "${CORE}/cores/arduino/IPAddress.cpp"
  "${CORE}/cores/arduino/WSerial.cpp"
  "${CORE}/cores/arduino/avr/dtostrf.c"
  "${CORE}/cores/arduino/Tone.cpp"
  "${CORE}/cores/arduino/abi.cpp"
  "${CORE}/cores/arduino/RingBuffer.cpp"
  "${CORE}/cores/arduino/WString.cpp"
  "${CORE}/cores/arduino/WInterrupts.cpp"
  "${CORE}/cores/arduino/Print.cpp"
  "${CORE}/cores/arduino/WMath.cpp"
  "${CORE}/cores/arduino/Stream.cpp"
  "${CORE}/cores/arduino/board.c"
  "${CORE}/cores/arduino/core_debug.c"
  "${CORE}/cores/arduino/hooks.c"
  "${CORE}/cores/arduino/itoa.c"
  "${CORE}/cores/arduino/main.cpp"
  "${CORE}/cores/arduino/pins_arduino.c"
  "${CORE}/cores/arduino/stm32/startup_stm32yyxx.S"
  "${CORE}/cores/arduino/wiring_analog.c"
  "${CORE}/cores/arduino/wiring_digital.c"
  "${CORE}/cores/arduino/wiring_pulse.cpp"
  "${CORE}/cores/arduino/wiring_shift.c"
  "${CORE}/cores/arduino/wiring_time.c")
target_include_directories(arduino_core PRIVATE ${COMMON_INCLUDES})

# the flight software. The .ino is plain C++ once Arduino.h is included first, which is all arduino-cli adds
set(CHIPSAT_SKETCH "" CACHE FILEPATH "a bench sketch to build instead of the flight software")
if(CHIPSAT_SKETCH)
  # a bench sketch includes the few .cpp files it needs itself, so globbing src/ would define them twice
  set(SKETCH_INO "${CHIPSAT_SKETCH}")
  set(SKETCH_SOURCES "")
  # only the bench sketches use SPI.h. Its global SPI object has a constructor, so linking it into the
  # flight software costs 324 bytes of flash and 128 of RAM for something never called
  set(SPI_SOURCES "${CORE}/libraries/SPI/src/SPI.cpp" "${CORE}/libraries/SPI/src/utility/spi_com.c")
else()
  set(SPI_SOURCES "")
  set(SKETCH_INO "${SKETCH}/V2_6_X_Code.ino")
  file(GLOB_RECURSE SKETCH_SOURCES CONFIGURE_DEPENDS ${SKETCH}/src/*.cpp ${SKETCH}/src/*.c)
endif()
set_source_files_properties("${SKETCH_INO}" PROPERTIES LANGUAGE CXX COMPILE_OPTIONS "-x;c++;-include;Arduino.h")

add_executable(chipsat
  "${SKETCH_INO}"
  ${SKETCH_SOURCES}
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_comp.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_crc_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_cec.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_cortex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_cordic.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_can.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_adc_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_ccb.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_comp_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_crc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_adc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_cryp.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_cryp_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dac.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dac_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dcache.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dcmi.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dcmi_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dfsdm.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dfsdm_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dma.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dma2d.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dma_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dsi.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_dts.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_eth.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_eth_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_exti.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_fdcan.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_firewall.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_flash.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_flash_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_flash_ramfunc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_fmac.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_fmpi2c.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_fmpi2c_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_fmpsmbus.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_fmpsmbus_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_gfxtim.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_gfxmmu.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_gpio.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_gpio_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_gpu2d.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_gtzc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_hash.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_hash_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_hcd.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_hrtim.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_hsem.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_i2c.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_i2c_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_i2s.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_i2s_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_i3c.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_icache.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_ipcc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_irda.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_iwdg.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_lcd.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_jpeg.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_lpawur.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_lptim.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_ltdc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_ltdc_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_mdf.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_mdios.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_mdma.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_mmc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_mmc_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_mrsubg.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_mrsubg_timer.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_nand.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_nor.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_opamp.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_opamp_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_ospi.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_otfdec.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_pccard.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_pcd.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_pcd_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_pka.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_pssi.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_pwr.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_pwr_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_qspi.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_radio.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_radio_timer.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_ramcfg.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_rcc_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_ramecc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_rng.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_rcc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_rng_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_rtc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_rtc_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_sai.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_sai_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_sd_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_sd.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_sdadc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_sdio.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_sdram.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_smartcard.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_smartcard_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_smbus.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_smbus_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_spdifrx.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_spi.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_spi_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_sram.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_subghz.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_swpmi.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_tim.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_tim_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_tsc.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_uart.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_usart_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_uart_ex.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_usart.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_wwdg.c"
  "${CORE}/libraries/SrcWrapper/src/HAL/stm32yyxx_hal_xspi.c"
  "${CORE}/libraries/SrcWrapper/src/HardwareTimer.cpp"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_adc.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_bdma.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_comp.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_cordic.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_crc.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_crs.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_dac.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_delayblock.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_dlyb.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_dma.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_dma2d.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_exti.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_fmac.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_fmc.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_fmpi2c.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_fsmc.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_gpio.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_hrtim.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_i2c.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_i3c.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_icache.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_lcsc.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_lpgpio.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_lptim.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_lpuart.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_opamp.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_mdma.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_pka.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_pwr.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_rcc.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_rng.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_rtc.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_sdmmc.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_spi.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_swpmi.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_system.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_tim.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_ucpd.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_usart.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_usb.c"
  "${CORE}/libraries/SrcWrapper/src/LL/stm32yyxx_ll_utils.c"
  "${CORE}/libraries/SrcWrapper/src/new.cpp"
  "${CORE}/libraries/SrcWrapper/src/stm32/PortNames.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/analog.cpp"
  "${CORE}/libraries/SrcWrapper/src/stm32/bootloader.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/clock.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/core_callback.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/dwt.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/hw_config.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/interrupt.cpp"
  "${CORE}/libraries/SrcWrapper/src/stm32/otp.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/pinmap.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/stm32_def.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/system_stm32yyxx.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/timer.c"
  "${CORE}/libraries/SrcWrapper/src/syscalls.c"
  "${CORE}/libraries/SrcWrapper/src/stm32/uart.c"
  "${CORE}/libraries/Wire/src/utility/twi.c"
  "${CORE}/libraries/Wire/src/Wire.cpp"
  "${CORE}/libraries/IWatchdog/src/IWatchdog.cpp"
  ${SPI_SOURCES})
target_include_directories(chipsat PRIVATE ${COMMON_INCLUDES}
  "${CORE}/libraries/SrcWrapper/src"
  "${CORE}/libraries/Wire/src"
  "${CORE}/libraries/IWatchdog/src"
  "${CORE}/libraries/SPI/src")

target_link_options(chipsat PRIVATE
  ${CHIP_FLAGS} -Os --specs=nano.specs
  -Wl,--defsym=LD_FLASH_OFFSET=0x0 -Wl,--defsym=LD_MAX_SIZE=262144 -Wl,--defsym=LD_MAX_DATA_SIZE=65536
  -Wl,--cref -Wl,--check-sections -Wl,--gc-sections -Wl,--entry=Reset_Handler
  -Wl,--unresolved-symbols=report-all -Wl,--warn-common
  "-Wl,--default-script=${VARIANT}/ldscript.ld" "-Wl,--script=${CORE}/system/ldscript.ld"
  "-Wl,-Map,${CMAKE_CURRENT_BINARY_DIR}/chipsat.map" -Wl,--no-warn-rwx-segments)
target_link_libraries(chipsat PRIVATE -Wl,--start-group arduino_core c -Wl,--end-group m gcc stdc++)

add_custom_command(TARGET chipsat POST_BUILD
  COMMAND ${CMAKE_OBJCOPY} -O binary $<TARGET_FILE:chipsat> ${CMAKE_CURRENT_BINARY_DIR}/chipsat.bin
  COMMAND ${CMAKE_OBJCOPY} -O ihex $<TARGET_FILE:chipsat> ${CMAKE_CURRENT_BINARY_DIR}/chipsat.hex
  COMMAND ${CMAKE_SIZE} $<TARGET_FILE:chipsat>)
