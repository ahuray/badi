//! The pinned llama.cpp release that the writing provider verifies before
//! launching it: the executable, its unpacked bundle, and the archive it came from.

pub const RUNTIME_SHA256: &str = "4c20c6b55baa75eafeb02c17f118ce93314ba69aef89a9b4156284d58dcbc0c8";
pub const RUNTIME_BYTES: u64 = 17_896;
pub const RUNTIME_ARCHIVE_FILENAME: &str = "llama-b10726-bin-ubuntu-x64.tar.gz";
pub const RUNTIME_ARCHIVE_SHA256: &str =
    "d3c4e406b2911c8c75d2d0858459645960f8f592c1ab372d565cf145b870c901";
pub const RUNTIME_ARCHIVE_BYTES: u64 = 16_702_536;
pub const RUNTIME_BUNDLE_MANIFEST_SHA256: &str =
    "d1dad3f66d4064b1c2a6d9dc7c824d3d50d2639f3b1d3dd22c7f4355edb99cba";
