//! gRPC bindings for the AOSP emulator's `EmulatorController` and
//! `SnapshotService`, generated at build time from the installed SDK's protos
//! (or the vendored copies in `proto/`).

// The generated code addresses sibling packages by relative path
// (`super::super::super::emulator_snapshot`), so the module tree mirrors the
// proto packages.
#[allow(clippy::all, clippy::pedantic, non_camel_case_types, missing_docs)]
pub mod android {
    pub mod emulation {
        /// `android.emulation.control`: EmulatorController + SnapshotService.
        pub mod control {
            tonic::include_proto!("android.emulation.control");
        }
    }
}

/// `emulator_snapshot`: snapshot metadata used by SnapshotService.
#[allow(clippy::all, clippy::pedantic, non_camel_case_types, missing_docs)]
pub mod emulator_snapshot {
    tonic::include_proto!("emulator_snapshot");
}

pub use android::emulation::control;

pub use control::emulator_controller_client::EmulatorControllerClient;
pub use control::snapshot_service_client::SnapshotServiceClient;

/// Where the protos came from at build time: `"sdk"` or `"vendored"`.
pub const PROTO_SOURCE: &str = env!("JIDR_PROTO_SOURCE");

/// Adds `authorization: Bearer <token>` (emulator `-grpc-use-token`).
#[derive(Debug, Clone)]
pub struct BearerToken(tonic::metadata::MetadataValue<tonic::metadata::Ascii>);

impl BearerToken {
    pub fn new(token: &str) -> Result<Self, tonic::metadata::errors::InvalidMetadataValue> {
        Ok(Self(format!("Bearer {token}").parse()?))
    }
}

impl tonic::service::Interceptor for BearerToken {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        req.metadata_mut().insert("authorization", self.0.clone());
        Ok(req)
    }
}

pub type AuthChannel =
    tonic::service::interceptor::InterceptedService<tonic::transport::Channel, BearerToken>;
pub type Controller = EmulatorControllerClient<AuthChannel>;
pub type Snapshots = SnapshotServiceClient<AuthChannel>;

/// Largest gRPC message we accept: a full-resolution RGBA frame of a large
/// tablet is ~25 MB (ADR-0001).
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// Authenticated clients for one emulator.
pub fn clients(channel: tonic::transport::Channel, token: BearerToken) -> (Controller, Snapshots) {
    (
        EmulatorControllerClient::with_interceptor(channel.clone(), token.clone())
            .max_decoding_message_size(MAX_MESSAGE),
        SnapshotServiceClient::with_interceptor(channel, token)
            .max_decoding_message_size(MAX_MESSAGE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn source_is_reported() {
        assert!(PROTO_SOURCE == "sdk" || PROTO_SOURCE == "vendored");
    }

    #[test]
    fn bearer_token_sets_header() {
        use tonic::service::Interceptor;
        let mut t = BearerToken::new("abc").unwrap();
        let req = t.call(tonic::Request::new(())).unwrap();
        assert_eq!(req.metadata().get("authorization").unwrap(), "Bearer abc");
        assert!(BearerToken::new("bad\ntoken").is_err());
    }

    /// The vendored copies compile on their own (the no-SDK build path),
    /// whichever source the crate itself was built from.
    #[test]
    fn vendored_protos_compile() {
        let vendored = Path::new(env!("CARGO_MANIFEST_DIR")).join("proto");
        let out = tempfile::tempdir().unwrap();
        tonic_prost_build::configure()
            .build_server(true)
            .out_dir(out.path())
            .compile_protos(
                &[
                    vendored.join("emulator_controller.proto"),
                    vendored.join("snapshot_service.proto"),
                ],
                &[vendored],
            )
            .unwrap();
        let generated =
            std::fs::read_to_string(out.path().join("android.emulation.control.rs")).unwrap();
        for rpc in [
            "stream_screenshot",
            "stream_input_event",
            "set_battery",
            "save_snapshot",
            "load_snapshot",
        ] {
            assert!(generated.contains(&format!("fn {rpc}")), "missing {rpc}");
        }
    }

    /// The vendored copies must still declare every RPC Jidr calls, so an SDK
    /// build and a vendored build expose the same API surface.
    #[test]
    fn sdk_protos_keep_our_rpcs() {
        let dir = Path::new(env!("JIDR_PROTO_DIR"));
        let text = std::fs::read_to_string(dir.join("emulator_controller.proto")).unwrap();
        for rpc in [
            "streamScreenshot",
            "getScreenshot",
            "streamInputEvent",
            "sendKey",
            "sendTouch",
            "setBattery",
            "setGps",
            "setPhysicalModel",
            "setSensor",
            "sendPhone",
            "sendSms",
            "getStatus",
            "setClipboard",
            "getClipboard",
            "streamLogcat",
            "sendFingerprint",
        ] {
            assert!(
                text.contains(&format!("rpc {rpc}(")),
                "{rpc} missing from {}",
                dir.display()
            );
        }
    }
}
