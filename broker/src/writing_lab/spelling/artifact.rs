use crate::semantic::provenance::{FileExpectation, VerifiedFile, verify_file};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const ENGINE_SHA256: &str = "32b16f4c3ffbf8fab764e02c1d3e99d2453db5fb6e5c57a1f05b6552523a1faf";
const LIBRARY_SHA256: &str = "c45ccf092f13c6007813940acc025cd2d5f5c9182a30c746168065ab69322e67";
const ERROR: &str = "invalid_spelling_artifact";

pub(super) fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut value = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(value, "{byte:02x}").expect("string write");
    }
    value
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSpec {
    path: PathBuf,
    sha256: String,
    bytes: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dictionary {
    id: String,
    aff: FileSpec,
    dic: FileSpec,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dictionaries {
    de: Dictionary,
    fa: Dictionary,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    binary: FileSpec,
    library: FileSpec,
    dictionaries: Dictionaries,
}

#[derive(Clone, Debug, Serialize)]
pub struct Identity {
    pub contract_id: &'static str,
    pub manifest_sha256: String,
    pub engine_sha256: String,
    pub engine_size: u64,
    pub library_sha256: String,
    pub library_size: u64,
    pub language: String,
    pub dictionary_id: String,
    pub aff_sha256: String,
    pub aff_size: u64,
    pub dic_sha256: String,
    pub dic_size: u64,
}
impl Identity {
    #[must_use]
    pub fn sha256(&self) -> String {
        digest(&serde_json::to_vec(self).expect("identity serialization"))
    }
}

pub struct Artifact {
    pub(super) manifest_path: PathBuf,
    pub(super) manifest: VerifiedFile,
    pub(super) binary: VerifiedFile,
    pub(super) library: VerifiedFile,
    pub(super) aff: VerifiedFile,
    pub(super) dic: VerifiedFile,
    pub(super) identity: Identity,
}

fn regular(path: &Path) -> Result<(), &'static str> {
    let value = path.to_str().ok_or(ERROR)?;
    if !path.is_absolute()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || fs::canonicalize(path).map_err(|_| ERROR)? != path
        || !fs::symlink_metadata(path).map_err(|_| ERROR)?.is_file()
    {
        return Err(ERROR);
    }
    Ok(())
}
fn expectation(spec: &FileSpec) -> Result<FileExpectation, &'static str> {
    if !(1..=64 * 1024 * 1024).contains(&spec.bytes) {
        return Err(ERROR);
    }
    regular(&spec.path)?;
    FileExpectation::new(&spec.path, &spec.sha256, spec.bytes).map_err(|_| ERROR)
}
fn verified(spec: &FileSpec) -> Result<VerifiedFile, &'static str> {
    verify_file(&expectation(spec)?).map_err(|_| ERROR)
}
fn validate_dictionary(dictionary: &Dictionary) -> Result<(), &'static str> {
    if dictionary.id.is_empty()
        || dictionary.id.len() > 96
        || !dictionary.id.as_bytes()[0].is_ascii_alphanumeric()
        || !dictionary.id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        || dictionary.aff.path.extension().is_none_or(|ext| ext != "aff")
        || dictionary.dic.path.extension().is_none_or(|ext| ext != "dic")
        || dictionary.aff.path.with_extension("dic") != dictionary.dic.path
        // Hunspell interprets commas inside -d as a list of dictionaries.
        || dictionary.aff.path.to_str().is_none_or(|path| path.contains(','))
    {
        return Err(ERROR);
    }
    expectation(&dictionary.aff)?;
    expectation(&dictionary.dic)?;
    Ok(())
}

