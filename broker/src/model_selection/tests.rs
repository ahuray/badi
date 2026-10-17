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
    parse_nvidia_vram_mib, parse_online, recommend_model, recommended_tier, run_bounded_command,
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
    let memory =
        parse_meminfo("MemTotal:       16384000 kB\nMemFree: 100 kB\nMemAvailable: 8192000 kB\n");
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
            advice
                .fit
                .is_some_and(|fit| { fit.required_host_memory_mib <= fit.usable_host_memory_mib })
        );
        assert!(!advice.runtime_ready);
    }
}

#[test]
fn artifact_bytes_decide_each_use_case_on_its_own() {
    // 2100 MiB usable: Gemma 3 1B (769 + 961 MiB) fits, the 1.1 GB code
    // model (1066 + 1035 MiB) does not, and the 1.3 GB Qwen3 1.7B never did.
    let hardware = profile(8_192, 4_148, 8);
    let writing = recommend_model(hardware.clone(), ModelUseCase::Writing);
    assert_eq!(writing.tier, Some(ModelTier::Balanced));
    assert_eq!(
        writing.recommended.map(|model| model.id),
        Some("gemma-3-1b")
    );
    assert_eq!(
        recommend_model(hardware, ModelUseCase::Code).tier,
        Some(ModelTier::Compact)
    );
}

#[cfg(feature = "local-model")]
#[test]
fn installed_writing_model_survives_power_advice_changes() {
    let mut hardware = profile(16_384, 8_192, 20);
    hardware.on_battery = Some(true);
    let balanced = super::WRITING_MODELS[1];
    assert_eq!(recommended_tier(&hardware), ModelTier::Compact);
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[balanced.filename], None),
        Ok(Some(balanced))
    );
    hardware.on_battery = None;
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[balanced.filename], None),
        Ok(Some(balanced))
    );
}

#[cfg(feature = "local-model")]
#[test]
fn installed_selection_prefers_advice_then_smaller_artifacts() {
    let mut hardware = profile(16_384, 8_192, 20);
    let compact = super::WRITING_MODELS[0];
    let balanced = super::WRITING_MODELS[1];
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[compact.filename], None),
        Ok(Some(compact))
    );
    let installed = [compact.filename, balanced.filename];
    assert_eq!(
        super::select_installed_writing_model(&hardware, &installed, None),
        Ok(Some(balanced))
    );
    hardware.on_battery = Some(true);
    assert_eq!(
        super::select_installed_writing_model(&hardware, &installed, None),
        Ok(Some(compact))
    );
}

#[cfg(feature = "local-model")]
#[test]
fn installed_selection_uses_total_memory_and_keeps_compatibility_gates() {
    let balanced = super::WRITING_MODELS[1];
    // A momentary dip in available memory must not refuse the installation.
    let mut hardware = profile(15_663, 4_300, 20);
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[balanced.filename], None),
        Ok(Some(balanced))
    );
    hardware.memory.available_mib = None;
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[balanced.filename], None),
        Ok(Some(balanced))
    );
    let floor = profile(4_096, 4_000, 8);
    assert_eq!(
        super::select_installed_writing_model(&floor, &[balanced.filename], None),
        Err(AdviceReason::InsufficientUsableMemory)
    );
    hardware.memory.total_mib = None;
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[balanced.filename], None),
        Err(AdviceReason::MemoryCapacityUnknown)
    );
    hardware.memory.total_mib = Some(0);
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[balanced.filename], None),
        Err(AdviceReason::MemoryCapacityInvalid)
    );
    let mut hardware = profile(15_663, 8_000, 2);
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[balanced.filename], None),
        Err(AdviceReason::InsufficientCompute)
    );
    hardware.architecture = "riscv64".to_owned();
    hardware.logical_cpus = 20;
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[balanced.filename], None),
        Err(AdviceReason::UnsupportedArchitecture)
    );
}

#[cfg(feature = "local-model")]
#[test]
fn start_waits_until_the_model_and_host_reserve_fit_available_memory() {
    // 1224 MiB of weights, 768 + 306 MiB runtime headroom, 2048 MiB reserve.
    let balanced = super::WRITING_MODELS[1];
    assert!(super::fits_available_memory(balanced, 4_346));
    assert!(!super::fits_available_memory(balanced, 4_345));
    assert!(!super::fits_available_memory(balanced, 0));
    let compact = super::WRITING_MODELS[0];
    assert!(super::fits_available_memory(compact, 3_579));
    assert!(!super::fits_available_memory(compact, 3_578));
}

