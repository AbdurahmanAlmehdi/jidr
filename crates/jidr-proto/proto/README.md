# Vendored emulator protos

Fallback copies of the AOSP emulator's public gRPC definitions (Apache-2.0, © The Android Open Source Project), taken from Android Emulator 35.6.11 (`$ANDROID_SDK_ROOT/emulator/lib/`):

- `emulator_controller.proto`
- `snapshot_service.proto`
- `snapshot.proto`

`build.rs` prefers the protos shipped with the installed SDK emulator. It uses these copies when no SDK is found, or when `JIDR_PROTO_VENDORED=1` is set. Refresh them when the minimum supported emulator version changes.
