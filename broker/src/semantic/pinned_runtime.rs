//! The pinned llama.cpp release that the writing provider verifies before
//! launching it: the executable, its unpacked bundle, and the archive it came from.

/// The release tag, and the directory under `runtimes/` it unpacks to.
pub const RUNTIME_RELEASE: &str = "b11840";
pub const RUNTIME_DIRECTORY: &str = "llama-b11840";
pub const RUNTIME_SHA256: &str = "a929dd0ce172a69aef809b39c0bd8ce61356e3587c71aad75cf292f0d108b33f";
pub const RUNTIME_BYTES: u64 = 17_864;
pub const RUNTIME_ARCHIVE_FILENAME: &str = "llama-b11840-bin-ubuntu-x64.tar.gz";
pub const RUNTIME_ARCHIVE_SHA256: &str =
    "5f6e72e772a1264df279d6b7a848570fdc00e5d968b72e7a917d19e07da96400";
pub const RUNTIME_ARCHIVE_BYTES: u64 = 22_423_573;
pub const RUNTIME_BUNDLE_MANIFEST_SHA256: &str =
    "f59712ac547a063d859b3908c46d17f1354c9b795a5c24be87a6755f87e27961";
