@echo off
rem Builds the flight software, fetching the compiler first if it is missing. build.cmd help for the rest
cmake -P "%~dp0cmake\chipsat.cmake" %*
