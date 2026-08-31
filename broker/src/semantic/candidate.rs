use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::model_selection::{ModelTier, ModelUseCase, catalog};

use super::provenance::{
    DirectoryManifestExpectation, FileExpectation, ProvenanceError, VerifiedDirectoryManifest,
    VerifiedFile, verify_directory_manifest, verify_file,
};
use super::runtime::{LlamaCppLaunch, RuntimeError};

pub const MODEL_FILENAME: &str = "Qwen3-1.7B-Q4_K_M.gguf";
pub const MODEL_SHA256: &str = "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5";
pub const MODEL_BYTES: u64 = 1_282_439_264;
pub const MODEL_QUANTIZATION: &str = "Q4_K_M";
pub const MODEL_QUANTIZER_REPOSITORY: &str = "ggml-org/Qwen3-1.7B-GGUF";
pub const MODEL_QUANTIZER_REVISION: &str = "daeb8e2d528a760970442092f6bf1e55c3b659eb";
pub const MODEL_UPSTREAM_REPOSITORY: &str = "Qwen/Qwen3-1.7B";
pub const MODEL_LICENSE: &str = "Apache-2.0";

pub const RUNTIME_FILENAME: &str = "llama-server";
pub const RUNTIME_SHA256: &str = "4c20c6b55baa75eafeb02c17f118ce93314ba69aef89a9b4156284d58dcbc0c8";
pub const RUNTIME_BYTES: u64 = 17_896;
pub const RUNTIME_ARCHIVE_FILENAME: &str = "llama-b10726-bin-ubuntu-x64.tar.gz";
pub const RUNTIME_ARCHIVE_SHA256: &str =
    "d3c4e406b2911c8c75d2d0858459645960f8f592c1ab372d565cf145b870c901";
pub const RUNTIME_ARCHIVE_BYTES: u64 = 16_702_536;
pub const RUNTIME_BUNDLE_MANIFEST_SHA256: &str =
    "d1dad3f66d4064b1c2a6d9dc7c824d3d50d2639f3b1d3dd22c7f4355edb99cba";
pub const RUNTIME_REPOSITORY: &str = "ggml-org/llama.cpp";
pub const RUNTIME_REVISION: &str = "85c55223caf0a2ad0d1d88e5a73ab3fe36107867";
pub const RUNTIME_VERSION: &str = "b10726";
pub const RUNTIME_DYNAMIC_BUNDLE_CONSTRAINT: &str =
    "exact-directory-manifest-pinned; system-dsos-platform-dependencies";

pub const MODEL_ALIAS: &str = "qwen3-1.7b-q4-k-m";
pub const THREADS: usize = 18;

#[derive(Clone, Eq, PartialEq)]
pub struct PinnedCandidatePaths {
    model: PathBuf,
    runtime: PathBuf,
    runtime_archive: PathBuf,
}

impl PinnedCandidatePaths {
    #[must_use]
    pub fn new(
        model: impl Into<PathBuf>,
        runtime: impl Into<PathBuf>,
        runtime_archive: impl Into<PathBuf>,
    ) -> Self {
        Self {
            model: model.into(),
            runtime: runtime.into(),
            runtime_archive: runtime_archive.into(),
        }
    }

    #[must_use]
    pub fn model(&self) -> &Path {
        &self.model
    }

    #[must_use]
    pub fn runtime(&self) -> &Path {
        &self.runtime
    }

    #[must_use]
    pub fn runtime_archive(&self) -> &Path {
        &self.runtime_archive
    }
}

pub struct VerifiedPinnedCandidate {
    model: VerifiedFile,
    runtime: VerifiedFile,
    runtime_bundle: VerifiedDirectoryManifest,
    runtime_archive: VerifiedFile,
}

