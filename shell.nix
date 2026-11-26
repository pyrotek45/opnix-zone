# `nix-shell` gives you everything needed to build and run on NixOS / Nix.
{ pkgs ? import <nixpkgs> {} }:
let
  libs = with pkgs; [ vulkan-loader alsa-lib udev libxkbcommon wayland libGL
    xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr xorg.libxcb ];
in pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ cargo rustc clippy rustfmt pkg-config gcc ];
  buildInputs = libs;
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath libs + ":/run/opengl-driver/lib";
  RUST_BACKTRACE = "1";
}