#[cfg(feature = "local-model")]
#[test]
fn installed_selection_never_invents_a_model_or_accepts_an_unknown_artifact() {
    let hardware = profile(16_384, 8_192, 20);
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[], None),
        Ok(None)
    );
    assert_eq!(
        super::select_installed_writing_model(&hardware, &["unreviewed.gguf"], None),
        Ok(None)
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
    assert_eq!(
        recommend_model(hardware.clone(), ModelUseCase::Code).tier,
        Some(ModelTier::Quality)
    );
    // Writing has no quality tier: a larger model screened no better.
    let writing = recommend_model(hardware, ModelUseCase::Writing);
    assert_eq!(
        writing.recommended.map(|model| model.id),
        Some("gemma-3-1b")
    );
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
fn broker_service_cap_holds_the_largest_writing_model() {
    let quality = super::WRITING_MODELS
        .iter()
        .max_by_key(|model| model.download_bytes)
        .expect("largest writing model");
    let fit = super::memory_fit(*quality, 16_000);
    let unit = include_str!("../../../packaging/systemd/badi-broker.service");
    assert!(unit.contains("ReadOnlyPaths=-%h/.local/share/badi/models"));
    let max = unit
        .lines()
        .find_map(|line| line.strip_prefix("MemoryMax="))
        .expect("memory cap");
    let max_mib: u64 = max.trim_end_matches('M').parse().expect("mebibytes");
    assert!(
        fit.required_host_memory_mib <= max_mib,
        "quality fit {} MiB exceeds MemoryMax {max_mib}",
        fit.required_host_memory_mib
    );
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
        assert!(model.sha256.len() == 64 && model.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(
            std::path::Path::new(model.filename)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
        );
        assert_eq!(std::path::Path::new(model.filename).components().count(), 1);
        assert!(["Apache-2.0", "Gemma", "Llama-3.2-Community"].contains(&model.license));
        assert!(!model.id.is_empty() && !model.name.is_empty() && !model.languages.is_empty());
        assert_eq!(model.runtime, "llama.cpp");
        assert_eq!(model.minimum_runtime_version, "b5092");
        assert_ne!(model.prompt_format, "");
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
    let output =
        run_bounded_command(&mut command, Duration::from_secs(1), 64).expect("bounded noisy probe");
    assert!(output.status.success());
    assert_eq!(output.stdout.len(), 64);
    assert!(output.truncated);
}

#[test]
fn the_recommendation_is_the_best_screened_model_that_fits() {
    let advice = recommend_model(profile(16_384, 8_192, 20), ModelUseCase::Writing);
    let recommended = advice.recommended.expect("a writing model fits");
    assert_eq!(recommended.id, "gemma-3-1b");
    assert!(
        advice
            .catalog
            .iter()
            .all(|choice| choice.fits || choice.artifact.tier > ModelTier::Balanced)
    );
    assert_eq!(advice.catalog.len(), super::WRITING_MODELS.len());
    // On battery only the compact model is advised, whatever its score.
    let mut battery = profile(16_384, 8_192, 20);
    battery.on_battery = Some(true);
    let compact = recommend_model(battery, ModelUseCase::Writing);
    assert_eq!(
        compact.recommended.map(|model| model.id),
        Some("qwen3-0.6b")
    );
    assert!(compact.catalog.iter().any(|choice| !choice.fits));
}

#[cfg(feature = "local-model")]
#[test]
fn an_installed_chosen_model_wins_while_it_fits() {
    let hardware = profile(16_384, 8_192, 20);
    let [compact, qwen, llama, gemma] = super::WRITING_MODELS;
    let installed = [
        compact.filename,
        qwen.filename,
        llama.filename,
        gemma.filename,
    ];
    assert_eq!(
        super::select_installed_writing_model(&hardware, &installed, None),
        Ok(Some(gemma))
    );
    assert_eq!(
        super::select_installed_writing_model(&hardware, &installed, Some("llama-3.2-1b")),
        Ok(Some(llama))
    );
    // A choice that is not installed, or unknown, falls back to the preference.
    assert_eq!(
        super::select_installed_writing_model(&hardware, &[qwen.filename], Some("gemma-3-1b")),
        Ok(Some(qwen))
    );
    assert_eq!(
        super::select_installed_writing_model(&hardware, &installed, Some("unknown")),
        Ok(Some(gemma))
    );
}
