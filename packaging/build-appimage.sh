#!/usr/bin/env bash
# Builds one self-contained AppImage per game into dist/.
#   packaging/build-appimage.sh            # both games
#   packaging/build-appimage.sh iron_town  # just one
# Needs: cargo, curl, the ALSA and udev headers (libasound2-dev libudev-dev).
# Build on the oldest distro you want to support: the AppImage needs a glibc
# at least as new as the one it was built against.
set -euo pipefail
cd "$(dirname "$0")/.."
GAMES=("${@:-iron_town ember_atoll}")
read -ra GAMES <<< "${GAMES[*]}"
mkdir -p dist target/appimage

TOOL=target/appimage/appimagetool
if [ ! -x "$TOOL" ]; then
  curl -fsSL -o "$TOOL" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$TOOL"
fi

for game in "${GAMES[@]}"; do
  case "$game" in
    iron_town)   title="Iron Town";   name=IronTown;   server=iron_server;  blurb="Retro robot team deathmatch with bots and online play" ;;
    ember_atoll) title="Ember Atoll"; name=EmberAtoll; server=ember_server; blurb="Retro co-op robot shooter on procedural islands" ;;
    *) echo "unknown game: $game" >&2; exit 1 ;;
  esac
  cargo build --release -p "$game" --bins
  dir="target/appimage/$name.AppDir"
  rm -rf "$dir"; mkdir -p "$dir/usr/bin"
  cp "target/release/$game" "target/release/$server" "$dir/usr/bin/"
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
  ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$dir" "dist/$name-x86_64.AppImage"
  echo "built dist/$name-x86_64.AppImage"
done
