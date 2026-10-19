use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io;
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustix::process::{Pid, PidfdFlags, Signal};
use sha2::{Digest, Sha256};

use super::{
    CompletionProvider as _, FixtureBehavior, LlamaCppLaunch, OwnedRuntime, ProvenanceError,
    RuntimeAddress, RuntimeError, SecretToken, SemanticClient, SemanticClientConfig,
    StableRuntimeIdentity, WritingProfile, terminate_child, terminate_owned_child_with,
};
use crate::semantic::provenance::{
    DirectoryManifestExpectation, FileExpectation, VerifiedFile, directory_manifest_sha256,
    verify_directory_manifest, verify_file,
};

const TEST_PROFILE: WritingProfile = WritingProfile {
    launch_contract_id: "badi.test.writing-profile.v1",
    context_size: 1_024,
    batch_size: 64,
    context_checkpoints: Some(0),
    memory_map: false,
    full_swa_cache: false,
    model_origin: Some("test_artifact"),
};

fn fixture_launch(directory: &Path) -> Result<LlamaCppLaunch, Box<dyn Error>> {
    let binary = directory.join("runtime");
    let model = directory.join("model");
    fs::write(&binary, b"fixture runtime")?;
    fs::write(&model, b"fixture model")?;
    Ok(LlamaCppLaunch::for_fixture(
        verify_observed_file(&binary)?,
        verify_observed_file(&model)?,
        FixtureBehavior::Ready,
    )?)
}

fn launch_environment(launch: &LlamaCppLaunch) -> BTreeMap<String, String> {
    let mut command = Command::new("/bin/true");
    launch.configure_mode(&mut command);
    command
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?.to_owned(), value?.to_str()?.to_owned())))
        .collect()
}

#[test]
fn a_writing_profile_binds_its_launch_and_identity() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let launch = fixture_launch(temporary.path())?;
    let production = launch.clone();
    assert_eq!(
        launch
            .clone()
            .with_writing_profile(WritingProfile::PRODUCTION)?
            .identity(),
        production.identity()
    );
    let profiled = launch.clone().with_writing_profile(TEST_PROFILE)?;
    let identity = profiled.identity();
    assert_eq!(identity.launch_contract_id, "badi.test.writing-profile.v1");
    assert_eq!(identity.context_size, 1_024);
    assert_eq!(
        (identity.batch_size, identity.ubatch_size),
        (Some(64), Some(64))
    );
    assert_eq!(identity.model_origin, Some("test_artifact"));
    // Only the profile's own fields differ from the production identity.
    let mut as_production = identity.clone();
    as_production.launch_contract_id = WritingProfile::PRODUCTION.launch_contract_id;
    as_production.context_size = super::CONTEXT_SIZE;
    as_production.batch_size = Some(16);
    as_production.ubatch_size = Some(16);
    as_production.model_origin = None;
    as_production.memory_map = WritingProfile::PRODUCTION.memory_map;
    as_production.full_swa_cache = WritingProfile::PRODUCTION.full_swa_cache;
    assert_eq!(as_production, production.identity());
    // An absent origin is omitted from the digest input, not serialized.
    let without_origin = launch
        .with_writing_profile(WritingProfile {
            model_origin: None,
            ..TEST_PROFILE
        })?
        .identity();
    assert!(
        serde_json::to_value(&without_origin)?
            .get("model_origin")
            .is_none()
    );
    assert_ne!(without_origin.sha256(), identity.sha256());

    for (selected, batch, checkpoints, memory_map, swa_full) in [
        (production, "16", None, "0", Some("1")),
        (profiled, "64", Some("0"), "0", None),
    ] {
        let environment = launch_environment(&selected);
        assert_eq!(environment["LLAMA_ARG_BATCH"], batch);
        assert_eq!(environment["LLAMA_ARG_UBATCH"], batch);
        assert_eq!(environment["LLAMA_ARG_CACHE_PROMPT"], "1");
        assert_eq!(environment["LLAMA_ARG_CACHE_RAM"], "128");
        assert_eq!(environment["LLAMA_ARG_CACHE_REUSE"], "16");
        assert_eq!(environment["LLAMA_ARG_MMAP"], memory_map);
        assert_eq!(
            environment
                .get("LLAMA_ARG_CTX_CHECKPOINTS")
                .map(String::as_str),
            checkpoints
        );
        assert_eq!(
            environment.get("LLAMA_ARG_SWA_FULL").map(String::as_str),
            swa_full
        );
    }
    Ok(())
}