impl Artifact {
    #[cfg(test)]
    pub(super) fn fixture(path: &Path) -> Self {
        fs::write(path, b"fixture").unwrap();
        let file =
            verify_file(&FileExpectation::new(path, digest(b"fixture"), 7).unwrap()).unwrap();
        Self {
            manifest_path: path.to_owned(),
            manifest: file.clone(),
            binary: file.clone(),
            library: file.clone(),
            aff: file.clone(),
            dic: file,
            identity: Identity {
                contract_id: "fixture",
                manifest_sha256: digest(b"fixture"),
                engine_sha256: digest(b"fixture"),
                engine_size: 7,
                library_sha256: digest(b"fixture"),
                library_size: 7,
                language: "de".to_owned(),
                dictionary_id: "fixture".to_owned(),
                aff_sha256: digest(b"fixture"),
                aff_size: 7,
                dic_sha256: digest(b"fixture"),
                dic_size: 7,
            },
        }
    }
    pub fn read(path: &Path, language: &str) -> Result<Self, &'static str> {
        regular(path)?;
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| ERROR)?
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ERROR)?;
        if bytes.len() > 16 * 1024 {
            return Err(ERROR);
        }
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
        if manifest.schema != "badi.spelling-artifact.v1"
            || manifest.binary.path != Path::new("/usr/bin/hunspell")
            || manifest.binary.sha256 != ENGINE_SHA256
            || manifest.library.sha256 != LIBRARY_SHA256
        {
            return Err(ERROR);
        }
        validate_dictionary(&manifest.dictionaries.de)?;
        validate_dictionary(&manifest.dictionaries.fa)?;
        let selected = match language {
            "de" => manifest.dictionaries.de,
            "fa" => manifest.dictionaries.fa,
            _ => return Err(ERROR),
        };
        let manifest_file = verified(&FileSpec {
            path: path.to_owned(),
            sha256: digest(&bytes),
            bytes: bytes.len() as u64,
        })?;
        let binary = verified(&manifest.binary)?;
        let library = verified(&manifest.library)?;
        let aff = verified(&selected.aff)?;
        let dic = verified(&selected.dic)?;
        let aff_text = fs::read_to_string(aff.path()).map_err(|_| ERROR)?;
        if aff_text
            .lines()
            .filter(|line| line.starts_with("SET "))
            .collect::<Vec<_>>()
            != ["SET UTF-8"]
        {
            return Err(ERROR);
        }
        let identity = Identity {
            contract_id: "badi.spelling-lab.hunspell.v1",
            manifest_sha256: manifest_file.sha256().to_owned(),
            engine_sha256: binary.sha256().to_owned(),
            engine_size: binary.identity().size,
            library_sha256: library.sha256().to_owned(),
            library_size: library.identity().size,
            language: language.to_owned(),
            dictionary_id: selected.id,
            aff_sha256: aff.sha256().to_owned(),
            aff_size: aff.identity().size,
            dic_sha256: dic.sha256().to_owned(),
            dic_size: dic.identity().size,
        };
        Ok(Self {
            manifest_path: path.to_owned(),
            manifest: manifest_file,
            binary,
            library,
            aff,
            dic,
            identity,
        })
    }
    pub(super) fn reverify(&self) -> Result<(), &'static str> {
        for file in [
            &self.manifest,
            &self.binary,
            &self.library,
            &self.aff,
            &self.dic,
        ] {
            file.reverify().map_err(|_| "spelling_artifact_changed")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_files_require_exact_digest_size_canonical_path_and_stable_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        fs::write(&path, b"original").unwrap();
        let spec = || FileSpec {
            path: path.clone(),
            sha256: digest(b"original"),
            bytes: 8,
        };
        let file = verified(&spec()).unwrap();
        fs::write(&path, b"modified").unwrap();
        assert!(file.reverify().is_err());
        assert!(verified(&spec()).is_err());
        fs::write(&path, b"original").unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(
            verified(&FileSpec {
                path: link,
                ..spec()
            })
            .is_err()
        );
        assert!(verified(&FileSpec { bytes: 0, ..spec() }).is_err());
        assert!(
            verified(&FileSpec {
                sha256: "A".repeat(64),
                ..spec()
            })
            .is_err()
        );
    }
    #[test]
    fn dictionary_specs_bind_one_canonical_pair_even_when_not_selected() {
        let directory = tempfile::tempdir().unwrap();
        let dictionary = |stem: &str| {
            let file = |extension: &str| {
                let path = directory.path().join(stem).with_extension(extension);
                fs::write(&path, b"fixture").unwrap();
                FileSpec {
                    path,
                    sha256: digest(b"fixture"),
                    bytes: 7,
                }
            };
            Dictionary {
                id: "fixture".to_owned(),
                aff: file("aff"),
                dic: file("dic"),
            }
        };
        validate_dictionary(&dictionary("valid")).unwrap();
        assert!(validate_dictionary(&dictionary("first,second")).is_err());
        let mut invalid = dictionary("valid");
        invalid.dic.bytes = 0;
        assert!(validate_dictionary(&invalid).is_err());
        let mut invalid = dictionary("valid");
        invalid.dic.sha256 = "A".repeat(64);
        assert!(validate_dictionary(&invalid).is_err());
        let mut invalid = dictionary("valid");
        invalid.dic.path = directory.path().join("other.dic");
        assert!(validate_dictionary(&invalid).is_err());
    }
    #[test]
    fn manifest_fields_are_strict_and_do_not_expose_executable_flags() {
        assert!(serde_json::from_str::<Manifest>("{\"schema\":\"x\",\"schema\":\"y\"}").is_err());
        let file = serde_json::json!({"path":"/absolute/file","sha256":"a".repeat(64),"bytes":1});
        let pair = serde_json::json!({"id":"fixture","aff":file,"dic":file});
        let mut value = serde_json::json!({"schema":"badi.spelling-artifact.v1","binary":file,"library":file,"dictionaries":{"de":pair,"fa":pair}});
        assert!(serde_json::from_value::<Manifest>(value.clone()).is_ok());
        value["args"] = serde_json::json!(["--unsafe"]);
        assert!(serde_json::from_value::<Manifest>(value).is_err());
    }
}
