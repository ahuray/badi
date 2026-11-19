//! Source identity embedded by `build.rs`. `unknown` means the build had no
//! usable Git metadata, for example in an exported source archive.

/// Full commit of the Rust inputs, or `unknown`.
pub const COMMIT: &str = env!("BADI_BUILD_COMMIT");
/// Whether the Rust inputs differed from `COMMIT`: `true`, `false` or `unknown`.
pub const DIRTY: &str = env!("BADI_BUILD_DIRTY");

/// The single `--version` line shared by the installed binaries.
#[must_use]
pub fn version_line(binary: &str) -> String {
    format!(
        "{binary} {} commit={COMMIT} dirty={DIRTY}",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests;
