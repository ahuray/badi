use crate::semantic::provenance::{FileIdentity, VerifiedFile};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;

const ERROR: &str = "spelling_identity_unverified";
const MAX_MAPS_BYTES: usize = 256 * 1024;

fn matches_file(metadata: &std::fs::Metadata, expected: FileIdentity) -> bool {
    metadata.is_file()
        && metadata.dev() == expected.device
        && metadata.ino() == expected.inode
        && metadata.len() == expected.size
}

fn verify_open_file(mut file: File, expected: &VerifiedFile) -> Result<(), &'static str> {
    if !matches_file(&file.metadata().map_err(|_| ERROR)?, expected.identity()) {
        return Err(ERROR);
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(expected.identity().size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 != expected.identity().size
        || super::artifact::digest(&bytes) != expected.sha256()
        || !matches_file(&file.metadata().map_err(|_| ERROR)?, expected.identity())
    {
        return Err(ERROR);
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct Mapping {
    device: (u32, u32),
    executable_range: String,
}

fn mapped_file(maps: &str, path: &str, expected: FileIdentity) -> Result<Mapping, &'static str> {
    let mut executable_range = None;
    let mut device = None;
    for line in maps.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.get(5).copied() != Some(path) {
            continue;
        }
        // A '(deleted)' suffix is not the verified file, even if a replacement
        // with identical bytes now occupies the original pathname.
        if fields.len() != 6 {
            return Err(ERROR);
        }
        let (start, end) = fields[0].split_once('-').ok_or(ERROR)?;
        let start = u64::from_str_radix(start, 16).map_err(|_| ERROR)?;
        let end = u64::from_str_radix(end, 16).map_err(|_| ERROR)?;
        let permissions = fields[1].as_bytes();
        let (major, minor) = fields[3].split_once(':').ok_or(ERROR)?;
        let major = u32::from_str_radix(major, 16).map_err(|_| ERROR)?;
        let minor = u32::from_str_radix(minor, 16).map_err(|_| ERROR)?;
        let inode: u64 = fields[4].parse().map_err(|_| ERROR)?;
        if start >= end
            || permissions.len() != 4
            || !matches!(permissions[0], b'r' | b'-')
            || !matches!(permissions[1], b'w' | b'-')
            || !matches!(permissions[2], b'x' | b'-')
            || !matches!(permissions[3], b'p' | b's')
            || u64::from_str_radix(fields[2], 16).is_err()
            || inode != expected.inode
            || device.is_some_and(|previous| previous != (major, minor))
        {
            return Err(ERROR);
        }
        device = Some((major, minor));
        if permissions[2] == b'x' {
            executable_range.get_or_insert_with(|| fields[0].to_owned());
        }
    }
    Ok(Mapping {
        device: device.ok_or(ERROR)?,
        executable_range: executable_range.ok_or(ERROR)?,
    })
}

fn calibrated_device(
    binary: FileIdentity,
    binary_mapping: &Mapping,
    library: FileIdentity,
    library_mapping: &Mapping,
) -> Result<(), &'static str> {
    // maps reports the superblock device; Btrfs stat reports a subvolume
    // device. Calibrate using the independently hashed running executable only
    // when both verified files belong to the same stat filesystem/subvolume.
    if binary.device == library.device && binary_mapping.device == library_mapping.device {
        Ok(())
    } else {
        Err(ERROR)
    }
}

