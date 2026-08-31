use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

const MIB: u64 = 1_048_576;
const HOST_RESERVE_MIB: u64 = 2_048;
const RUNTIME_BASE_MIB: u64 = 768;
const HARDWARE_SCHEMA: &str = "badi.hardware.v1";
const MODEL_ADVICE_SCHEMA: &str = "badi.model-advice.v2";
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DownloadPlan {
    pub tool: &'static str,
    pub arguments: Vec<&'static str>,
    pub expected_sha256: &'static str,
}

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
}

const WRITING_PROMPT_FORMAT: &str = "llama_cpp_native_prefix_completion_v1";
const WRITING_RUNTIME_CAVEAT: &str =
    "Native-prefix context bounds, latency, and inline quality are not validated by Badi.";
const CODE_PROMPT_FORMAT: &str = "qwen2_5_coder_instruct_chat_template";
const CODE_RUNTIME_CAVEAT: &str = "An instruct GGUF is not proof of fill-in-the-middle quality; latency and inline quality are not validated by Badi.";

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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: WRITING_PROMPT_FORMAT,
        runtime_caveat: WRITING_RUNTIME_CAVEAT,
    },
    ModelArtifact {
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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: WRITING_PROMPT_FORMAT,
        runtime_caveat: WRITING_RUNTIME_CAVEAT,
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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: CODE_PROMPT_FORMAT,
        runtime_caveat: CODE_RUNTIME_CAVEAT,
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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: CODE_PROMPT_FORMAT,
        runtime_caveat: CODE_RUNTIME_CAVEAT,
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
        minimum_runtime_version: LLAMA_CPP_MINIMUM_VERSION,
        prompt_format: CODE_PROMPT_FORMAT,
        runtime_caveat: CODE_RUNTIME_CAVEAT,
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
    match select_model(&hardware, use_case) {
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
        },
    }
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
    let catalog = catalog(use_case);
    let recommended = catalog
        .iter()
        .rev()
        .copied()
        .find(|artifact| {
            artifact.tier <= ceiling
                && memory_fit(*artifact, usable_host_memory_mib).required_host_memory_mib
                    <= usable_host_memory_mib
        })
        .ok_or(AdviceReason::InsufficientUsableMemory)?;
    let alternatives = catalog
        .iter()
        .copied()
        .filter(|artifact| {
            artifact.tier < recommended.tier
                && memory_fit(*artifact, usable_host_memory_mib).required_host_memory_mib
                    <= usable_host_memory_mib
        })
        .collect();
    Ok(Selection {
        recommended,
        alternatives,
        fit: memory_fit(recommended, usable_host_memory_mib),
    })
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

pub(crate) fn catalog(use_case: ModelUseCase) -> &'static [ModelArtifact; 3] {
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

