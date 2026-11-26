#!/usr/bin/env bash
# Cross-compiles the Windows builds from Linux into dist/:
# IronTown.exe, EmberAtoll.exe and a zip per game with its dedicated server.
# Needs: rustup target add x86_64-pc-windows-gnu, and the MinGW-w64 compiler
# (apt install gcc-mingw-w64-x86-64). On Windows itself, plain
# `cargo build --release -p iron_town` gives the same exe.
set -euo pipefail
cd "$(dirname "$0")/.."
TARGET=x86_64-pc-windows-gnu
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
mkdir -p dist
for pair in iron_town:IronTown:iron_server ember_atoll:EmberAtoll:ember_server; do
  IFS=: read -r game name server <<< "$pair"
  cargo build --release --target $TARGET -p "$game" --bins
  out="target/$TARGET/release"
  x86_64-w64-mingw32-strip "$out/$game.exe" "$out/$server.exe" || true
  cp "$out/$game.exe" "dist/$name.exe"
  stage="target/windows-zip/$name"
  rm -rf "$stage"; mkdir -p "$stage"
  cp "$out/$game.exe" "$stage/$name.exe"
  cp "$out/$server.exe" "$stage/$name-Server.exe"
  (cd target/windows-zip && rm -f "../../dist/$name-windows.zip" && zip -qr "../../dist/$name-windows.zip" "$name")
  echo "built dist/$name.exe and dist/$name-windows.zip"
done