impl VerifiedPinnedCandidate {
    pub fn verify(paths: &PinnedCandidatePaths) -> Result<Self, CandidateError> {
        verify_catalog_identity()?;
        verify_filename(paths.model(), MODEL_FILENAME, "model_filename")?;
        verify_filename(paths.runtime(), RUNTIME_FILENAME, "runtime_filename")?;
        verify_filename(
            paths.runtime_archive(),
            RUNTIME_ARCHIVE_FILENAME,
            "runtime_archive_filename",
        )?;
        let runtime_bundle_path = paths
            .runtime()
            .parent()
            .ok_or(CandidateError::Filename("runtime_bundle_parent"))?;
        let runtime_bundle = verify_directory_manifest(&DirectoryManifestExpectation::new(
            runtime_bundle_path,
            RUNTIME_BUNDLE_MANIFEST_SHA256,
        )?)?;
        let runtime = verify_file(&FileExpectation::new(
            paths.runtime(),
            RUNTIME_SHA256,
            RUNTIME_BYTES,
        )?)?;
        if runtime.path().parent() != Some(runtime_bundle.path()) {
            return Err(CandidateError::Filename("runtime_bundle_parent"));
        }
        let runtime_archive = verify_file(&FileExpectation::new(
            paths.runtime_archive(),
            RUNTIME_ARCHIVE_SHA256,
            RUNTIME_ARCHIVE_BYTES,
        )?)?;
        let model = verify_file(&FileExpectation::new(
            paths.model(),
            MODEL_SHA256,
            MODEL_BYTES,
        )?)?;
        Ok(Self {
            model,
            runtime,
            runtime_bundle,
            runtime_archive,
        })
    }

    pub fn launch(&self) -> Result<LlamaCppLaunch, RuntimeError> {
        LlamaCppLaunch::new(
            self.runtime.clone(),
            self.runtime_bundle.clone(),
            self.model.clone(),
            MODEL_ALIAS,
            THREADS,
        )
    }

    pub fn reverify(&self) -> Result<(), ProvenanceError> {
        self.runtime_bundle.reverify()?;
        self.runtime.reverify()?;
        self.runtime_archive.reverify()?;
        self.model.reverify()
    }

    #[must_use]
    pub const fn model(&self) -> &VerifiedFile {
        &self.model
    }

    #[must_use]
    pub const fn runtime(&self) -> &VerifiedFile {
        &self.runtime
    }

    #[must_use]
    pub const fn runtime_bundle(&self) -> &VerifiedDirectoryManifest {
        &self.runtime_bundle
    }

    #[must_use]
    pub const fn runtime_archive(&self) -> &VerifiedFile {
        &self.runtime_archive
    }
}

#[derive(Debug, Error)]
pub enum CandidateError {
    #[error("pinned semantic catalog identity mismatch: {0}")]
    CatalogMismatch(&'static str),
    #[error("pinned semantic path has the wrong leaf name: {0}")]
    Filename(&'static str),
    #[error("pinned semantic file provenance failed")]
    Provenance(#[from] ProvenanceError),
}

fn verify_catalog_identity() -> Result<(), CandidateError> {
    let artifact = catalog(ModelUseCase::Writing)
        .iter()
        .find(|artifact| artifact.tier == ModelTier::Balanced)
        .ok_or(CandidateError::CatalogMismatch("missing_balanced_writing"))?;
    for (matches, field) in [
        (
            artifact.repository == MODEL_QUANTIZER_REPOSITORY,
            "repository",
        ),
        (artifact.revision == MODEL_QUANTIZER_REVISION, "revision"),
        (artifact.filename == MODEL_FILENAME, "filename"),
        (artifact.sha256 == MODEL_SHA256, "sha256"),
        (artifact.download_bytes == MODEL_BYTES, "download_bytes"),
        (artifact.quantization == MODEL_QUANTIZATION, "quantization"),
        (artifact.license == MODEL_LICENSE, "license"),
        (artifact.runtime == "llama.cpp", "runtime"),
        (
            artifact.prompt_format == "llama_cpp_native_prefix_completion_v1",
            "prompt_format",
        ),
    ] {
        if !matches {
            return Err(CandidateError::CatalogMismatch(field));
        }
    }
    Ok(())
}

fn verify_filename(path: &Path, expected: &str, field: &'static str) -> Result<(), CandidateError> {
    if path.file_name() == Some(OsStr::new(expected)) {
        Ok(())
    } else {
        Err(CandidateError::Filename(field))
    }
}
