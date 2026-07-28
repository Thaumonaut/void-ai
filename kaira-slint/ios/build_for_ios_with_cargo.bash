#!/usr/bin/env bash
# Slint's official iOS build glue (from slint-ui/slint scripts/), invoked by Xcode as
# a postCompile Run Script. It cargo-builds the `$1` bin for the device/simulator
# arch, lipos it into the .app, writes the dSYM, and re-signs. Run from the project
# dir (where Cargo.toml lives) — see project.yml `postCompileScripts`.
set -euvx
export PATH="/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH:$HOME/.cargo/bin"

# libopus-ios was built with -mios-version-min=16.0; keep the link target in step so
# ld doesn't fail on the version mismatch. (Xcode sets this from deploymentTarget too.)
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-16.0}"

if [[ "$CONFIGURATION" != "Debug" ]]; then
    CARGO_PROFILE=release; MAYBE_RELEASE=--release
else
    CARGO_PROFILE=debug;   MAYBE_RELEASE=
fi
export CARGO_PROFILE_RELEASE_DEBUG="${CARGO_PROFILE_RELEASE_DEBUG:-1}"
# Reuse the crate's shared target/ dir (same as `cargo build` from the CLI) instead of a
# throwaway per-build dir under DerivedData. Otherwise Xcode recompiles the entire tree
# (webrtc + skia = ~15 GB) from scratch every clean build — slow, and it filled the disk.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SRCROOT/target}"

# Bake build-time tokens from the gitignored ../.env.local into the build (realtime.rs + ios_map.rs
# read them via option_env!): the backend auth token + the publishable Mapbox token. Absent (fresh
# clone) → empty → point that build at your own backend / set your own tokens.
if [ -f "$SRCROOT/../.env.local" ]; then
    for _v in BOT_AUTH_TOKEN MAPBOX_PUBLIC_TOKEN; do
        _val="$(grep -E "^$_v=" "$SRCROOT/../.env.local" | head -1 | cut -d= -f2-)"
        [ -n "$_val" ] && export "$_v=$_val"
    done
fi

IS_SIMULATOR=0
if [ "${LLVM_TARGET_TRIPLE_SUFFIX-}" = "-simulator" ]; then IS_SIMULATOR=1; fi

executables=()
for arch in $ARCHS; do
    case "$arch" in
        arm64)
            if [ $IS_SIMULATOR -eq 0 ]; then CARGO_TARGET=aarch64-apple-ios
            else CARGO_TARGET=aarch64-apple-ios-sim; fi ;;
        x86_64)
            export CFLAGS_x86_64_apple_ios="-target x86_64-apple-ios"
            CARGO_TARGET=x86_64-apple-ios ;;
    esac
    cargo build $MAYBE_RELEASE --target $CARGO_TARGET --bin "$1" "${@:2}"
    executables+=("$CARGO_TARGET_DIR/$CARGO_TARGET/$CARGO_PROFILE/$1")
done

lipo -create -output "$TARGET_BUILD_DIR/$EXECUTABLE_PATH" "${executables[@]}"

if [ -n "${DWARF_DSYM_FOLDER_PATH:-}" ] && [ -n "${DWARF_DSYM_FILE_NAME:-}" ]; then
    mkdir -p "$DWARF_DSYM_FOLDER_PATH"
    dsymutil "$TARGET_BUILD_DIR/$EXECUTABLE_PATH" -o "$DWARF_DSYM_FOLDER_PATH/$DWARF_DSYM_FILE_NAME"
fi

if [ $IS_SIMULATOR -eq 0 ] && [ "${CODE_SIGNING_ALLOWED:-YES}" != "NO" ] && [ -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]; then
    ENTITLEMENTS_FILE="${TARGET_TEMP_DIR}/${PRODUCT_NAME}.app.xcent"
    if [ -s "$ENTITLEMENTS_FILE" ]; then
        codesign --force --sign "${EXPANDED_CODE_SIGN_IDENTITY}" --entitlements "$ENTITLEMENTS_FILE" "${TARGET_BUILD_DIR}/${EXECUTABLE_PATH}"
    else
        codesign --force --sign "${EXPANDED_CODE_SIGN_IDENTITY}" "${TARGET_BUILD_DIR}/${EXECUTABLE_PATH}"
    fi
fi
