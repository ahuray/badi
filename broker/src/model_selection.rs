use std::fs;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

const MIB: u64 = 1_048_576;
const HARDWARE_SCHEMA: &str = "badi.hardware.v1";
const MODEL_ADVICE_SCHEMA: &str = "badi.model-advice.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelUseCase {
    Writing,
    Code,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Compact,
    Balanced,
    Quality,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct CpuFeatures {
    pub avx2: bool,
    pub avx512f: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MemoryProfile {
    pub total_mib: Option<u64>,
    pub available_mib: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct GpuProfile {
    pub nvidia: bool,
    pub amd: bool,
    pub intel: bool,
    pub dedicated_memory_mib: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HardwareProfile {
    pub schema: &'static str,
    pub architecture: String,
    pub logical_cpus: usize,
    pub cpu: CpuFeatures,
    pub memory: MemoryProfile,
    pub gpu: GpuProfile,
    pub on_battery: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ModelArtifact {
    pub use_case: ModelUseCase,
    pub tier: ModelTier,
    pub repository: &'static str,
    pub revision: &'static str,
    pub filename: &'static str,
    pub sha256: &'static str,
    pub download_bytes: u64,
    pub quantization: &'static str,
    pub license: &'static str,
    pub runtime: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadPlan {
    pub tool: &'static str,
    pub arguments: Vec<&'static str>,
    pub expected_sha256: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelAdvice {
    pub schema: &'static str,
    pub hardware: HardwareProfile,
    pub use_case: ModelUseCase,
    pub tier: ModelTier,
    pub recommended: ModelArtifact,
    pub alternatives: Vec<ModelArtifact>,
    pub rationale: Vec<String>,
    pub download: DownloadPlan,
    pub runtime_ready: bool,
}

const WRITING_MODELS: [ModelArtifact; 3] = [
    ModelArtifact {
        use_case: ModelUseCase::Writing,
        tier: ModelTier::Compact,
        repository: "Qwen/Qwen3-0.6B-GGUF",
        revision: "23749fefcc72300e3a2ad315e1317431b06b590a",
        filename: "Qwen3-0.6B-Q8_0.gguf",
        sha256: "9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031",
        download_bytes: 639_446_688,
        quantization: "Q8_0",
        license: "Apache-2.0",
        runtime: "llama.cpp",
    },
    ModelArtifact {
        use_case: ModelUseCase::Writing,
        tier: ModelTier::Balanced,
        repository: "Qwen/Qwen3-1.7B-GGUF",
        revision: "90862c4b9d2787eaed51d12237eafdfe7c5f6077",
        filename: "Qwen3-1.7B-Q8_0.gguf",
        sha256: "061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a",
        download_bytes: 1_834_426_016,
        quantization: "Q8_0",
        license: "Apache-2.0",
        runtime: "llama.cpp",
    },
    ModelArtifact {
        use_case: ModelUseCase::Writing,
        tier: ModelTier::Quality,
        repository: "Qwen/Qwen3-4B-GGUF",
        revision: "bc640142c66e1fdd12af0bd68f40445458f3869b",
        filename: "Qwen3-4B-Q4_K_M.gguf",
        sha256: "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
        download_bytes: 2_497_280_256,
        quantization: "Q4_K_M",
        license: "Apache-2.0",
        runtime: "llama.cpp",
    },
];

const CODE_MODELS: [ModelArtifact; 3] = [
    ModelArtifact {
        use_case: ModelUseCase::Code,
        tier: ModelTier::Compact,
        repository: "Qwen/Qwen2.5-Coder-0.5B-Instruct-GGUF",
        revision: "ebb2015119c907b064c512bf053e945850b5875f",
        filename: "qwen2.5-coder-0.5b-instruct-q4_k_m.gguf",
        sha256: "1d9614638d18024d0fbb36575a15f1302a3adf044df10345688ec4f6e1c4ff32",
        download_bytes: 491_400_064,
        quantization: "Q4_K_M",
        license: "Apache-2.0",
        runtime: "llama.cpp",
    },
    ModelArtifact {
        use_case: ModelUseCase::Code,
        tier: ModelTier::Balanced,
        repository: "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF",
        revision: "f86cb2c1fa58255f8052cc32aeede1b7482d4361",
        filename: "qwen2.5-coder-1.5b-instruct-q4_k_m.gguf",
        sha256: "cc324af070c2ecbfd324a30884d2f951a7ff756aba85cb811a6ec436933bb046",
        download_bytes: 1_117_320_768,
        quantization: "Q4_K_M",
        license: "Apache-2.0",
        runtime: "llama.cpp",
    },
    ModelArtifact {
        use_case: ModelUseCase::Code,
        tier: ModelTier::Quality,
        repository: "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF",
        revision: "13fb94bfda8c8cf22497dc57b78f391a9acb426a",
        filename: "qwen2.5-coder-7b-instruct-q4_k_m.gguf",
        sha256: "509287f78cb4d4cf6b3843734733b914b2c158e43e22a7f4bf5e963800894d3c",
        download_bytes: 4_683_073_536,
        quantization: "Q4_K_M",
        license: "Apache-2.0",
        runtime: "llama.cpp",
    },
];

#[must_use]
pub fn detect_hardware() -> HardwareProfile {
    let memory = fs::read_to_string("/proc/meminfo")
        .map_or_else(|_| MemoryProfile::default(), |value| parse_meminfo(&value));
    HardwareProfile {
        schema: HARDWARE_SCHEMA,
        architecture: std::env::consts::ARCH.to_owned(),
        logical_cpus: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        cpu: detect_cpu_features(),
        memory,
        gpu: detect_gpus(Path::new("/sys/class/drm")),
        on_battery: detect_on_battery(Path::new("/sys/class/power_supply")),
    }
}

#[must_use]
pub fn recommend_model(hardware: HardwareProfile, use_case: ModelUseCase) -> ModelAdvice {
    let tier = recommended_tier(&hardware);
    let catalog = catalog(use_case);
    let recommended = *catalog
        .iter()
        .find(|model| model.tier == tier)
        .expect("every catalog contains every tier");
    let alternatives = catalog
        .iter()
        .copied()
        .filter(|model| model.tier != tier)
        .collect();
    let rationale = rationale(&hardware, tier);
    let download = DownloadPlan {
        tool: "hf",
        arguments: vec![
            "download",
            recommended.repository,
            recommended.filename,
            "--revision",
            recommended.revision,
        ],
        expected_sha256: recommended.sha256,
    };
    ModelAdvice {
        schema: MODEL_ADVICE_SCHEMA,
        hardware,
        use_case,
        tier,
        recommended,
        alternatives,
        rationale,
        download,
        runtime_ready: false,
    }
}

#[must_use]
pub fn recommended_tier(hardware: &HardwareProfile) -> ModelTier {
    let total = hardware.memory.total_mib.unwrap_or(0);
    let available = hardware.memory.available_mib.unwrap_or(0);
    let vram = hardware.gpu.dedicated_memory_mib.unwrap_or(0);
    let slow_x86 = hardware.architecture == "x86_64" && !hardware.cpu.avx2;

    if slow_x86 || hardware.logical_cpus < 4 || total < 6_144 || available < 1_536 {
        return ModelTier::Compact;
    }

    let gpu_has_headroom = vram >= 6_144 && total >= 8_192 && available >= 3_072;
    let host_has_headroom = total >= 16_384 && available >= 6_144 && hardware.logical_cpus >= 8;
    if (gpu_has_headroom || host_has_headroom) && hardware.on_battery != Some(true) {
        ModelTier::Quality
    } else {
        ModelTier::Balanced
    }
}

#[must_use]
pub fn parse_meminfo(value: &str) -> MemoryProfile {
    MemoryProfile {
        total_mib: meminfo_mib(value, "MemTotal"),
        available_mib: meminfo_mib(value, "MemAvailable"),
    }
}

fn meminfo_mib(value: &str, wanted: &str) -> Option<u64> {
    value.lines().find_map(|line| {
        let (key, rest) = line.split_once(':')?;
        if key != wanted {
            return None;
        }
        rest.split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
            .map(|kib| kib / 1_024)
    })
}

fn catalog(use_case: ModelUseCase) -> &'static [ModelArtifact; 3] {
    match use_case {
        ModelUseCase::Writing => &WRITING_MODELS,
        ModelUseCase::Code => &CODE_MODELS,
    }
}

fn rationale(hardware: &HardwareProfile, tier: ModelTier) -> Vec<String> {
    let total = hardware
        .memory
        .total_mib
        .map_or_else(|| "unknown RAM".to_owned(), |mib| format!("{mib} MiB RAM"));
    let available = hardware.memory.available_mib.map_or_else(
        || "unknown available memory".to_owned(),
        |mib| format!("{mib} MiB currently available"),
    );
    let mut reasons = vec![format!(
        "{total}, {available}, and {} logical CPUs select the {tier:?} tier.",
        hardware.logical_cpus
    )];
    if let Some(vram) = hardware.gpu.dedicated_memory_mib {
        reasons.push(format!("Detected up to {vram} MiB dedicated GPU memory."));
    }
    if hardware.on_battery == Some(true) {
        reasons.push("Battery power caps the recommendation below the quality tier.".to_owned());
    }
    reasons.push(
        "This is a candidate only; Badi will not activate it until latency and quality gates pass."
            .to_owned(),
    );
    reasons
}

fn detect_cpu_features() -> CpuFeatures {
    CpuFeatures {
        #[cfg(target_arch = "x86_64")]
        avx2: std::arch::is_x86_feature_detected!("avx2"),
        #[cfg(not(target_arch = "x86_64"))]
        avx2: false,
        #[cfg(target_arch = "x86_64")]
        avx512f: std::arch::is_x86_feature_detected!("avx512f"),
        #[cfg(not(target_arch = "x86_64"))]
        avx512f: false,
    }
}

fn detect_gpus(root: &Path) -> GpuProfile {
    let mut profile = GpuProfile::default();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !is_drm_card(name) {
                continue;
            }
            let device = entry.path().join("device");
            let vendor = fs::read_to_string(device.join("vendor")).unwrap_or_default();
            match vendor.trim() {
                "0x10de" => profile.nvidia = true,
                "0x1002" => profile.amd = true,
                "0x8086" => profile.intel = true,
                _ => {}
            }
            if let Ok(bytes) = fs::read_to_string(device.join("mem_info_vram_total")) {
                if let Ok(bytes) = bytes.trim().parse::<u64>() {
                    let mib = bytes / MIB;
                    profile.dedicated_memory_mib = Some(
                        profile
                            .dedicated_memory_mib
                            .map_or(mib, |current| current.max(mib)),
                    );
                }
            }
        }
    }
    if let Some(mib) = nvidia_vram_mib() {
        profile.nvidia = true;
        profile.dedicated_memory_mib = Some(
            profile
                .dedicated_memory_mib
                .map_or(mib, |current| current.max(mib)),
        );
    }
    profile
}

fn is_drm_card(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn nvidia_vram_mib() -> Option<u64> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .max()
}

fn detect_on_battery(root: &Path) -> Option<bool> {
    let entries = fs::read_dir(root).ok()?;
    let mut found_battery = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if fs::read_to_string(path.join("type")).is_ok_and(|value| value.trim() == "Battery") {
            found_battery = true;
            if fs::read_to_string(path.join("status"))
                .is_ok_and(|value| value.trim() == "Discharging")
            {
                return Some(true);
            }
        }
    }
    found_battery.then_some(false)
}

#[cfg(test)]
mod tests {
    use super::{
        CODE_MODELS, CpuFeatures, GpuProfile, HARDWARE_SCHEMA, HardwareProfile, MemoryProfile,
        ModelTier, ModelUseCase, WRITING_MODELS, parse_meminfo, recommend_model, recommended_tier,
    };

    fn profile(total_mib: u64, available_mib: u64, logical_cpus: usize) -> HardwareProfile {
        HardwareProfile {
            schema: HARDWARE_SCHEMA,
            architecture: "x86_64".to_owned(),
            logical_cpus,
            cpu: CpuFeatures {
                avx2: true,
                avx512f: false,
            },
            memory: MemoryProfile {
                total_mib: Some(total_mib),
                available_mib: Some(available_mib),
            },
            gpu: GpuProfile::default(),
            on_battery: Some(false),
        }
    }

    #[test]
    fn parses_linux_memory_without_guessing_missing_values() {
        let memory = parse_meminfo(
            "MemTotal:       16384000 kB\nMemFree: 100 kB\nMemAvailable: 8192000 kB\n",
        );
        assert_eq!(
            memory,
            MemoryProfile {
                total_mib: Some(16_000),
                available_mib: Some(8_000),
            }
        );
        assert_eq!(parse_meminfo("MemTotal: 1024 kB\n").available_mib, None);
    }

    #[test]
    fn recommendation_stays_inside_safe_memory_and_power_tiers() {
        assert_eq!(
            recommended_tier(&profile(4_096, 3_000, 8)),
            ModelTier::Compact
        );
        assert_eq!(
            recommended_tier(&profile(8_192, 4_000, 8)),
            ModelTier::Balanced
        );
        assert_eq!(
            recommended_tier(&profile(32_768, 20_000, 16)),
            ModelTier::Quality
        );

        let mut battery = profile(32_768, 20_000, 16);
        battery.on_battery = Some(true);
        assert_eq!(recommended_tier(&battery), ModelTier::Balanced);
    }

    #[test]
    fn dedicated_memory_can_select_quality_with_host_headroom() {
        let mut hardware = profile(8_192, 4_000, 8);
        hardware.gpu = GpuProfile {
            nvidia: true,
            dedicated_memory_mib: Some(8_192),
            ..GpuProfile::default()
        };
        assert_eq!(recommended_tier(&hardware), ModelTier::Quality);
    }

    #[test]
    fn use_case_selects_a_pinned_artifact_and_non_executing_download_plan() {
        let advice = recommend_model(profile(8_192, 4_000, 8), ModelUseCase::Code);
        assert_eq!(advice.tier, ModelTier::Balanced);
        assert_eq!(
            advice.recommended.repository,
            "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF"
        );
        assert_eq!(advice.download.tool, "hf");
        assert!(!advice.runtime_ready);
    }

    #[test]
    fn catalog_metadata_is_pinned_and_uses_safe_weight_files() {
        for model in WRITING_MODELS.into_iter().chain(CODE_MODELS) {
            assert!(
                model.revision.len() == 40 && model.revision.bytes().all(|b| b.is_ascii_hexdigit())
            );
            assert!(
                model.sha256.len() == 64 && model.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            );
            assert!(
                std::path::Path::new(model.filename)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
            );
            assert_eq!(std::path::Path::new(model.filename).components().count(), 1);
            assert_eq!(model.license, "Apache-2.0");
        }
    }
}
