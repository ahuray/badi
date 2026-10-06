use super::*;

fn fixture() -> AssessmentInput {
    AssessmentInput {
        candidate: CandidateMetadata {
            id: "example/model:weights.gguf".to_owned(),
            revision: Some("a".repeat(40)),
            sha256: Some("b".repeat(64)),
            artifact_bytes: Some(256 * MIB),
            tokenizer_identity: Some(format!("sha256:{}", "c".repeat(64))),
            quantization: Some("Q8_0".to_owned()),
            architecture: Some("llama".to_owned()),
            context_length: Some(8192),
            languages: vec!["en".to_owned(), "de".to_owned(), "fa".to_owned()],
            format: "gguf".to_owned(),
            access: "public".to_owned(),
            license: Some("apache-2.0".to_owned()),
            state: StateRequirements::Transformer {
                layers: 32,
                kv_heads: 8,
                key_length: 64,
                value_length: 64,
            },
        },
        device: DeviceInspection {
            schema: "badi.device-inspection.v1".to_owned(),
            inspected_at_unix_s: 1000,
            fingerprint: "d".repeat(64),
            architecture: "x86_64".to_owned(),
            cpu: CpuInspection {
                model: Some("fixture CPU".to_owned()),
                logical_cpus: 8,
                physical_cores: Some(4),
                packages: Some(1),
                features: vec!["avx2".to_owned()],
            },
            total_memory_bytes: Some(16 * 1024 * MIB),
            available_memory_bytes: Some(8 * 1024 * MIB),
            gpus: Vec::new(),
            disk: DiskInspection {
                path: "/tmp".to_owned(),
                total_bytes: Some(100 * 1024 * MIB),
                available_bytes: Some(20 * 1024 * MIB),
            },
            on_battery: Some(false),
            power_profile: Some("balanced".to_owned()),
            memory_pressure: None,
            limitations: Vec::new(),
        },
        settings: QualificationSettings {
            runtime_identity: "e".repeat(64),
            runtime_version: "b10726".to_owned(),
            runtime_architecture: "x86_64".to_owned(),
            required_cpu_features: Vec::new(),
            backend: "cpu".to_owned(),
            context_tokens: 2048,
            batch_tokens: 16,
            threads: 4,
            parallel_sequences: 1,
            cache_element_bytes: 2,
            recurrent_snapshots: 0,
            context_checkpoints: 0,
            prompt_format: "native-prefix-v1".to_owned(),
            generation_settings_sha256: "f".repeat(64),
            languages: vec!["en".to_owned(), "de".to_owned(), "fa".to_owned()],
            evaluation_version: EVALUATION_VERSION.to_owned(),
        },
        evidence: None,
        now_unix_s: 1000,
    }
}

fn attach_good_evidence(input: &mut AssessmentInput) {
    input.evidence = Some(QualificationEvidence {
        identity: evidence_identity(&input.candidate, &input.device, &input.settings),
        measured_at_unix_s: 999,
        loaded_and_exercised: true,
        actual_backend: "cpu".to_owned(),
        artifact_verified: true,
        cold_start_ms: 2000,
        preparation_ms: 400,
        peak_rss_bytes: 800 * MIB,
        peak_vram_bytes: None,
        sustained_seconds: 1800,
        cancellation_recovery_passed: true,
        cleanup_verified: true,
        prompt_reuse_measured: true,
        paced_typing_measured: true,
        confirmation_set_sha256: "1".repeat(64),
        confirmation_untouched: true,
        review_protocol_sha256: "2".repeat(64),
        languages: input
            .settings
            .languages
            .iter()
            .map(|language| LanguageMeasurement {
                language: language.clone(),
                total: 40,
                reviewed: 40,
                useful_on_time: 28,
                harmful: 0,
                generic_or_incorrect: 6,
                abstained_or_failed: 6,
                p50_complete_word_ms: 200,
                p95_complete_word_ms: 500,
            })
            .collect(),
    });
}

fn passed(report: &Assessment, name: &str) -> bool {
    report
        .stages
        .iter()
        .find(|stage| stage.state == name)
        .unwrap()
        .passed
}

#[test]
fn metadata_fit_never_conveys_exercise_quality_or_recommendation() {
    let input = fixture();
    let report = assess(&input);
    assert!(passed(&report, "estimated_fit"));
    assert!(!passed(&report, "loaded_and_exercised"));
    assert!(!report.recommended);
    assert_eq!(report.estimate.state_bytes, Some(128 * MIB));
    assert_eq!(rank(&[report]).status, "no_qualified_model");
}

