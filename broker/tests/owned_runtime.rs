//! Owned llama.cpp runtime lifecycle against a fake llama-server. The runtime
//! launches this test executable itself as that server, so the target has no
//! libtest harness: `main` either serves or runs the tests below in order.
//! Name filters and `--exact`, `--skip`, `--list` and `--ignored` behave as
//! in libtest.

use std::error::Error;
use std::fs;
use std::future::Future;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use badi_broker::engine::{Broker, BrokerConfig, BrokerError, BrokerEventSink, SessionAuthority};
use badi_broker::protocol::{
    Activation, AdapterKind, Capability, Coordinates, ProviderKind, SessionId, SessionOpenPayload,
    TargetDescriptor, TargetKind,
};
use badi_broker::provider::{CompletionProvider, ProviderRequest};
use badi_broker::semantic::client::CompletionDisposition;
use badi_broker::semantic::provenance::{
    FileExpectation, ProvenanceError, VerifiedFile, verify_file,
};
use badi_broker::semantic::runtime::{
    FIXTURE_TOKEN_CANARY, FixtureBehavior, LlamaCppLaunch, RuntimeError, WARM_UP_TIMEOUT,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::io::AsyncReadExt as _;
use tokio::net::UnixStream;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

#[path = "support/fake_llama_server.rs"]
mod fake_llama_server;

type TestResult = Result<(), Box<dyn Error>>;
type Test = (&'static str, fn() -> TestResult);

/// Each test's name and a runner that drives it on its own runtime.
macro_rules! tests {
    ($($test:ident),* $(,)?) => {
        [$((stringify!($test), (|| block_on($test())) as fn() -> TestResult)),*]
    };
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some(fake_llama_server::ARGUMENT) {
        return fake_llama_server::run();
    }
    run_tests(&tests![
        owned_runtime_keeps_its_token_private_and_is_reaped,
        runtime_rejects_wrong_artifacts_bad_health_early_exit_and_orphans,
        failed_warm_up_leaves_the_owned_runtime_serving,
        owned_runtime_death_stops_broker_and_retires_sessions,
    ])
}

async fn owned_runtime_keeps_its_token_private_and_is_reaped() -> TestResult {
    let fixture = OwnedFixture::new()?;
    let runtime = fixture.launch(FixtureBehavior::Ready)?.spawn().await?;
    assert_eq!(CompletionProvider::kind(&runtime), ProviderKind::LocalModel);
    assert!(runtime.endpoint().ip().is_loopback());
    let process_id = runtime.process_id().expect("owned child process");
    let cmdline = fs::read(format!("/proc/{process_id}/cmdline"))?;
    assert!(
        !cmdline
            .windows(FIXTURE_TOKEN_CANARY.len())
            .any(|window| window == FIXTURE_TOKEN_CANARY.as_bytes())
    );
    assert!(!format!("{runtime:?}").contains(FIXTURE_TOKEN_CANARY));

    let lifecycle = runtime.shutdown()?;
    assert_eq!(lifecycle.process_id(), process_id);
    assert!(lifecycle.reaped());
    assert_eq!(lifecycle.exit_code(), None);
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    Ok(())
}

async fn runtime_rejects_wrong_artifacts_bad_health_early_exit_and_orphans() -> TestResult {
    let fixture = OwnedFixture::new()?;
    let wrong_digest = FileExpectation::new(
        fixture.model.path(),
        "0".repeat(64),
        fixture.model.identity().size,
    )?;
    assert!(matches!(
        verify_file(&wrong_digest),
        Err(ProvenanceError::DigestMismatch)
    ));

    let link_path = fixture.directory.path().join("linked-model.gguf");
    symlink(fixture.model.path(), &link_path)?;
    let linked = FileExpectation::new(
        fs::canonicalize(fixture.directory.path())?.join("linked-model.gguf"),
        fixture.model.sha256(),
        fixture.model.identity().size,
    )?;
    assert!(matches!(
        verify_file(&linked),
        Err(ProvenanceError::NonCanonicalPath | ProvenanceError::Symlink)
    ));

    let early = fixture.launch(FixtureBehavior::EarlyExit)?.spawn().await;
    assert!(matches!(early, Err(RuntimeError::EarlyExit(_))));
    let malformed = fixture
        .launch(FixtureBehavior::MalformedHealth)?
        .spawn()
        .await;
    assert!(matches!(malformed, Err(RuntimeError::Health(_))));
    let timeout = fixture
        .launch(FixtureBehavior::NoBind)?
        .with_startup_timeout(Duration::from_millis(80))?
        .spawn()
        .await;
    assert!(matches!(timeout, Err(RuntimeError::StartupTimeout)));

    let runtime = fixture.launch(FixtureBehavior::Ready)?.spawn().await?;
    let process_id = runtime.process_id().expect("owned child process");
    drop(runtime);
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    Ok(())
}

async fn failed_warm_up_leaves_the_owned_runtime_serving() -> TestResult {
    let fixture = OwnedFixture::new()?;
    let runtime = fixture.launch(FixtureBehavior::Ready)?.spawn().await?;
    // The fake server closes the non-streaming warm-up without a reply.
    let report = runtime.warm_up().await;
    assert_eq!(report.failure(), Some("transport"));
    assert!(report.elapsed() < WARM_UP_TIMEOUT);
    assert!(runtime.is_alive());
    let observed = runtime
        .client()
        .complete_observed(
            ProviderRequest {
                before: "Thank you".to_owned(),
                after: String::new(),
                language: Some("en".to_owned()),
            },
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(observed.disposition(), CompletionDisposition::Suggested);
    assert_eq!(observed.output(), Some(" for your time."));
    let process_id = runtime.process_id().expect("owned child process");
    drop(runtime);
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    Ok(())
}

async fn owned_runtime_death_stops_broker_and_retires_sessions() -> TestResult {
    let fixture = OwnedFixture::new()?;
    let runtime = Arc::new(fixture.launch(FixtureBehavior::Ready)?.spawn().await?);
    assert!(runtime.is_alive());
    let process_id = runtime.process_id().expect("owned live child");
    let pid = rustix::process::Pid::from_raw(i32::try_from(process_id)?)
        .expect("positive owned child PID");
    let broker = Broker::new(runtime.clone(), BrokerConfig::default());
    let coordinates = Coordinates {
        session_id: SessionId::new(),
        focus_epoch: 1,
        revision: 0,
    };
    let payload = SessionOpenPayload {
        target: TargetDescriptor {
            kind: TargetKind::Fixture,
            app_id: "runtime-lifetime-test".to_owned(),
            target_id: "disposable-field".to_owned(),
            origin: None,
        },
        activation: Activation::Always,
    };
    let authority = SessionAuthority {
        protocol_version: 1,
        adapter_kind: AdapterKind::Test,
        capabilities: vec![Capability::Context, Capability::Suggestion],
    };
    let (sender, _events) = tokio::sync::mpsc::channel(8);
    let sink = BrokerEventSink::new(sender, CancellationToken::new());
    broker
        .open_session(
            coordinates,
            payload.clone(),
            authority.clone(),
            sink.clone(),
        )
        .await?;
    assert_eq!(broker.session_count().await, 1);

    let socket = fixture.directory.path().join("private/broker.sock");
    let server_socket = socket.clone();
    let server_broker = broker.clone();
    let server =
        tokio::spawn(async move { badi_broker::server::run(&server_socket, server_broker).await });
    let mut connection = timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(connection) = UnixStream::connect(&socket).await {
                break connection;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    // The fake server cannot exit on its own. Its unreaped owned PID remains
    // bound while the server observes the exit event.
    rustix::process::kill_process(pid, rustix::process::Signal::KILL)?;
    let outcome = timeout(Duration::from_secs(2), server).await??;
    assert!(matches!(
        outcome,
        Err(badi_broker::server::ServerError::ProviderExited)
    ));
    assert!(!runtime.is_alive());
    assert!(!socket.exists());
    assert_eq!(broker.session_count().await, 0);
    assert_eq!(connection.read(&mut [0_u8]).await?, 0);
    assert!(matches!(
        broker
            .open_session(coordinates, payload, authority, sink)
            .await,
        Err(BrokerError::ShuttingDown)
    ));
    drop(broker);
    let runtime = Arc::try_unwrap(runtime).expect("server dropped runtime ownership");
    let observation = runtime.shutdown()?;
    assert!(observation.reaped());
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    Ok(())
}

/// A private model file and this executable as the verified runtime binary.
struct OwnedFixture {
    directory: TempDir,
    binary: VerifiedFile,
    model: VerifiedFile,
}

impl OwnedFixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let model_path = fs::canonicalize(directory.path())?.join("fixture-model.gguf");
        fs::write(&model_path, b"Badi owned-runtime fixture model\n")?;
        let binary = verify_observed_file(&fs::canonicalize(std::env::current_exe()?)?)?;
        let model = verify_observed_file(&model_path)?;
        Ok(Self {
            directory,
            binary,
            model,
        })
    }

    fn launch(&self, behavior: FixtureBehavior) -> Result<LlamaCppLaunch, RuntimeError> {
        LlamaCppLaunch::for_fixture(self.binary.clone(), self.model.clone(), behavior)
    }
}

fn verify_observed_file(path: &Path) -> Result<VerifiedFile, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .fold(String::new(), |hex, byte| hex + &format!("{byte:02x}"));
    let expectation = FileExpectation::new(path, digest, u64::try_from(bytes.len())?)?;
    Ok(verify_file(&expectation)?)
}

fn block_on(test: impl Future<Output = TestResult>) -> TestResult {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(test)
}

fn run_tests(tests: &[Test]) -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let (mut filters, mut skips) = (Vec::new(), Vec::new());
    let (mut exact, mut list, mut ignored_only) = (false, false, false);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--exact" => exact = true,
            "--list" => list = true,
            "--ignored" => ignored_only = true,
            "--skip" => skips.extend(arguments.next()),
            "--test-threads" | "--color" | "--format" | "--logfile" | "-Z" => {
                arguments.next();
            }
            flag if flag.starts_with('-') => {}
            _ => filters.push(argument),
        }
    }
    let matches = |name: &str, pattern: &String| {
        if exact {
            name == pattern
        } else {
            name.contains(pattern.as_str())
        }
    };
    // None of these tests is ignored.
    let selected = tests
        .iter()
        .filter(|(name, _)| {
            !ignored_only
                && (filters.is_empty() || filters.iter().any(|pattern| matches(name, pattern)))
                && !skips.iter().any(|pattern| matches(name, pattern))
        })
        .collect::<Vec<_>>();
    if list {
        for (name, _) in &selected {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    println!("\nrunning {} tests", selected.len());
    let mut failed = Vec::new();
    for (name, test) in &selected {
        let passed = match std::panic::catch_unwind(test) {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                eprintln!("{name}: {error}");
                false
            }
            Err(_) => false,
        };
        println!("test {name} ... {}", if passed { "ok" } else { "FAILED" });
        if !passed {
            failed.push(*name);
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        selected.len() - failed.len(),
        failed.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(101)
    }
}
