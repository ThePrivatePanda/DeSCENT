#!/bin/sh
# Builds the flight software, fetching the compiler first if it's missing. ./build.sh help for the rest
exec cmake -P "$(dirname "$0")/cmake/chipsat.cmake" "$@"