#[test]
fn complete_valid_local_evidence_qualifies_each_language() {
    let mut input = fixture();
    attach_good_evidence(&mut input);
    let report = assess(&input);
    assert!(report.recommended, "{report:#?}");
    assert!(report.stages.iter().all(|stage| stage.passed));
    assert_eq!(rank(&[report]).candidate_id, Some(input.candidate.id));
}

#[test]
fn successful_exercise_remains_distinct_from_missing_quality_measurements() {
    let mut input = fixture();
    attach_good_evidence(&mut input);
    input.evidence.as_mut().unwrap().languages.clear();
    let report = assess(&input);
    assert!(report.evidence_valid);
    assert!(passed(&report, "loaded_and_exercised"));
    assert!(!passed(&report, "meets_performance"));
    assert!(!passed(&report, "meets_prediction_quality"));
    assert!(!report.recommended);
}

#[test]
fn shared_and_unverified_gpu_memory_never_increase_cpu_capacity() {
    let mut input = fixture();
    input.device.available_memory_bytes = Some(2300 * MIB);
    input.device.gpus.push(GpuInspection {
        id: "card0".to_owned(),
        vendor: "intel".to_owned(),
        device: Some("shared GPU".to_owned()),
        driver: Some("i915".to_owned()),
        memory_kind: "shared".to_owned(),
        total_bytes: Some(16 * 1024 * MIB),
        available_bytes: Some(12 * 1024 * MIB),
        execution_verified: false,
    });
    assert!(!passed(&assess(&input), "estimated_fit"));
    input.device.available_memory_bytes = Some(8 * 1024 * MIB);
    input.settings.backend = "vulkan".to_owned();
    attach_good_evidence(&mut input);
    assert!(!assess(&input).recommended);
}

#[test]
fn cross_compiled_runtime_and_missing_instructions_do_not_fit() {
    let mut input = fixture();
    input.settings.runtime_architecture = "aarch64".to_owned();
    assert!(!passed(&assess(&input), "estimated_fit"));
    input.settings.runtime_architecture = "x86_64".to_owned();
    input.settings.required_cpu_features = vec!["avx512f".to_owned()];
    assert!(!passed(&assess(&input), "estimated_fit"));
    input.settings.required_cpu_features = vec!["avx2".to_owned()];
    assert!(passed(&assess(&input), "estimated_fit"));
}

#[test]
fn unknown_recurrent_and_overflow_metadata_fail_closed() {
    let mut input = fixture();
    for state in [
        StateRequirements::Unknown,
        StateRequirements::Recurrent,
        StateRequirements::Transformer {
            layers: u64::MAX,
            kv_heads: 8,
            key_length: 64,
            value_length: 64,
        },
    ] {
        input.candidate.state = state;
        let report = assess(&input);
        assert_eq!(report.estimate.state_bytes, None);
        assert!(!passed(&report, "estimated_fit"));
    }
    input = fixture();
    input.candidate.architecture = Some("qwen35".to_owned());
    assert_eq!(assess(&input).estimate.state_bytes, None);
}

#[test]
fn lfm2_counts_attention_and_shortconv_state_with_bound_runtime_settings() {
    let mut input = fixture();
    input.candidate.architecture = Some("lfm2".to_owned());
    input.candidate.state = StateRequirements::Lfm2 {
        attention_layers: 6,
        convolution_layers: 10,
        kv_heads: 8,
        key_length: 64,
        value_length: 64,
        embedding_length: 1024,
        convolution_cache_length: 3,
    };
    let expected = 24 * MIB + 10 * 1024 * 2 * 4 + 10 * 4096;
    assert_eq!(assess(&input).estimate.state_bytes, Some(expected));
    input.settings.recurrent_snapshots = 2;
    input.settings.context_checkpoints = 2;
    assert_eq!(
        assess(&input).estimate.state_bytes,
        Some((24 * MIB + 10 * 1024 * 2 * 4 * 3 + 10 * 4096) * 3)
    );
    input.settings.runtime_version = "unreviewed".to_owned();
    assert_eq!(assess(&input).estimate.state_bytes, None);
}

#[test]
fn evidence_invalidates_model_tokenizer_revision_runtime_and_generation_changes() {
    let mut original = fixture();
    attach_good_evidence(&mut original);
    let mutations: [fn(&mut AssessmentInput); 11] = [
        |input| input.candidate.revision = Some("3".repeat(40)),
        |input| input.candidate.sha256 = Some("4".repeat(64)),
        |input| input.candidate.tokenizer_identity = Some(format!("sha256:{}", "5".repeat(64))),
        |input| input.candidate.quantization = Some("Q4_K_M".to_owned()),
        |input| input.device.cpu.model = Some("changed CPU".to_owned()),
        |input| input.settings.runtime_identity = "7".repeat(64),
        |input| input.settings.generation_settings_sha256 = "8".repeat(64),
        |input| input.settings.context_tokens = 4096,
        |input| input.settings.threads = 2,
        |input| {
            input.settings.languages.pop();
        },
        |input| input.settings.evaluation_version = "old".to_owned(),
    ];
    for mutation in mutations {
        let mut changed = original.clone();
        mutation(&mut changed);
        let report = assess(&changed);
        assert!(!report.evidence_valid);
        assert!(!report.recommended);
    }
}