#[test]
fn writing_profiles_outside_their_bounds_are_rejected() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let launch = fixture_launch(temporary.path())?;
    let long_contract: &'static str = "c".repeat(129).leak();
    let long_origin: &'static str = "o".repeat(65).leak();
    let contract = |launch_contract_id| WritingProfile {
        launch_contract_id,
        ..TEST_PROFILE
    };
    let context = |context_size| WritingProfile {
        context_size,
        ..TEST_PROFILE
    };
    let batch = |batch_size| WritingProfile {
        batch_size,
        ..TEST_PROFILE
    };
    let checkpoints = |context_checkpoints| WritingProfile {
        context_checkpoints,
        ..TEST_PROFILE
    };
    let origin = |model_origin| WritingProfile {
        model_origin,
        ..TEST_PROFILE
    };
    for (profile, field) in [
        (contract(""), "launch_contract_id"),
        (contract("two words"), "launch_contract_id"),
        (contract("badi.v1\n"), "launch_contract_id"),
        (contract(long_contract), "launch_contract_id"),
        (context(0), "context_size"),
        (context(8_193), "context_size"),
        (batch(0), "batch_size"),
        (batch(1_025), "batch_size"),
        (checkpoints(Some(65)), "context_checkpoints"),
        (origin(Some("")), "model_origin"),
        (origin(Some("Origin")), "model_origin"),
        (origin(Some("test-artifact")), "model_origin"),
        (origin(Some(long_origin)), "model_origin"),
    ] {
        assert!(
            matches!(
                launch.clone().with_writing_profile(profile),
                Err(RuntimeError::InvalidConfig(name)) if name == field
            ),
            "{profile:?}"
        );
    }
    let boundary = WritingProfile {
        launch_contract_id: &long_contract[1..],
        context_size: 8_192,
        batch_size: 8_192,
        context_checkpoints: Some(64),
        model_origin: Some(&long_origin[1..]),
        memory_map: true,
        full_swa_cache: true,
    };
    assert!(launch.with_writing_profile(boundary).is_ok());
    Ok(())
}

/// Installed runtimes and run provenance compare the identity digest, so
/// loading changes must be explicit in the serialized production identity.
#[test]
fn production_identity_serialization_is_pinned() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let launch = fixture_launch(temporary.path())?;
    assert_eq!(
        serde_json::to_string(&launch.identity())?,
        concat!(
            "{\"launch_contract_id\":\"badi.writing.completion-and-spelling.en-de-fa.v2\"",
            ",\"binary_sha256\":\"7c63829baa2f5451c23789c5085b4de0627e5208b6c4bb2483217998f8cc38f0\"",
            ",\"runtime_bundle_manifest_sha256\":null",
            ",\"model_sha256\":\"c7a3a8c7435ef8e4cf1ca2d261f7e09ca85de6cdb70f7c689a836680523a180c\"",
            ",\"model_size\":13,\"model_alias\":\"fixture-en-v1\",\"threads\":1",
            ",\"context_size\":512,\"gpu_layers\":0,\"batch_size\":16,\"ubatch_size\":16,\"memory_map\":false",
            ",\"full_swa_cache\":true}",
        )
    );
    Ok(())
}

#[test]
fn contained_launch_execs_through_the_parent_death_helper() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let root = fs::canonicalize(temporary.path())?;
    let bundle_path = root.join("runtime");
    fs::create_dir(&bundle_path)?;
    let binary_path = bundle_path.join("llama-server");
    fs::write(&binary_path, b"fixture")?;
    let model_path = root.join("model.gguf");
    fs::write(&model_path, b"model")?;
    let bundle = verify_directory_manifest(&DirectoryManifestExpectation::new(
        &bundle_path,
        directory_manifest_sha256(&bundle_path)?,
    )?)?;
    let binary = verify_observed_file(&binary_path)?;
    let model = verify_observed_file(&model_path)?;

    let direct = LlamaCppLaunch::new(binary.clone(), bundle, model.clone(), "fixture", 1)?;
    assert_eq!(direct.runtime_command()?.get_program(), binary_path);
    let command = direct.contained().runtime_command()?;
    assert_eq!(command.get_program(), std::env::current_exe()?);
    let arguments: Vec<_> = command.get_args().collect();
    assert_eq!(
        arguments,
        [
            std::ffi::OsStr::new(crate::semantic::process::EXEC_HELPER_FLAG),
            std::ffi::OsStr::new(&std::process::id().to_string()),
            binary_path.as_os_str(),
        ]
    );
    // The helper passes no arguments, so a fixture cannot be contained.
    let fixture = LlamaCppLaunch::for_fixture(binary, model, FixtureBehavior::Ready)?;
    assert!(matches!(
        fixture.contained().validate(),
        Err(RuntimeError::InvalidConfig("runtime_bundle"))
    ));
    Ok(())
}

