use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use std::cmp::Reverse;

use serde::Serialize;

const MIB: u64 = 1_048_576;
const HOST_RESERVE_MIB: u64 = 2_048;
const RUNTIME_BASE_MIB: u64 = 768;
const HARDWARE_SCHEMA: &str = "badi.hardware.v1";
const MODEL_ADVICE_SCHEMA: &str = "badi.model-advice.v3";
const LLAMA_CPP_MINIMUM_VERSION: &str = "b5092";
const NVIDIA_SMI_TIMEOUT: Duration = Duration::from_secs(2);
const NVIDIA_SMI_MAX_STDOUT_BYTES: usize = 16 * 1_024;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdviceStatus {
    Candidate,
    NoFit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdviceReason {
    CandidateFitsHostMemory,
    MemoryCapacityUnknown,
    MemoryCapacityInvalid,
    InsufficientUsableMemory,
    InsufficientCompute,
    UnsupportedArchitecture,
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

// The three vendor flags are a stable additive JSON contract; replacing them
// with an enum collection would make hardware v1 harder to consume.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct GpuProfile {
    pub nvidia: bool,
    pub amd: bool,
    pub intel: bool,
    pub hybrid: bool,
    /// Largest detected total dedicated-memory value. This is not usable capacity.
    pub dedicated_memory_mib: Option<u64>,
    /// Capacity exposed by a validated inference backend. Detection alone never sets this.
    pub usable_memory_mib: Option<u64>,
    /// Validated backend associated with `usable_memory_mib`, when one exists.
    pub backend: Option<String>,
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
    /// Stable short name, as `badi model use ID` and the settings name it.
    pub id: &'static str,
    pub name: &'static str,
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
    pub minimum_runtime_version: &'static str,
    pub prompt_format: &'static str,
    pub runtime_caveat: &'static str,
    /// Languages Badi measured or routes for this model.
    pub languages: &'static [&'static str],
    /// Blind-reviewed useful suggestions among the 135 development requests
    /// of the 2026-10-17 Prediction Lab screen (en, de, fr, es, fa), when
    /// the model took part. A screening score, not a qualification.
    pub screen_useful: Option<u16>,
    /// Languages whose trailing space is healed with this model by default,
    /// each promoted by its own blind review against the unhealed prompt;
    /// `trailing_space_healing` in settings overrides it per language.
    pub healed_languages: &'static [&'static str],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadPlan {
    pub tool: &'static str,
    pub arguments: Vec<&'static str>,
    pub expected_sha256: &'static str,
}

