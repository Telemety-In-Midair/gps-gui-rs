# gps-gui-rs

A GPS tracking GUI written in Rust: an interactive slippy map that plots a live
position and the track behind it.

- **GUI**: [egui](https://github.com/emilk/egui) / eframe
- **Map**: [walkers](https://github.com/podusowski/walkers) slippy-map widget
- **Tiles**: OpenStreetMap over HTTP, cached to disk (`.cache/`) so previously
  viewed areas keep rendering offline
- **GPS**: the phone's built-in GNSS (Android LocationManager over JNI) on
  device; a simulated source on desktop. Both feed the same channel, so an
  external BLE ([btleplug](https://github.com/deviceplug/btleplug)) source could
  slot in the same way

## Run (desktop)

```sh
cargo run
```

The map opens on Greenwich and a simulated fix traces a slow loop. Use
**Center on GPS** to follow the position, **Zoom in/out**, and **Clear track**.

## Run (Android)

One crate builds both: the desktop `[[bin]]` and an Android `cdylib` loaded from
a NativeActivity via `android_main` (`src/lib.rs`). The Android build uses the
`wgpu` renderer, reads the phone's GNSS via LocationManager, and caches tiles to
the app's writable data dir for offline reuse.

Prerequisites: Android SDK + NDK, and `rustup target add aarch64-linux-android`
(add `armv7-linux-androideabi` too for 32-bit devices).

```sh
# no-Java flow with xbuild (recommended), arm64-v8a only
cargo install xbuild
x doctor                              # verify SDK/NDK are found
adb devices -l
x run --release --device adb:<serial> # build APK, install, launch

# or with cargo-apk, which is the only route to an armeabi-v7a APK
cargo install cargo-apk
cargo apk run --lib                                 # both ABIs in build_targets
cargo apk run --lib --target armv7-linux-androideabi # just the 32-bit one
```

xbuild's `--arch` takes only `arm64` or `x64`, so 32-bit ARM has to go through
cargo-apk; `build_targets` in Cargo.toml lists both ABIs for it, and `--target`
overrides that list when you want one.

`--lib` is not optional: `cargo apk run` builds exactly one artifact, and this
crate has both a lib and a bin, which it reports as `Error: Invalid args.` The
cdylib is the Android artifact; the `[[bin]]` is desktop-only.

`--release` additionally needs a signing key, or the build fails after
compiling with `Configure a release keystore via
[package.metadata.android.signing.release]`. Either add that section, or point
`CARGO_APK_RELEASE_KEYSTORE` and `CARGO_APK_RELEASE_KEYSTORE_PASSWORD` at one.
Debug builds fall back to the usual `~/.android/debug.keystore`, generated on
first use.

Point `ANDROID_HOME` / `ANDROID_NDK_HOME` at your installs (or let `x doctor`
locate them). Permissions (INTERNET for tiles, LOCATION for GPS) live in two
places that must agree: `manifest.yaml` is what xbuild reads, and Cargo.toml's
`[package.metadata.android]` is what cargo-apk reads. Neither tool warns when
the other's list is short, so edit both.

## Architecture

- `src/gps.rs` - GPS source. Produces `GpsFix` values on a background thread and
  sends them over an `mpsc` channel. Swapping in BLE later means replacing
  `spawn_simulated` with a BLE-backed producer feeding the same channel.
- `src/app.rs` - eframe app: drains the channel each frame, holds the tile
  source, map state, current position, and track.
- `src/marker.rs` - a walkers `Plugin` that draws the track polyline and the
  current-position marker.

## Look sheet

Every size and spacing on the pages is a fraction - of the screen, of the text
height, of the toolbar icon - kept in `gps-gui.look` beside the config rather
than in the code. Nothing in it is a point count. The adjuster (Settings,
"Adjust the look", or `cargo run -- --adjust`) picks a thing on any page and
moves the measures behind it live, then writes the sheet back in place. See
`docs/ui.md` for the format.

## Offline maps

HTTP tiles are cached to `.cache/`, so areas you have already viewed load
without a network. For fully offline use, walkers also supports a local tile
directory (`LocalTiles`) or a bundled `.pmtiles` file.
