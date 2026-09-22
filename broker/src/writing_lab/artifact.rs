//! Explicit Lab artifact selection. Descriptors select only verified weights;
//! they cannot change runtime executables, environment, or inference settings.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::semantic::provenance::FileExpectation;

const MAX_DESCRIPTOR_BYTES: u64 = 16 * 1024;
const MAX_MODEL_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    schema: String,
    weights_path: PathBuf,
    sha256: String,
    bytes: u64,
    alias: String,
}

#[derive(Clone, Debug)]
pub struct ModelArtifactOverride {
    pub(crate) weights: FileExpectation,
    pub(crate) alias: String,
}

fn canonical_regular(path: &Path) -> Result<(), &'static str> {
    let text = path.to_str().ok_or("invalid_model_artifact")?;
    if !path.is_absolute() || text.len() > 4096 || text.chars().any(char::is_control) {
        return Err("invalid_model_artifact");
    }
    let canonical = fs::canonicalize(path).map_err(|_| "invalid_model_artifact")?;
    let metadata = fs::symlink_metadata(path).map_err(|_| "invalid_model_artifact")?;
    if canonical != path || !metadata.file_type().is_file() {
        return Err("invalid_model_artifact");
    }
    Ok(())
}

impl ModelArtifactOverride {
    pub fn read(path: &Path) -> Result<Self, &'static str> {
        canonical_regular(path)?;
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| "invalid_model_artifact")?
            .take(MAX_DESCRIPTOR_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "invalid_model_artifact")?;
        Self::parse(&bytes)
    }

    fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() as u64 > MAX_DESCRIPTOR_BYTES {
            return Err("invalid_model_artifact");
        }
        let descriptor: Descriptor =
            serde_json::from_slice(bytes).map_err(|_| "invalid_model_artifact")?;
        if descriptor.schema != "badi.lab-model-artifact.v1"
            || !(1..=MAX_MODEL_BYTES).contains(&descriptor.bytes)
            || descriptor.alias.is_empty()
            || descriptor.alias.len() > 96
            || !descriptor.alias.as_bytes()[0].is_ascii_alphanumeric()
            || !descriptor
                .alias
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err("invalid_model_artifact");
        }
        let weights =
            FileExpectation::new(descriptor.weights_path, descriptor.sha256, descriptor.bytes)
                .map_err(|_| "invalid_model_artifact")?;
        canonical_regular(weights.path())?;
        Ok(Self {
            weights,
            alias: descriptor.alias,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::provenance::verify_file;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    fn descriptor(path: &Path, bytes: &[u8]) -> Value {
        let mut digest = String::with_capacity(64);
        for byte in Sha256::digest(bytes) {
            write!(digest, "{byte:02x}").unwrap();
        }
        json!({"schema":"badi.lab-model-artifact.v1","weights_path":path,
            "sha256":digest,"bytes":bytes.len(),
            "alias":"disposable-weights.fixture"})
    }

    #[test]
    fn explicit_artifact_requires_complete_bounded_descriptor() {
        let root = tempfile::tempdir().unwrap();
        let weights = root.path().join("weights.gguf");
        fs::write(&weights, b"disposable weights").unwrap();
        let valid = descriptor(&weights, b"disposable weights");
        for field in ["schema", "weights_path", "sha256", "bytes", "alias"] {
            let mut value = valid.clone();
            value.as_object_mut().unwrap().remove(field);
            assert!(ModelArtifactOverride::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for (field, value) in [
            ("runtime", json!("/bin/true")),
            ("bytes", json!(0)),
            ("bytes", json!(MAX_MODEL_BYTES + 1)),
            ("alias", json!("--runtime")),
            ("alias", json!("private\ntext")),
            ("alias", json!("a".repeat(97))),
            ("sha256", json!("A".repeat(64))),
            ("weights_path", json!("relative.gguf")),
        ] {
            let mut changed = valid.clone();
            changed[field] = value;
            assert!(ModelArtifactOverride::parse(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
        let mut oversized = serde_json::to_vec(&valid).unwrap();
        oversized.extend(vec![b' '; usize::try_from(MAX_DESCRIPTOR_BYTES).unwrap()]);
        assert!(ModelArtifactOverride::parse(&oversized).is_err());
        assert!(ModelArtifactOverride::parse(br#"{"schema":"x","schema":"y"}"#).is_err());
    }

    #[test]
    fn explicit_weights_keep_hash_size_identity_and_symlink_reverification() {
        let root = tempfile::tempdir().unwrap();
        let weights = root.path().join("weights.gguf");
        fs::write(&weights, b"first weights").unwrap();
        let value = descriptor(&weights, b"first weights");
        let selected = ModelArtifactOverride::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let verified = verify_file(&selected.weights).unwrap();
        fs::write(&weights, b"other weights").unwrap();
        assert!(verify_file(&selected.weights).is_err());
        assert!(verified.reverify().is_err());
        fs::write(&weights, b"first weights").unwrap();
        let link = root.path().join("linked.gguf");
        std::os::unix::fs::symlink(&weights, &link).unwrap();
        let linked = descriptor(&link, b"first weights");
        assert!(ModelArtifactOverride::parse(&serde_json::to_vec(&linked).unwrap()).is_err());
        let spec = root.path().join("artifact.json");
        fs::write(&spec, serde_json::to_vec(&value).unwrap()).unwrap();
        ModelArtifactOverride::read(&spec).unwrap();
        let spec_link = root.path().join("linked.json");
        std::os::unix::fs::symlink(&spec, &spec_link).unwrap();
        assert!(ModelArtifactOverride::read(&spec_link).is_err());
    }
}