fn detect_gpus(root: &Path) -> GpuProfile {
    detect_gpus_with(root, &SystemNvidiaMemoryProbe)
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
    // to create such descendants. A future general-purpose runner would need process-group control.
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
mod tests {
    use std::fs;
    #[cfg(target_os = "linux")]
    use std::path::Path;
    use std::process::Command;
    use std::time::Duration;
    #[cfg(target_os = "linux")]
    use std::time::Instant;

    use super::{
        AdviceReason, AdviceStatus, CODE_MODELS, CpuFeatures, GpuProfile, HARDWARE_SCHEMA,
        HardwareProfile, MemoryProfile, ModelTier, ModelUseCase, NvidiaMemoryProbe, WRITING_MODELS,
        detect_gpus_with, detect_on_battery, parse_battery_status, parse_meminfo,
        parse_nvidia_vram_mib, parse_online, recommend_model, recommended_tier,
        run_bounded_command,
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
    fn current_class_machine_stays_balanced_for_both_use_cases() {
        let mut hardware = profile(15_663, 10_482, 20);
        hardware.on_battery = None;
        for use_case in [ModelUseCase::Writing, ModelUseCase::Code] {
            let advice = recommend_model(hardware.clone(), use_case);
            assert_eq!(advice.status, AdviceStatus::Candidate);
            assert_eq!(advice.tier, Some(ModelTier::Balanced));
            assert!(
                advice.fit.is_some_and(|fit| {
                    fit.required_host_memory_mib <= fit.usable_host_memory_mib
                })
            );
            assert!(!advice.runtime_ready);
        }
    }

    #[test]
    fn artifact_bytes_can_downgrade_one_use_case_before_another() {
        let hardware = profile(8_192, 4_300, 8);
        assert_eq!(
            recommend_model(hardware.clone(), ModelUseCase::Writing).tier,
            Some(ModelTier::Compact)
        );
        assert_eq!(
            recommend_model(hardware, ModelUseCase::Code).tier,
            Some(ModelTier::Balanced)
        );
    }

    #[test]
    fn constrained_profiles_downgrade_or_return_no_fit() {
        let low_memory = recommend_model(profile(2_500, 900, 8), ModelUseCase::Writing);
        assert_eq!(low_memory.status, AdviceStatus::NoFit);
        assert_eq!(low_memory.reason, AdviceReason::InsufficientUsableMemory);
        assert!(low_memory.recommended.is_none());
        assert!(low_memory.download.is_none());

        let mut unknown = profile(16_384, 8_192, 8);
        unknown.memory.available_mib = None;
        let unknown = recommend_model(unknown, ModelUseCase::Writing);
        assert_eq!(unknown.status, AdviceStatus::NoFit);
        assert_eq!(unknown.reason, AdviceReason::MemoryCapacityUnknown);

        let mut unsupported = profile(16_384, 8_192, 8);
        unsupported.architecture = "riscv64".to_owned();
        let unsupported = recommend_model(unsupported, ModelUseCase::Code);
        assert_eq!(unsupported.status, AdviceStatus::NoFit);
        assert_eq!(unsupported.reason, AdviceReason::UnsupportedArchitecture);
    }

    #[test]
    fn arm_non_avx2_battery_and_hybrid_profiles_are_capped() {
        let mut arm = profile(32_768, 20_000, 16);
        arm.architecture = "aarch64".to_owned();
        assert_eq!(recommended_tier(&arm), ModelTier::Compact);

        let mut no_avx2 = profile(32_768, 20_000, 16);
        no_avx2.cpu.avx2 = false;
        assert_eq!(recommended_tier(&no_avx2), ModelTier::Compact);

        let mut battery = profile(32_768, 20_000, 16);
        battery.on_battery = Some(true);
        assert_eq!(recommended_tier(&battery), ModelTier::Compact);

        let mut hybrid = profile(32_768, 20_000, 16);
        hybrid.gpu.hybrid = true;
        assert_eq!(recommended_tier(&hybrid), ModelTier::Balanced);
    }

    #[test]
    fn quality_requires_known_ac_power_and_substantial_host_headroom() {
        let hardware = profile(32_768, 20_000, 16);
        assert_eq!(recommended_tier(&hardware), ModelTier::Quality);
        for use_case in [ModelUseCase::Writing, ModelUseCase::Code] {
            assert_eq!(
                recommend_model(hardware.clone(), use_case).tier,
                Some(ModelTier::Quality)
            );
        }
    }

    #[test]
    fn live_available_memory_always_preserves_the_host_reserve() {
        let advice = recommend_model(profile(32_768, 8_192, 16), ModelUseCase::Code);
        assert_eq!(advice.status, AdviceStatus::Candidate);
        assert_eq!(advice.tier, Some(ModelTier::Balanced));
        let fit = advice.fit.expect("balanced candidate fit");
        assert_eq!(fit.usable_host_memory_mib, 8_192 - super::HOST_RESERVE_MIB);
        assert!(fit.required_host_memory_mib <= fit.usable_host_memory_mib);
        assert_eq!(8_192 - fit.usable_host_memory_mib, super::HOST_RESERVE_MIB);
    }

    #[test]
    fn power_status_parser_is_exact_and_tri_state() {
        assert_eq!(parse_battery_status("Discharging\n"), Some(true));
        for status in ["Charging", "Full", "Not charging"] {
            assert_eq!(parse_battery_status(status), Some(false));
        }
        for status in ["Unknown", "", "discharging", "Not Charging"] {
            assert_eq!(parse_battery_status(status), None);
        }
        assert!(parse_online("1\n"));
        assert!(!parse_online("0\n"));
        assert!(!parse_online("unknown"));
    }

    #[test]
    fn power_supply_detector_does_not_turn_unknown_battery_state_into_ac() {
        let root = tempfile::tempdir().expect("temporary power-supply root");
        let battery = root.path().join("BAT0");
        fs::create_dir(&battery).expect("battery directory");
        fs::write(battery.join("type"), "Battery\n").expect("battery type");
        let mains = root.path().join("AC");
        fs::create_dir(&mains).expect("mains directory");
        fs::write(mains.join("type"), "Mains\n").expect("mains type");
        fs::write(mains.join("online"), "1\n").expect("mains online");

        assert_eq!(detect_on_battery(root.path()), None);
        fs::write(battery.join("status"), "Unknown\n").expect("unknown status");
        assert_eq!(detect_on_battery(root.path()), None);
        fs::remove_file(battery.join("status")).expect("remove unknown status");
        fs::create_dir(battery.join("status")).expect("unreadable status path");
        assert_eq!(detect_on_battery(root.path()), None);
    }

    #[test]
    fn power_supply_detector_accepts_only_explicit_power_states() {
        for (status, expected) in [
            ("Discharging", Some(true)),
            ("Charging", Some(false)),
            ("Full", Some(false)),
            ("Not charging", Some(false)),
        ] {
            let root = tempfile::tempdir().expect("temporary power-supply root");
            let battery = root.path().join("BAT0");
            fs::create_dir(&battery).expect("battery directory");
            fs::write(battery.join("type"), "Battery\n").expect("battery type");
            fs::write(battery.join("status"), status).expect("battery status");
            assert_eq!(detect_on_battery(root.path()), expected);
        }

        let root = tempfile::tempdir().expect("temporary mains root");
        let mains = root.path().join("AC");
        fs::create_dir(&mains).expect("mains directory");
        fs::write(mains.join("type"), "Mains\n").expect("mains type");
        fs::write(mains.join("online"), "1\n").expect("mains online");
        assert_eq!(detect_on_battery(root.path()), Some(false));
        fs::write(mains.join("online"), "0\n").expect("mains offline");
        assert_eq!(detect_on_battery(root.path()), None);
    }

    #[test]
    fn use_case_selects_a_pinned_artifact_and_non_executing_download_plan() {
        let advice = recommend_model(profile(8_192, 4_500, 8), ModelUseCase::Code);
        assert_eq!(advice.tier, Some(ModelTier::Balanced));
        assert_eq!(
            advice.recommended.expect("candidate").repository,
            "Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF"
        );
        assert_eq!(advice.download.expect("download plan").tool, "hf");
        assert!(!advice.runtime_ready);
    }

    #[test]
    fn catalog_metadata_is_pinned_and_records_runtime_constraints() {
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
            assert_eq!(model.runtime, "llama.cpp");
            assert_eq!(model.minimum_runtime_version, "b5092");
            assert!(!model.prompt_format.is_empty());
            assert!(model.runtime_caveat.contains("not validated by Badi"));
        }
    }

    #[test]
    fn balanced_writing_candidate_matches_the_qualified_artifact() {
        let model = WRITING_MODELS[1];
        assert_eq!(model.tier, ModelTier::Balanced);
        assert_eq!(model.repository, "ggml-org/Qwen3-1.7B-GGUF");
        assert_eq!(model.revision, "daeb8e2d528a760970442092f6bf1e55c3b659eb");
        assert_eq!(model.filename, "Qwen3-1.7B-Q4_K_M.gguf");
        assert_eq!(model.download_bytes, 1_282_439_264);
        assert_eq!(model.prompt_format, "llama_cpp_native_prefix_completion_v1");
        assert_eq!(
            model.sha256,
            "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5"
        );
    }

    struct FixedNvidiaProbe(Option<u64>);

    impl NvidiaMemoryProbe for FixedNvidiaProbe {
        fn detected_total_memory_mib(&self) -> Option<u64> {
            self.0
        }
    }

    #[test]
    fn gpu_detection_uses_injected_probe_without_claiming_usable_capacity() {
        let root = tempfile::tempdir().expect("temporary DRM root");
        let device = root.path().join("card0/device");
        fs::create_dir_all(&device).expect("DRM device directory");
        fs::write(device.join("vendor"), "0x8086\n").expect("DRM vendor");
        let profile = detect_gpus_with(root.path(), &FixedNvidiaProbe(Some(8_192)));
        assert!(profile.nvidia);
        assert!(profile.intel);
        assert!(profile.hybrid);
        assert_eq!(profile.dedicated_memory_mib, Some(8_192));
        assert_eq!(profile.usable_memory_mib, None);
        assert_eq!(profile.backend, None);
    }

    #[test]
    fn nvidia_output_parser_rejects_noise_and_selects_largest_valid_value() {
        assert_eq!(parse_nvidia_vram_mib(b"4096\n8192\n"), Some(8_192));
        assert_eq!(parse_nvidia_vram_mib(b"not memory\n"), None);
        assert_eq!(parse_nvidia_vram_mib(&[0xff]), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_command_kills_and_reaps_a_blocking_direct_child() {
        let root = tempfile::tempdir().expect("temporary probe root");
        let pid_path = root.path().join("probe.pid");
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf '%s\\n' \"$$\" > \"$1\"; while :; do :; done",
            "badi-probe",
            pid_path.to_str().expect("UTF-8 temporary path"),
        ]);

        let started = Instant::now();
        assert!(
            run_bounded_command(&mut command, Duration::from_millis(250), 64).is_none(),
            "a timed-out probe must not produce output"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "direct-child timeout must remain bounded"
        );

        let pid = fs::read_to_string(&pid_path)
            .expect("probe PID")
            .trim()
            .parse::<u32>()
            .expect("numeric probe PID");
        assert!(
            !Path::new("/proc").join(pid.to_string()).exists(),
            "wait must reap the killed direct child"
        );
    }

    #[test]
    fn bounded_command_caps_and_marks_noisy_stdout() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf '%01000d' 0"]);
        let output = run_bounded_command(&mut command, Duration::from_secs(1), 64)
            .expect("bounded noisy probe");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 64);
        assert!(output.truncated);
    }
}