#[test]
fn runtime_socket_stays_inside_a_private_directory() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt as _;

    let temporary = tempfile::tempdir()?;
    let open = temporary.path().join("open");
    fs::create_dir(&open)?;
    fs::set_permissions(&open, fs::Permissions::from_mode(0o755))?;
    assert!(matches!(
        super::prepare_runtime_socket(&open),
        Err(RuntimeError::InvalidConfig("runtime_dir"))
    ));
    let private = fs::canonicalize(temporary.path())?.join("private");
    fs::create_dir(&private)?;
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700))?;
    let live = private.join("llama-1.sock");
    let dead = private.join("llama-999999999.sock");
    let _live_listener = std::os::unix::net::UnixListener::bind(&live)?;
    std::os::unix::net::UnixListener::bind(&dead)?;
    let socket = super::prepare_runtime_socket(&private)?;
    assert_eq!(
        socket,
        private.join(format!("llama-{}.sock", std::process::id()))
    );
    assert!(!socket.exists());
    assert!(live.exists(), "a live process keeps its socket");
    assert!(!dead.exists(), "a dead process does not keep a socket");
    Ok(())
}

#[test]
fn shutdown_allows_a_busy_child_to_finish_its_term_cleanup() -> Result<(), Box<dyn Error>> {
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            "trap 'sleep 0.35; exit 0' TERM; printf r; while :; do sleep 10 & wait $!; done",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let mut ready = [0_u8; 1];
    child
        .stdout
        .as_mut()
        .expect("readiness pipe")
        .read_exact(&mut ready)?;
    assert_eq!(ready, *b"r");
    let status = terminate_child(&mut child)?;
    assert!(status.success(), "TERM cleanup was interrupted: {status}");
    Ok(())
}

#[tokio::test]
async fn post_spawn_bundle_mutation_is_rejected_and_child_is_reaped() -> Result<(), Box<dyn Error>>
{
    let temporary = tempfile::tempdir()?;
    let root = fs::canonicalize(temporary.path())?;
    let bundle_path = root.join("runtime");
    let model_directory = root.join("model");
    fs::create_dir(&bundle_path)?;
    fs::create_dir(&model_directory)?;
    let binary_path = bundle_path.join("llama-server");
    fs::write(
        &binary_path,
        b"#!/bin/sh\ntrap '' TERM\nwhile :; do /bin/sleep 1; done\n",
    )?;
    fs::set_permissions(&binary_path, fs::Permissions::from_mode(0o700))?;
    let library_path = bundle_path.join("libfixture.so");
    fs::write(&library_path, b"reviewed")?;
    let model_path = model_directory.join("fixture.gguf");
    fs::write(&model_path, b"model")?;

    let binary = verify_observed_file(&binary_path)?;
    let model = verify_observed_file(&model_path)?;
    let bundle_digest = directory_manifest_sha256(&bundle_path)?;
    let bundle = verify_directory_manifest(&DirectoryManifestExpectation::new(
        &bundle_path,
        bundle_digest,
    )?)?;
    let launch = LlamaCppLaunch::new(binary, bundle, model, "fixture", 1)?
        .with_runtime_directory(root.join("sockets"));
    let process_id = Arc::new(AtomicU32::new(0));
    let observed_process_id = Arc::clone(&process_id);
    let result = launch
        .spawn_with_post_spawn_hook(move |spawned_process_id| {
            observed_process_id.store(spawned_process_id, Ordering::SeqCst);
            let replacement = library_path.with_extension("tmp");
            fs::write(&replacement, b"tampered").expect("test mutation must succeed");
            fs::rename(&replacement, &library_path).expect("test mutation must succeed");
        })
        .await;
    assert!(matches!(
        result,
        Err(RuntimeError::Provenance(ProvenanceError::IdentityChanged))
    ));
    let process_id = process_id.load(Ordering::SeqCst);
    assert_ne!(process_id, 0);
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    Ok(())
}

