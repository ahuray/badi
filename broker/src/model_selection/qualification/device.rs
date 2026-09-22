use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::super::{detect_hardware, is_drm_card, run_bounded_command};
use super::{MIB, digest, now_unix_s};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpuInspection {
    pub model: Option<String>,
    pub logical_cpus: usize,
    pub physical_cores: Option<usize>,
    pub packages: Option<usize>,
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuInspection {
    pub id: String,
    pub vendor: String,
    pub device: Option<String>,
    pub driver: Option<String>,
    /// Unknown is retained instead of assigning integrated GPU reservations as VRAM.
    pub memory_kind: String,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    /// Only actual owned-runtime execution evidence can validate a backend.
    pub execution_verified: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskInspection {
    pub path: String,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInspection {
    pub schema: String,
    pub inspected_at_unix_s: u64,
    pub fingerprint: String,
    pub architecture: String,
    pub cpu: CpuInspection,
    pub total_memory_bytes: Option<u64>,
    pub available_memory_bytes: Option<u64>,
    pub gpus: Vec<GpuInspection>,
    pub disk: DiskInspection,
    pub on_battery: Option<bool>,
    pub power_profile: Option<String>,
    pub memory_pressure: Option<String>,
    pub limitations: Vec<String>,
}

#[must_use]
pub fn inspect_device(cache_path: &Path) -> DeviceInspection {
    let hardware = detect_hardware();
    let mut report = DeviceInspection {
        schema: "badi.device-inspection.v1".to_owned(),
        inspected_at_unix_s: now_unix_s(),
        fingerprint: String::new(),
        architecture: hardware.architecture,
        cpu: inspect_cpu(hardware.logical_cpus),
        total_memory_bytes: hardware.memory.total_mib.map(|value| value * MIB),
        available_memory_bytes: hardware.memory.available_mib.map(|value| value * MIB),
        gpus: inspect_gpus(Path::new("/sys/class/drm")),
        disk: inspect_disk(cache_path),
        on_battery: hardware.on_battery,
        power_profile: read_trimmed(Path::new("/sys/firmware/acpi/platform_profile")),
        memory_pressure: read_trimmed(Path::new("/proc/pressure/memory")),
        limitations: vec![
            "GPU detection is not proof of llama.cpp execution. No detected GPU memory is added to host RAM.".to_owned(),
            "Available memory, disk and power are a snapshot; assessment must run again immediately before loading.".to_owned(),
        ],
    };
    // Stable device/configuration identity excludes volatile free capacity and timestamps.
    // Power changes invalidate performance evidence even when the model still fits.
    report.fingerprint = fingerprint(&report);
    report
}

pub(super) fn fingerprint(report: &DeviceInspection) -> String {
    let gpu_identity: Vec<_> = report
        .gpus
        .iter()
        .map(|gpu| {
            (
                &gpu.id,
                &gpu.vendor,
                &gpu.device,
                &gpu.driver,
                &gpu.memory_kind,
                gpu.total_bytes,
            )
        })
        .collect();
    digest(&(
        &report.architecture,
        &report.cpu,
        report.total_memory_bytes,
        gpu_identity,
        report.on_battery,
        &report.power_profile,
    ))
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn inspect_cpu(logical_cpus: usize) -> CpuInspection {
    let text = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim() == key).then(|| value.trim().to_owned())
        })
    };
    let mut cores = BTreeSet::new();
    let mut packages = BTreeSet::new();
    if let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.strip_prefix("cpu").is_some_and(|value| {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
            }) {
                continue;
            }
            let topology = entry.path().join("topology");
            if let (Some(package), Some(core)) = (
                read_trimmed(&topology.join("physical_package_id")),
                read_trimmed(&topology.join("core_id")),
            ) {
                packages.insert(package.clone());
                cores.insert((package, core));
            }
        }
    }
    CpuInspection {
        model: field("model name").or_else(|| field("Hardware")),
        logical_cpus,
        physical_cores: (!cores.is_empty()).then_some(cores.len()),
        packages: (!packages.is_empty()).then_some(packages.len()),
        features: field("flags")
            .or_else(|| field("Features"))
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    }
}

fn inspect_gpus(root: &Path) -> Vec<GpuInspection> {
    let mut result = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            if !is_drm_card(&id) {
                continue;
            }
            let path = entry.path().join("device");
            let vendor = read_trimmed(&path.join("vendor")).unwrap_or_else(|| "unknown".to_owned());
            let total = read_number(&path.join("mem_info_vram_total"));
            let used = read_number(&path.join("mem_info_vram_used"));
            // sysfs does not reliably distinguish UMA carve-outs from discrete VRAM.
            // Intel has integrated and discrete products; vendor alone cannot decide.
            result.push(GpuInspection {
                id,
                vendor,
                device: read_trimmed(&path.join("device")),
                driver: fs::read_link(path.join("driver")).ok().and_then(|value| {
                    value
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                }),
                memory_kind: "unknown".to_owned(),
                total_bytes: total,
                available_bytes: total
                    .zip(used)
                    .and_then(|(total, used)| total.checked_sub(used)),
                execution_verified: false,
            });
        }
    }
    result.sort_by(|left, right| left.id.cmp(&right.id));
    result
}

fn read_number(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

fn inspect_disk(requested: &Path) -> DiskInspection {
    let path = requested
        .ancestors()
        .find(|path| path.exists())
        .unwrap_or(requested);
    let mut command = Command::new("df");
    command
        .args(["--block-size=1", "--output=size,avail", "--"])
        .arg(path)
        .env("LC_ALL", "C");
    let pair = run_bounded_command(&mut command, Duration::from_secs(2), 4096)
        .filter(|output| output.status.success() && !output.truncated)
        .and_then(|output| parse_disk(&String::from_utf8_lossy(&output.stdout)));
    DiskInspection {
        path: path.to_string_lossy().to_string(),
        total_bytes: pair.map(|pair| pair.0),
        available_bytes: pair.map(|pair| pair.1),
    }
}

fn parse_disk(text: &str) -> Option<(u64, u64)> {
    let mut fields = text.lines().nth(1)?.split_whitespace();
    let total = fields.next()?.parse::<u64>().ok()?;
    let available = fields.next()?.parse::<u64>().ok()?;
    (fields.next().is_none() && total > 0 && available <= total).then_some((total, available))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_probe_does_not_treat_missing_or_inconsistent_values_as_capacity() {
        assert_eq!(parse_disk(" 1B-blocks Avail\n 100 90\n"), Some((100, 90)));
        assert_eq!(parse_disk("1B-blocks Avail\n100 101\n"), None);
        assert_eq!(parse_disk("1B-blocks Avail\n"), None);
        assert_eq!(parse_disk("1B-blocks Avail\n0 0\n"), None);
    }

    #[test]
    fn gpu_vendor_never_proves_dedicated_capacity_or_execution() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("card0/device");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("vendor"), "0x8086\n").unwrap();
        fs::write(path.join("mem_info_vram_total"), "1024").unwrap();
        fs::write(path.join("mem_info_vram_used"), "2048").unwrap();
        let devices = inspect_gpus(root.path());
        assert_eq!(devices[0].memory_kind, "unknown");
        assert_eq!(devices[0].available_bytes, None);
        assert!(!devices[0].execution_verified);
    }
}
