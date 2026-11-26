#!/usr/bin/env bash
# Builds one self-contained AppImage per game into dist/.
#   packaging/build-appimage.sh            # both games
#   packaging/build-appimage.sh iron_town  # just one
# Needs: cargo, curl, mksquashfs (squashfs-tools) and the ALSA and udev
# headers (libasound2-dev libudev-dev).
#
# An AppImage is the AppImage runtime (a small static launcher) followed by a
# squashfs image of the app folder; we assemble exactly that.
# Portability: with cargo-zigbuild installed (`pip install ziglang
# cargo-zigbuild`) the games link against glibc 2.31, so the AppImages run on
# any distro from about 2020 on (Ubuntu 20.04, Debian 11, SteamOS). Without
# it they need a glibc as new as the build machine's.
set -euo pipefail
cd "$(dirname "$0")/.."
GAMES=("${@:-iron_town ember_atoll}")
read -ra GAMES <<< "${GAMES[*]}"
mkdir -p dist target/appimage

RUNTIME=target/appimage/runtime-x86_64
if [ ! -s "$RUNTIME" ]; then
  curl -fsSL -o "$RUNTIME" https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64
fi

for game in "${GAMES[@]}"; do
  case "$game" in
    iron_town)   title="Iron Town";   name=IronTown;   server=iron_server;  blurb="Retro robot team deathmatch with bots and online play" ;;
    ember_atoll) title="Ember Atoll"; name=EmberAtoll; server=ember_server; blurb="Retro co-op robot shooter on procedural islands" ;;
    *) echo "unknown game: $game" >&2; exit 1 ;;
  esac
  if command -v cargo-zigbuild >/dev/null; then
    # Only the sonames of ALSA / udev / wayland are linked; the player's own copies load at run time.
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-Wl,--allow-shlib-undefined" \
      cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.31 -p "$game" --bins
    bin=target/x86_64-unknown-linux-gnu/release
  else
    cargo build --release -p "$game" --bins
    bin=target/release
  fi
  dir="target/appimage/$name.AppDir"
  rm -rf "$dir"; mkdir -p "$dir/usr/bin"
  cp "$bin/$game" "$bin/$server" "$dir/usr/bin/"
  strip "$dir/usr/bin/"* || true
  cp "packaging/$game.png" "$dir/$game.png"
  ln -sf "$game.png" "$dir/.DirIcon"
  cat > "$dir/$game.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=$title
Comment=$blurb
Exec=$game
Icon=$game
Categories=Game;ActionGame;
Terminal=false
DESKTOP
  # Runs the game; "AppImage server ..." runs the dedicated server instead.
  cat > "$dir/AppRun" <<APPRUN
#!/bin/sh
here="\$(dirname "\$(readlink -f "\$0")")"
if [ "\$1" = "server" ]; then shift; exec "\$here/usr/bin/$server" "\$@"; fi
exec "\$here/usr/bin/$game" "\$@"
APPRUN
  chmod +x "$dir/AppRun"
  image="target/appimage/$name.squashfs"
  rm -f "$image"
  mksquashfs "$dir" "$image" -root-owned -noappend -comp zstd -quiet
  cat "$RUNTIME" "$image" > "dist/$name-x86_64.AppImage"
  chmod +x "dist/$name-x86_64.AppImage"
  echo "built dist/$name-x86_64.AppImage"
done