#[test]
fn failed_termination_keeps_child_owned_for_retry() -> Result<(), Box<dyn Error>> {
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg("while :; do /bin/sleep 1; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let process_id = child.id();
    let mut child = Some(child);
    let failure = terminate_owned_child_with(&mut child, |_| {
        Err(io::Error::other("injected termination failure"))
    });
    assert!(failure.is_err());
    assert_eq!(
        child.as_ref().map(std::process::Child::id),
        Some(process_id)
    );

    terminate_owned_child_with(&mut child, terminate_child)?;
    assert!(child.is_none());
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    Ok(())
}

fn owned_runtime_at(endpoint: std::net::SocketAddr) -> Result<OwnedRuntime, Box<dyn Error>> {
    let child = Command::new("/bin/sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let exit_notice =
        rustix::process::pidfd_open(Pid::from_child(&child), PidfdFlags::empty()).ok();
    Ok(OwnedRuntime {
        child: Mutex::new(Some(child)),
        exit_notice,
        client: SemanticClient::new(SemanticClientConfig::new(
            endpoint,
            "fixture",
            "public-fixture-token",
        )?)?,
        endpoint: RuntimeAddress::Loopback(endpoint),
        token_credential: SecretToken::fixture(),
        identity: StableRuntimeIdentity {
            launch_contract_id: crate::writing::WRITING_CONTRACT,
            binary_sha256: "0".repeat(64),
            runtime_bundle_manifest_sha256: None,
            model_sha256: "0".repeat(64),
            model_size: 1,
            model_alias: "fixture".to_owned(),
            model_origin: None,
            threads: 1,
            context_size: super::CONTEXT_SIZE,
            gpu_layers: super::GPU_LAYERS,
            batch_size: Some(16),
            ubatch_size: Some(16),
            memory_map: true,
            full_swa_cache: false,
        },
    })
}

#[tokio::test]
async fn exit_is_observed_by_event_and_by_the_polling_fallback() -> Result<(), Box<dyn Error>> {
    let endpoint = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 9));
    for event_driven in [true, false] {
        let mut runtime = owned_runtime_at(endpoint)?;
        assert!(runtime.exit_notice.is_some());
        if !event_driven {
            runtime.exit_notice = None;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), runtime.exited())
                .await
                .is_err(),
            "a live runtime must not report an exit"
        );
        let pid = Pid::from_raw(i32::try_from(runtime.process_id().unwrap())?).unwrap();
        rustix::process::kill_process(pid, Signal::KILL)?;
        tokio::time::timeout(Duration::from_secs(3), runtime.exited()).await?;
        assert!(!runtime.is_alive());
    }
    Ok(())
}

#[tokio::test]
async fn warm_up_reports_completion_and_failure_without_failing_the_runtime()
-> Result<(), Box<dyn Error>> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let runtime = owned_runtime_at(listener.local_addr()?)?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("warm-up connection");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.expect("warm-up request");
            assert_ne!(read, 0);
            request.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&request);
            let Some((headers, body)) = text.split_once("\r\n\r\n") else {
                continue;
            };
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .expect("content length")
                .parse()
                .expect("length");
            if body.len() >= length {
                break;
            }
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .await
            .expect("warm-up reply");
        let (_stalled, _) = listener.accept().await.expect("stalled connection");
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let completed = runtime.warm_up().await;
    assert_eq!(completed.failure(), None);
    let rendered = completed.to_string();
    assert!(rendered.starts_with("completed elapsed_ms="), "{rendered}");

    let failed = runtime.warm_up_within(Duration::from_millis(100)).await;
    assert_eq!(failed.failure(), Some("timeout"));
    assert!(failed.elapsed() < Duration::from_millis(500));
    let rendered = failed.to_string();
    assert!(
        rendered.starts_with("failed class=timeout elapsed_ms="),
        "{rendered}"
    );
    assert!(crate::provider::CompletionProvider::is_alive(&runtime));
    server.abort();
    let process_id = runtime.process_id().expect("owned child");
    drop(runtime);
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    Ok(())
}

fn verify_observed_file(path: &Path) -> Result<VerifiedFile, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    Ok(verify_file(&FileExpectation::new(
        path,
        super::encode_lower_hex(Sha256::digest(&bytes)),
        u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    )?)?)
}
