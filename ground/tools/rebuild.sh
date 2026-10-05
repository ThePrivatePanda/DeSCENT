#!/bin/sh
# Rebuild both binaries in app/target so the paths people actually launch are current.
# Called by the post-commit and post-merge hooks; safe to run by hand.
#
# include_dir! reads web/ when the macro expands, so a build carries a copy of the
# dashboard inside it. A binary left from an earlier commit serves an older UI and
# nothing on screen says so, which is why this exists.
set -e
cd "$(dirname "$0")/../app"

# DG_BUILD_WRAP is a command to run the build under, for a machine that queues its
# heavy jobs. Without it the build is only niced.
build() {
  ${DG_BUILD_WRAP:-nice -n 19} cargo build --release -j2 "$@"
}

build
if rustup target list --installed 2>/dev/null | grep -q x86_64-unknown-linux-musl; then
  build --target x86_64-unknown-linux-musl
fi
