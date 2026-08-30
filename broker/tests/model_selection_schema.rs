use std::fs;
use std::path::{Path, PathBuf};

use badi_broker::model_selection::{
    AdviceStatus, CpuFeatures, GpuProfile, HardwareProfile, MemoryProfile, ModelAdvice,
    ModelUseCase, recommend_model,
};
use jsonschema::Registry;
use serde_json::Value;

const HARDWARE_SCHEMA_ID: &str = "urn:badi:schema:hardware:v1";

fn schema_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn profile(total_mib: Option<u64>, available_mib: Option<u64>) -> HardwareProfile {
    HardwareProfile {
        schema: "badi.hardware.v1",
        architecture: "x86_64".to_owned(),
        logical_cpus: 12,
        cpu: CpuFeatures {
            avx2: true,
            avx512f: false,
        },
        memory: MemoryProfile {
            total_mib,
            available_mib,
        },
        gpu: GpuProfile::default(),
        on_battery: Some(false),
    }
}

fn assert_semantic_invariants(advice: &ModelAdvice) {
    assert!(!advice.runtime_ready);
    match advice.status {
        AdviceStatus::Candidate => {
            let recommended = advice.recommended.as_ref().expect("candidate artifact");
            let fit = advice.fit.expect("candidate fit");
            let download = advice.download.as_ref().expect("candidate download");
            assert_eq!(advice.tier, Some(recommended.tier));
            assert_eq!(advice.use_case, recommended.use_case);
            assert_eq!(
                fit.required_host_memory_mib,
                fit.artifact_memory_mib + fit.runtime_headroom_mib
            );
            assert!(fit.required_host_memory_mib <= fit.usable_host_memory_mib);
            let available = advice
                .hardware
                .memory
                .available_mib
                .expect("candidate available memory");
            assert!(fit.usable_host_memory_mib + 2_048 <= available);
            assert_eq!(download.expected_sha256, recommended.sha256);
            assert_eq!(
                download.arguments.as_slice(),
                &[
                    "download",
                    recommended.repository,
                    recommended.filename,
                    "--revision",
                    recommended.revision,
                ]
            );
            for alternative in &advice.alternatives {
                assert_eq!(alternative.use_case, advice.use_case);
                assert!(alternative.tier < recommended.tier);
                let artifact_memory_mib = alternative.download_bytes.div_ceil(1_048_576);
                let required_host_memory_mib =
                    artifact_memory_mib + 768 + artifact_memory_mib.div_ceil(4);
                assert!(required_host_memory_mib <= fit.usable_host_memory_mib);
            }
        }
        AdviceStatus::NoFit => {
            assert!(advice.tier.is_none());
            assert!(advice.recommended.is_none());
            assert!(advice.alternatives.is_empty());
            assert!(advice.fit.is_none());
            assert!(advice.download.is_none());
        }
    }
}

#[test]
fn representative_hardware_and_advice_outputs_match_their_formal_schemas() {
    let root = schema_root();
    let hardware_schema = read_json(&root.join("badi.hardware.v1.schema.json"));
    let advice_schema = read_json(&root.join("badi.model-advice.v2.schema.json"));
    let hardware_validator = jsonschema::validator_for(&hardware_schema).expect("hardware schema");
    let registry = Registry::new()
        .add(HARDWARE_SCHEMA_ID, hardware_schema)
        .expect("hardware schema resource")
        .prepare()
        .expect("hardware schema registry");
    let advice_validator = jsonschema::options()
        .with_registry(&registry)
        .build(&advice_schema)
        .expect("model-advice schema");

    let candidate = recommend_model(profile(Some(16_384), Some(8_192)), ModelUseCase::Writing);
    let no_fit = recommend_model(profile(Some(2_048), Some(512)), ModelUseCase::Code);
    let unknown = recommend_model(profile(None, None), ModelUseCase::Writing);

    for advice in [candidate, no_fit, unknown] {
        assert_semantic_invariants(&advice);
        let instance = serde_json::to_value(advice).expect("serialize advice");
        assert!(
            hardware_validator.is_valid(&instance["hardware"]),
            "embedded hardware must match badi.hardware.v1"
        );
        if let Err(error) = advice_validator.validate(&instance) {
            panic!("model advice failed schema: {error}");
        }
    }
}

#[test]
fn producer_preserves_cross_field_model_invariants_at_tier_boundaries() {
    for advice in [
        recommend_model(profile(Some(8_192), Some(4_000)), ModelUseCase::Writing),
        recommend_model(profile(Some(8_192), Some(4_500)), ModelUseCase::Code),
        recommend_model(profile(Some(16_384), Some(8_192)), ModelUseCase::Writing),
        recommend_model(profile(Some(32_768), Some(8_192)), ModelUseCase::Code),
        recommend_model(profile(Some(32_768), Some(20_000)), ModelUseCase::Code),
        recommend_model(profile(Some(2_048), Some(512)), ModelUseCase::Writing),
    ] {
        assert_semantic_invariants(&advice);
    }
}

#[test]
fn schema_rejects_runtime_readiness_and_mixed_candidate_states() {
    let root = schema_root();
    let hardware_schema = read_json(&root.join("badi.hardware.v1.schema.json"));
    let advice_schema = read_json(&root.join("badi.model-advice.v2.schema.json"));
    let registry = Registry::new()
        .add(HARDWARE_SCHEMA_ID, hardware_schema)
        .expect("hardware schema resource")
        .prepare()
        .expect("hardware schema registry");
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&advice_schema)
        .expect("model-advice schema");
    let advice = recommend_model(profile(Some(16_384), Some(8_192)), ModelUseCase::Writing);
    let mut instance = serde_json::to_value(advice).expect("serialize advice");

    instance["runtime_ready"] = Value::Bool(true);
    assert!(!validator.is_valid(&instance));

    instance["runtime_ready"] = Value::Bool(false);
    instance["status"] = Value::String("no_fit".to_owned());
    assert!(!validator.is_valid(&instance));
}

#[test]
fn hardware_schema_requires_usable_gpu_memory_and_backend_as_a_pair() {
    let schema = read_json(&schema_root().join("badi.hardware.v1.schema.json"));
    let validator = jsonschema::validator_for(&schema).expect("hardware schema");
    let mut instance =
        serde_json::to_value(profile(Some(16_384), Some(8_192))).expect("serialize hardware");

    instance["gpu"]["backend"] = Value::String("llama.cpp".to_owned());
    assert!(!validator.is_valid(&instance));

    instance["gpu"]["usable_memory_mib"] = Value::from(4_096);
    assert!(validator.is_valid(&instance));

    instance["gpu"]["backend"] = Value::Null;
    assert!(!validator.is_valid(&instance));
}