/// Where `hf download --local-dir` must place a writing GGUF. The broker reads
/// `~/.local/share/badi/models/<filename>`, not the Hugging Face cache.
pub const MODEL_DOWNLOAD_DIRECTORY: &str = "~/.local/share/badi/models";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryFit {
    pub execution_target: &'static str,
    pub usable_host_memory_mib: u64,
    pub artifact_memory_mib: u64,
    pub runtime_headroom_mib: u64,
    pub required_host_memory_mib: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelAdvice {
    pub schema: &'static str,
    pub status: AdviceStatus,
    pub reason: AdviceReason,
    pub hardware: HardwareProfile,
    pub use_case: ModelUseCase,
    pub tier: Option<ModelTier>,
    pub recommended: Option<ModelArtifact>,
    pub alternatives: Vec<ModelArtifact>,
    pub rationale: Vec<String>,
    pub fit: Option<MemoryFit>,
    pub download: Option<DownloadPlan>,
    pub runtime_ready: bool,
    /// Every pinned model for this use, and whether it fits this device.
    pub catalog: Vec<CatalogChoice>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogChoice {
    #[serde(flatten)]
    pub artifact: ModelArtifact,
    pub fits: bool,
}

const WRITING_PROMPT_FORMAT: &str = "llama_cpp_native_prefix_completion_v1";
const WRITING_RUNTIME_CAVEAT: &str =
    "Native-prefix context bounds, latency, and inline quality are not validated by Badi.";
const CODE_PROMPT_FORMAT: &str = "qwen2_5_coder_instruct_chat_template";
const CODE_RUNTIME_CAVEAT: &str = "An instruct GGUF is not proof of fill-in-the-middle quality; latency and inline quality are not validated by Badi.";

/// Writing models, compact first. Within a tier the recommendation prefers
/// the higher screen score, then the smaller download.
const WRITING_LANGUAGES: &[&str] = &["en", "de", "fr", "es", "fa"];
const CODE_LANGUAGES: &[&str] = &["code"];

const WRITING_MODELS: [ModelArtifact; 4] = [
    ModelArtifact {
        id: "qwen3-0.6b",
        name: "Qwen3 0.6B",
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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: WRITING_PROMPT_FORMAT,
        runtime_caveat: WRITING_RUNTIME_CAVEAT,
        languages: WRITING_LANGUAGES,
        screen_useful: None,
        healed_languages: &["en", "fa"],
    },
    ModelArtifact {
        id: "qwen3-1.7b",
        name: "Qwen3 1.7B",
        use_case: ModelUseCase::Writing,
        tier: ModelTier::Balanced,
        repository: "ggml-org/Qwen3-1.7B-GGUF",
        revision: "daeb8e2d528a760970442092f6bf1e55c3b659eb",
        filename: "Qwen3-1.7B-Q4_K_M.gguf",
        sha256: "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5",
        download_bytes: 1_282_439_264,
        quantization: "Q4_K_M",
        license: "Apache-2.0",
        runtime: "llama.cpp",
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: WRITING_PROMPT_FORMAT,
        runtime_caveat: WRITING_RUNTIME_CAVEAT,
        languages: WRITING_LANGUAGES,
        screen_useful: Some(33),
        healed_languages: &["en", "fa"],
    },
    ModelArtifact {
        id: "llama-3.2-1b",
        name: "Llama 3.2 1B",
        use_case: ModelUseCase::Writing,
        tier: ModelTier::Balanced,
        repository: "QuantFactory/Llama-3.2-1B-GGUF",
        revision: "983f8cd1e4a8227322945f7f4a9aa97346ff1f65",
        filename: "Llama-3.2-1B.Q4_K_M.gguf",
        sha256: "ac81ee4bf5b82d94f380a87fd48560254b970107a94b6f228b2a36991853af60",
        download_bytes: 807_690_208,
        quantization: "Q4_K_M",
        license: "Llama-3.2-Community",
        runtime: "llama.cpp",
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: WRITING_PROMPT_FORMAT,
        runtime_caveat: WRITING_RUNTIME_CAVEAT,
        languages: WRITING_LANGUAGES,
        screen_useful: Some(41),
        healed_languages: &["en", "fa"],
    },
    ModelArtifact {
        id: "gemma-3-1b",
        name: "Gemma 3 1B",
        use_case: ModelUseCase::Writing,
        tier: ModelTier::Balanced,
        repository: "gguf-org/gemma-3-1b-pt-gguf",
        revision: "53a59a084b70c19aaa73f5854538f6b3da32d043",
        filename: "gemma-3-1b-pt-q4_k_m.gguf",
        sha256: "3c313086e97e47432c8aa45e2884c0f9fe727af9766c2291d06b8cb1e709af0f",
        download_bytes: 806_056_736,
        quantization: "Q4_K_M",
        license: "Gemma",
        runtime: "llama.cpp",
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: WRITING_PROMPT_FORMAT,
        runtime_caveat: WRITING_RUNTIME_CAVEAT,
        languages: WRITING_LANGUAGES,
        screen_useful: Some(46),
        // German: 15 useful against 7 of 67, 3 harmful against 2 (user
        // decision, 2026-10-18). French and Spanish: 12 and 14 useful of 20
        // boundary requests ending in a space, none harmful, where the
        // unhealed prompt shows nothing (2026-10-18 small-model screen).
        healed_languages: &["en", "de", "fa", "fr", "es"],
    },
];

const CODE_MODELS: [ModelArtifact; 3] = [
    ModelArtifact {
        id: "qwen2.5-coder-0.5b",
        name: "Qwen2.5 Coder 0.5B",
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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: CODE_PROMPT_FORMAT,
        runtime_caveat: CODE_RUNTIME_CAVEAT,
        languages: CODE_LANGUAGES,
        screen_useful: None,
        healed_languages: &[],
    },
    ModelArtifact {
        id: "qwen2.5-coder-1.5b",
        name: "Qwen2.5 Coder 1.5B",
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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: CODE_PROMPT_FORMAT,
        runtime_caveat: CODE_RUNTIME_CAVEAT,
        languages: CODE_LANGUAGES,
        screen_useful: None,
        healed_languages: &[],
    },
    ModelArtifact {
        id: "qwen2.5-coder-7b",
        name: "Qwen2.5 Coder 7B",
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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: CODE_PROMPT_FORMAT,
        runtime_caveat: CODE_RUNTIME_CAVEAT,
        languages: CODE_LANGUAGES,
        screen_useful: None,
        healed_languages: &[],
    },
];

#[must_use]
pub fn detect_hardware() -> HardwareProfile {
    detect_hardware_with(&SystemNvidiaMemoryProbe)
}

/// Like [`detect_hardware`] without running `nvidia-smi`. CPU inference never
/// uses GPU capacity; DRM vendor detection alone still marks hybrid systems.
#[cfg(feature = "local-model")]
#[must_use]
pub(crate) fn detect_cpu_inference_hardware() -> HardwareProfile {
    detect_hardware_with(&NoNvidiaMemoryProbe)
}

fn detect_hardware_with(nvidia_probe: &dyn NvidiaMemoryProbe) -> HardwareProfile {
    HardwareProfile {
        schema: HARDWARE_SCHEMA,
        architecture: std::env::consts::ARCH.to_owned(),
        logical_cpus: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        cpu: detect_cpu_features(),
        memory: current_memory(),
        gpu: detect_gpus_with(Path::new("/sys/class/drm"), nvidia_probe),
        on_battery: detect_on_battery(Path::new("/sys/class/power_supply")),
    }
}

#[must_use]
pub fn current_memory() -> MemoryProfile {
    fs::read_to_string("/proc/meminfo")
        .map_or_else(|_| MemoryProfile::default(), |value| parse_meminfo(&value))
}

#[must_use]
pub fn recommend_model(hardware: HardwareProfile, use_case: ModelUseCase) -> ModelAdvice {
    let selection = select_model(&hardware, use_case);
    let catalog = catalog(use_case)
        .iter()
        .map(|artifact| CatalogChoice {
            artifact: *artifact,
            fits: selection.as_ref().is_ok_and(|selection| {
                selection.recommended == *artifact || selection.alternatives.contains(artifact)
            }),
        })
        .collect();
    match selection {
        Ok(selection) => {
            let recommended = selection.recommended;
            let download = DownloadPlan {
                tool: "hf",
                arguments: vec![
                    "download",
                    recommended.repository,
                    recommended.filename,
                    "--revision",
                    recommended.revision,
                    "--local-dir",
                    MODEL_DOWNLOAD_DIRECTORY,
                ],
                expected_sha256: recommended.sha256,
            };
            ModelAdvice {
                schema: MODEL_ADVICE_SCHEMA,
                status: AdviceStatus::Candidate,
                reason: AdviceReason::CandidateFitsHostMemory,
                rationale: candidate_rationale(&hardware, recommended, selection.fit),
                hardware,
                use_case,
                tier: Some(recommended.tier),
                recommended: Some(recommended),
                alternatives: selection.alternatives,
                fit: Some(selection.fit),
                download: Some(download),
                runtime_ready: false,
                catalog,
            }
        }
        Err(reason) => ModelAdvice {
            schema: MODEL_ADVICE_SCHEMA,
            status: AdviceStatus::NoFit,
            reason,
            rationale: no_fit_rationale(&hardware, reason),
            hardware,
            use_case,
            tier: None,
            recommended: None,
            alternatives: Vec::new(),
            fit: None,
            download: None,
            runtime_ready: false,
            catalog,
        },
    }
}

/// Download advice expresses a preference, not the identity of an installation.
/// Power changes may alter that preference without making an installed artifact
/// incompatible. Selection uses total memory, so a transient dip in available
/// memory never changes or refuses the installed model; total memory minus the
/// host reserve is the hard floor. [`fits_available_memory`] gates the start.
#[cfg(feature = "local-model")]
pub(crate) fn select_installed_writing_model(
    hardware: &HardwareProfile,
    installed_filenames: &[&str],
    chosen: Option<&str>,
) -> Result<Option<ModelArtifact>, AdviceReason> {
    let mut capacity = hardware.clone();
    capacity.memory.available_mib = hardware.memory.total_mib;
    let selection = select_model(&capacity, ModelUseCase::Writing)?;
    let usable = selection.fit.usable_host_memory_mib;
    let mut installed: Vec<_> = catalog(ModelUseCase::Writing)
        .iter()
        .copied()
        .filter(|model| installed_filenames.contains(&model.filename))
        .collect();
    if installed.is_empty() {
        return Ok(None);
    }
    installed.retain(|model| memory_fit(*model, usable).required_host_memory_mib <= usable);
    // The user's installed choice wins whenever it fits safely; otherwise
    // the advised tier comes first, then the recommendation's preference.
    if let Some(model) = chosen.and_then(|id| installed.iter().find(|model| model.id == id)) {
        return Ok(Some(*model));
    }
    installed.sort_by_key(|model| {
        Reverse((model.tier <= selection.recommended.tier, preference(model)))
    });
    installed
        .first()
        .copied()
        .map(Some)
        .ok_or(AdviceReason::InsufficientUsableMemory)
}

/// Whether `model` can start now, with its runtime headroom, without eating
/// into the host reserve.
#[cfg(feature = "local-model")]
#[must_use]
pub(crate) fn fits_available_memory(model: ModelArtifact, available_mib: u64) -> bool {
    let usable = available_mib.saturating_sub(HOST_RESERVE_MIB);
    memory_fit(model, usable).required_host_memory_mib <= usable
}

#[must_use]
pub fn recommended_tier(hardware: &HardwareProfile) -> ModelTier {
    let Ok((total_mib, available_mib)) = reported_memory(hardware) else {
        return ModelTier::Compact;
    };
    tier_ceiling(hardware, total_mib, available_mib).unwrap_or(ModelTier::Compact)
}

#[must_use]
pub fn parse_meminfo(value: &str) -> MemoryProfile {
    MemoryProfile {
        total_mib: meminfo_mib(value, "MemTotal"),
        available_mib: meminfo_mib(value, "MemAvailable"),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Selection {
    recommended: ModelArtifact,
    alternatives: Vec<ModelArtifact>,
    fit: MemoryFit,
}

fn select_model(
    hardware: &HardwareProfile,
    use_case: ModelUseCase,
) -> Result<Selection, AdviceReason> {
    let (total_mib, available_mib) = reported_memory(hardware)?;
    let ceiling = tier_ceiling(hardware, total_mib, available_mib)?;
    let usable_host_memory_mib = available_mib
        .saturating_sub(HOST_RESERVE_MIB)
        .min(total_mib.saturating_sub(HOST_RESERVE_MIB));
    let mut fitting: Vec<ModelArtifact> = catalog(use_case)
        .iter()
        .copied()
        .filter(|artifact| {
            artifact.tier <= ceiling
                && memory_fit(*artifact, usable_host_memory_mib).required_host_memory_mib
                    <= usable_host_memory_mib
        })
        .collect();
    fitting.sort_by_key(|artifact| Reverse(preference(artifact)));
    let recommended = *fitting
        .first()
        .ok_or(AdviceReason::InsufficientUsableMemory)?;
    Ok(Selection {
        recommended,
        alternatives: fitting[1..].to_vec(),
        fit: memory_fit(recommended, usable_host_memory_mib),
    })
}

/// What a recommendation prefers among models that fit: a higher screen
/// score, then a higher tier, then a smaller download.
fn preference(artifact: &ModelArtifact) -> (u16, ModelTier, Reverse<u64>) {
    (
        artifact.screen_useful.unwrap_or(0),
        artifact.tier,
        Reverse(artifact.download_bytes),
    )
}

fn reported_memory(hardware: &HardwareProfile) -> Result<(u64, u64), AdviceReason> {
    let total_mib = hardware
        .memory
        .total_mib
        .ok_or(AdviceReason::MemoryCapacityUnknown)?;
    let available_mib = hardware
        .memory
        .available_mib
        .ok_or(AdviceReason::MemoryCapacityUnknown)?;
    if total_mib == 0 || available_mib > total_mib {
        return Err(AdviceReason::MemoryCapacityInvalid);
    }
    Ok((total_mib, available_mib))
}

fn tier_ceiling(
    hardware: &HardwareProfile,
    total_mib: u64,
    available_mib: u64,
) -> Result<ModelTier, AdviceReason> {
    if !matches!(hardware.architecture.as_str(), "x86_64" | "aarch64") {
        return Err(AdviceReason::UnsupportedArchitecture);
    }
    if hardware.logical_cpus < 4 {
        return Err(AdviceReason::InsufficientCompute);
    }
    let constrained_cpu = hardware.architecture == "aarch64"
        || (hardware.architecture == "x86_64" && !hardware.cpu.avx2);
    if constrained_cpu || hardware.on_battery == Some(true) {
        return Ok(ModelTier::Compact);
    }
    if hardware.gpu.hybrid || hardware.on_battery.is_none() {
        return Ok(ModelTier::Balanced);
    }
    if total_mib >= 24_576 && available_mib >= 8_192 && hardware.logical_cpus >= 12 {
        return Ok(ModelTier::Quality);
    }
    if total_mib >= 8_192 && hardware.logical_cpus >= 6 {
        Ok(ModelTier::Balanced)
    } else {
        Ok(ModelTier::Compact)
    }
}

fn memory_fit(artifact: ModelArtifact, usable_host_memory_mib: u64) -> MemoryFit {
    let artifact_memory_mib = artifact.download_bytes.div_ceil(MIB);
    let runtime_headroom_mib = RUNTIME_BASE_MIB + artifact_memory_mib.div_ceil(4);
    MemoryFit {
        execution_target: "cpu_host_memory",
        usable_host_memory_mib,
        artifact_memory_mib,
        runtime_headroom_mib,
        required_host_memory_mib: artifact_memory_mib + runtime_headroom_mib,
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

pub(crate) fn catalog(use_case: ModelUseCase) -> &'static [ModelArtifact] {
    match use_case {
        ModelUseCase::Writing => &WRITING_MODELS,
        ModelUseCase::Code => &CODE_MODELS,
    }
}

fn candidate_rationale(
    hardware: &HardwareProfile,
    artifact: ModelArtifact,
    fit: MemoryFit,
) -> Vec<String> {
    let mut reasons = vec![format!(
        "The {} MiB artifact plus {} MiB conservative runtime headroom fits within {} MiB usable host memory.",
        fit.artifact_memory_mib, fit.runtime_headroom_mib, fit.usable_host_memory_mib
    )];
    if let Some(useful) = artifact.screen_useful {
        reasons.push(format!(
            "{} gave the most useful suggestions of the models that fit: {useful} of 135 in the 2026-10-17 blind development screen.",
            artifact.name
        ));
    }
    if hardware.architecture == "aarch64"
        || (hardware.architecture == "x86_64" && !hardware.cpu.avx2)
    {
        reasons.push(
            "CPU compatibility is unbenchmarked, so the candidate is capped at compact.".to_owned(),
        );
    }
    if hardware.on_battery == Some(true) {
        reasons.push("Battery discharge caps the candidate at compact.".to_owned());
    } else if hardware.on_battery.is_none() {
        reasons
            .push("Power state is unknown, so the candidate is capped below quality.".to_owned());
    }
    if hardware.gpu.hybrid {
        reasons.push(
            "Multiple GPU vendors are present, so ambiguous acceleration is ignored.".to_owned(),
        );
    }
    if let Some(vram) = hardware.gpu.dedicated_memory_mib {
        reasons.push(format!(
            "Detected {vram} MiB total dedicated GPU memory, but no validated backend exposes usable GPU capacity."
        ));
    }
    reasons.push(format!(
        "{} {} remains a candidate only; its prompt contract, latency, memory use, and quality gates have not passed.",
        artifact.runtime, artifact.minimum_runtime_version
    ));
    reasons
}

fn no_fit_rationale(hardware: &HardwareProfile, reason: AdviceReason) -> Vec<String> {
    let detail = match reason {
        AdviceReason::MemoryCapacityUnknown => {
            "Total and currently available host memory are both required; at least one is unknown."
        }
        AdviceReason::MemoryCapacityInvalid => {
            "Reported host memory is internally inconsistent, so Badi will not guess."
        }
        AdviceReason::InsufficientUsableMemory => {
            "No artifact plus conservative runtime headroom fits after the host reserve."
        }
        AdviceReason::InsufficientCompute => {
            "Fewer than four logical CPUs is below the unvalidated inline-inference floor."
        }
        AdviceReason::UnsupportedArchitecture => {
            "This architecture has no reviewed Badi runtime compatibility baseline."
        }
        AdviceReason::CandidateFitsHostMemory => {
            "A candidate fit was expected but no recommendation was produced."
        }
    };
    let mut reasons = vec![detail.to_owned()];
    if let Some(vram) = hardware.gpu.dedicated_memory_mib {
        reasons.push(format!(
            "Detected {vram} MiB total dedicated GPU memory is not usable capacity without a validated backend."
        ));
    }
    reasons.push("No download is planned and runtime_ready remains false.".to_owned());
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

trait NvidiaMemoryProbe {
    fn detected_total_memory_mib(&self) -> Option<u64>;
}

struct SystemNvidiaMemoryProbe;

impl NvidiaMemoryProbe for SystemNvidiaMemoryProbe {
    fn detected_total_memory_mib(&self) -> Option<u64> {
        let mut command = Command::new("nvidia-smi");
        command.args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"]);
        let output = run_bounded_command(
            &mut command,
            NVIDIA_SMI_TIMEOUT,
            NVIDIA_SMI_MAX_STDOUT_BYTES,
        )?;
        if !output.status.success() || output.truncated {
            return None;
        }
        parse_nvidia_vram_mib(&output.stdout)
    }
}

#[cfg(feature = "local-model")]
struct NoNvidiaMemoryProbe;

#[cfg(feature = "local-model")]
impl NvidiaMemoryProbe for NoNvidiaMemoryProbe {
    fn detected_total_memory_mib(&self) -> Option<u64> {
        None
    }
}

fn detect_gpus_with(root: &Path, nvidia_probe: &dyn NvidiaMemoryProbe) -> GpuProfile {
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
                    record_detected_vram(&mut profile, bytes / MIB);
                }
            }
        }
    }
    if let Some(mib) = nvidia_probe.detected_total_memory_mib() {
        profile.nvidia = true;
        record_detected_vram(&mut profile, mib);
    }
    profile.hybrid = [profile.nvidia, profile.amd, profile.intel]
        .into_iter()
        .filter(|detected| *detected)
        .count()
        > 1;
    profile
}

fn record_detected_vram(profile: &mut GpuProfile, mib: u64) {
    profile.dedicated_memory_mib = Some(
        profile
            .dedicated_memory_mib
            .map_or(mib, |current| current.max(mib)),
    );
}

fn is_drm_card(name: &str) -> bool {
    name.strip_prefix("card").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

#[derive(Debug)]
struct BoundedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    truncated: bool,
}

fn run_bounded_command(
    command: &mut Command,
    timeout: Duration,
    max_stdout_bytes: usize,
) -> Option<BoundedOutput> {
    // This supervises and reaps the direct child. A descendant that inherits stdout could keep
    // the reader open after that child exits; nvidia-smi is invoked directly and is not expected
    // to create such descendants.
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let Some(stdout) = child.stdout.take() else {
        kill_and_reap(&mut child);
        return None;
    };
    let reader = thread::spawn(move || read_capped(stdout, max_stdout_bytes));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                kill_and_reap(&mut child);
                break None;
            }
        }
    };
    let captured = reader.join().ok()?.ok()?;
    Some(BoundedOutput {
        status: status?,
        stdout: captured.bytes,
        truncated: captured.truncated,
    })
}

fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[derive(Debug)]
struct CappedBytes {
    bytes: Vec<u8>,
    truncated: bool,
}

fn read_capped(mut reader: impl Read, limit: usize) -> io::Result<CappedBytes> {
    let mut bytes = Vec::with_capacity(limit.min(4_096));
    let mut buffer = [0_u8; 4_096];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let retained = count.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < count;
    }
    Ok(CappedBytes { bytes, truncated })
}

fn parse_nvidia_vram_mib(stdout: &[u8]) -> Option<u64> {
    std::str::from_utf8(stdout)
        .ok()?
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .max()
}

fn parse_battery_status(value: &str) -> Option<bool> {
    match value.trim() {
        "Discharging" => Some(true),
        "Charging" | "Full" | "Not charging" => Some(false),
        _ => None,
    }
}

fn parse_online(value: &str) -> bool {
    value.trim() == "1"
}

fn detect_on_battery(root: &Path) -> Option<bool> {
    let entries = fs::read_dir(root).ok()?;
    let mut found_battery = false;
    let mut unknown_battery = false;
    let mut online_mains = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(supply_type) = fs::read_to_string(path.join("type")) else {
            continue;
        };
        match supply_type.trim() {
            "Battery" => {
                found_battery = true;
                match fs::read_to_string(path.join("status"))
                    .ok()
                    .as_deref()
                    .and_then(parse_battery_status)
                {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => unknown_battery = true,
                }
            }
            "Mains" => {
                online_mains |=
                    fs::read_to_string(path.join("online")).is_ok_and(|value| parse_online(&value));
            }
            _ => {}
        }
    }
    if found_battery {
        (!unknown_battery).then_some(false)
    } else {
        online_mains.then_some(false)
    }
}

#[cfg(test)]
mod tests;
