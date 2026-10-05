# The compiler build.sh / build.cmd put in deps/. Nothing outside this folder is used

set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR arm)

set(CHIPSAT_GCC_DIR ${CMAKE_CURRENT_LIST_DIR}/../deps/xpack-arm-none-eabi-gcc-14.2.1-1.1/bin)
if(CMAKE_HOST_WIN32)
  set(EXE .exe)
endif()

set(CMAKE_C_COMPILER ${CHIPSAT_GCC_DIR}/arm-none-eabi-gcc${EXE})
set(CMAKE_CXX_COMPILER ${CHIPSAT_GCC_DIR}/arm-none-eabi-g++${EXE})
set(CMAKE_ASM_COMPILER ${CHIPSAT_GCC_DIR}/arm-none-eabi-gcc${EXE})
set(CMAKE_OBJCOPY ${CHIPSAT_GCC_DIR}/arm-none-eabi-objcopy${EXE} CACHE FILEPATH "")
set(CMAKE_SIZE ${CHIPSAT_GCC_DIR}/arm-none-eabi-size${EXE} CACHE FILEPATH "")

# no OS to run a test program on, so only check that the compiler makes a library
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