#[test]
fn quality_cannot_be_bought_with_speed_or_larger_languages() {
    let mut input = fixture();
    attach_good_evidence(&mut input);
    let evidence = input.evidence.as_mut().unwrap();
    evidence.languages[0].p50_complete_word_ms = 10;
    evidence.languages[0].p95_complete_word_ms = 20;
    evidence.languages[0].harmful = 1;
    evidence.languages[0].generic_or_incorrect -= 1;
    assert!(!passed(&assess(&input), "meets_prediction_quality"));
    let evidence = input.evidence.as_mut().unwrap();
    evidence.languages[0].harmful = 0;
    evidence.languages[0].generic_or_incorrect += 1;
    evidence.languages[2].useful_on_time = 23;
    evidence.languages[2].generic_or_incorrect += 5;
    assert!(!passed(&assess(&input), "meets_prediction_quality"));
}

#[test]
fn all_assigned_cases_missing_outcomes_and_duplicates_are_gates() {
    let mut input = fixture();
    attach_good_evidence(&mut input);
    input.evidence.as_mut().unwrap().languages[0].abstained_or_failed = 0;
    assert!(!assess(&input).evidence_valid);
    attach_good_evidence(&mut input);
    input.evidence.as_mut().unwrap().languages[1].language = "en".to_owned();
    assert!(!assess(&input).evidence_valid);
    attach_good_evidence(&mut input);
    input.evidence.as_mut().unwrap().languages[0].reviewed = 39;
    assert!(!assess(&input).recommended);
}

#[test]
fn late_complete_words_diagnostics_and_unexercised_lifecycle_never_qualify() {
    let mut input = fixture();
    attach_good_evidence(&mut input);
    input.evidence.as_mut().unwrap().languages[2].p95_complete_word_ms = 551;
    assert!(!passed(&assess(&input), "meets_performance"));
    attach_good_evidence(&mut input);
    input.evidence.as_mut().unwrap().sustained_seconds = 1799;
    assert!(!assess(&input).recommended);
    attach_good_evidence(&mut input);
    input.evidence.as_mut().unwrap().paced_typing_measured = false;
    assert!(!assess(&input).recommended);
    attach_good_evidence(&mut input);
    input.evidence.as_mut().unwrap().cleanup_verified = false;
    assert!(!passed(&assess(&input), "loaded_and_exercised"));
}

#[test]
fn refresh_resources_and_expire_evidence_even_for_previously_good_candidate() {
    let mut input = fixture();
    attach_good_evidence(&mut input);
    input.device.available_memory_bytes = Some(1000 * MIB);
    assert!(!assess(&input).recommended);
    input = fixture();
    attach_good_evidence(&mut input);
    input.now_unix_s += 61;
    assert!(!passed(&assess(&input), "estimated_fit"));
    input.now_unix_s += 31 * 24 * 3600;
    input.device.inspected_at_unix_s = input.now_unix_s;
    assert!(!assess(&input).evidence_valid);
}

#[test]
fn ranking_is_quality_before_speed_with_no_qualified_fallback() {
    let mut accurate = fixture();
    attach_good_evidence(&mut accurate);
    accurate.candidate.id = "accurate".to_owned();
    attach_good_evidence(&mut accurate);
    let mut faster = accurate.clone();
    faster.candidate.id = "faster".to_owned();
    attach_good_evidence(&mut faster);
    for row in &mut faster.evidence.as_mut().unwrap().languages {
        row.p50_complete_word_ms = 20;
        row.p95_complete_word_ms = 50;
        row.useful_on_time -= 1;
        row.generic_or_incorrect += 1;
    }
    let recommendation = rank(&[assess(&faster), assess(&accurate)]);
    assert_eq!(recommendation.ranking, ["accurate", "faster"]);
    assert_eq!(rank(&[]).status, "no_qualified_model");
}

#[test]
fn request_contract_rejects_unknown_fields_and_round_trips() {
    let input = fixture();
    let json = serde_json::to_value(&input).unwrap();
    let roundtrip: AssessmentInput = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(roundtrip, input);
    let mut changed = json;
    changed["settings"]["allow_remote_inference"] = serde_json::json!(true);
    assert!(serde_json::from_value::<AssessmentInput>(changed).is_err());
}