pub(super) fn verify(
    pid: u32,
    binary: &VerifiedFile,
    library: &VerifiedFile,
) -> Result<(), &'static str> {
    let executable = format!("/proc/{pid}/exe");
    if std::fs::read_link(&executable).map_err(|_| ERROR)? != binary.path() {
        return Err(ERROR);
    }
    verify_open_file(File::open(&executable).map_err(|_| ERROR)?, binary)?;
    let mut maps = String::new();
    File::open(format!("/proc/{pid}/maps"))
        .map_err(|_| ERROR)?
        .take((MAX_MAPS_BYTES + 1) as u64)
        .read_to_string(&mut maps)
        .map_err(|_| ERROR)?;
    if maps.len() > MAX_MAPS_BYTES {
        return Err(ERROR);
    }
    let binary_mapping = mapped_file(
        &maps,
        binary.path().to_str().ok_or(ERROR)?,
        binary.identity(),
    )?;
    let library_mapping = mapped_file(
        &maps,
        library.path().to_str().ok_or(ERROR)?,
        library.identity(),
    )?;
    let mapped_path = format!("/proc/{pid}/map_files/{}", library_mapping.executable_range);
    if std::fs::read_link(&mapped_path).map_err(|_| ERROR)? != library.path() {
        return Err(ERROR);
    }
    match File::open(&mapped_path) {
        Ok(file) => verify_open_file(file, library)?,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            calibrated_device(
                binary.identity(),
                &binary_mapping,
                library.identity(),
                &library_mapping,
            )?;
        }
        Err(_) => return Err(ERROR),
    }
    library.reverify().map_err(|_| ERROR)?;
    if !matches_file(
        &std::fs::metadata(&executable).map_err(|_| ERROR)?,
        binary.identity(),
    ) || std::fs::read_link(&executable).map_err(|_| ERROR)? != binary.path()
    {
        return Err(ERROR);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_mapping_requires_consistent_device_exact_inode_and_live_executable_mapping() {
        let identity = FileIdentity {
            device: rustix::fs::makedev(8, 3),
            inode: 123,
            size: 7,
        };
        let path = "/usr/lib/libhunspell.so";
        let valid = "1000-2000 r-xp 0000 08:03 123 /usr/lib/libhunspell.so\n2000-3000 r--p 1000 08:03 123 /usr/lib/libhunspell.so";
        assert_eq!(mapped_file(valid, path, identity).unwrap().device, (8, 3));
        for changed in [
            valid.replacen("08:03", "08:04", 1),
            valid.replace(" 123 ", " 124 "),
            format!("{valid} (deleted)"),
            valid.replace("r-xp", "r--p"),
            valid.replace("1000-2000", "2000-1000"),
            valid.replace("1000-2000", "garbage"),
            valid.replace("r-xp", "r-?p"),
            valid.replace(path, "/different/library.so"),
        ] {
            assert_eq!(mapped_file(&changed, path, identity), Err(ERROR));
            assert_eq!(
                mapped_file(
                    &changed.replace(path, "/usr/bin/hunspell"),
                    "/usr/bin/hunspell",
                    identity
                ),
                Err(ERROR)
            );
        }
    }
    #[test]
    fn calibrated_device_handles_btrfs_without_accepting_another_filesystem_or_mapping_device() {
        let binary = FileIdentity {
            device: rustix::fs::makedev(0, 32),
            inode: 201_807,
            size: 137_912,
        };
        let library = FileIdentity {
            inode: 201_824,
            size: 809_080,
            ..binary
        };
        let executable = Mapping {
            device: (0, 30),
            executable_range: "1000-2000".to_owned(),
        };
        let loaded = Mapping {
            device: (0, 30),
            executable_range: "3000-4000".to_owned(),
        };
        calibrated_device(binary, &executable, library, &loaded).unwrap();
        assert_eq!(
            calibrated_device(
                binary,
                &executable,
                FileIdentity {
                    device: rustix::fs::makedev(0, 33),
                    ..library
                },
                &loaded
            ),
            Err(ERROR)
        );
        assert_eq!(
            calibrated_device(
                binary,
                &executable,
                library,
                &Mapping {
                    device: (0, 31),
                    ..loaded
                }
            ),
            Err(ERROR)
        );
        let regular = Mapping {
            device: (8, 3),
            executable_range: "1000-2000".to_owned(),
        };
        let identity = FileIdentity {
            device: rustix::fs::makedev(8, 3),
            ..binary
        };
        calibrated_device(identity, &regular, identity, &regular).unwrap();
    }
    #[test]
    fn executed_file_verification_rejects_same_bytes_on_another_inode_and_changed_bytes() {
        use crate::semantic::provenance::{FileExpectation, verify_file};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, b"fixture").unwrap();
        let expected = verify_file(
            &FileExpectation::new(&path, super::super::artifact::digest(b"fixture"), 7).unwrap(),
        )
        .unwrap();
        verify_open_file(File::open(&path).unwrap(), &expected).unwrap();
        let copy = directory.path().join("copy");
        std::fs::write(&copy, b"fixture").unwrap();
        assert_eq!(
            verify_open_file(File::open(copy).unwrap(), &expected),
            Err(ERROR)
        );
        std::fs::write(&path, b"changed").unwrap();
        assert_eq!(
            verify_open_file(File::open(path).unwrap(), &expected),
            Err(ERROR)
        );
    }
}
