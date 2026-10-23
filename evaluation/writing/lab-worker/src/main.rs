//! Local Prediction Lab worker. Stdio is the only control channel; no broker
//! policy, desktop integration or writing persistence is touched.

use std::io::{BufRead, Read, Write};
use std::path::PathBuf;

use badi_writing_lab::{MAX_FRAME_BYTES, Request};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn emit(value: &Value) -> std::io::Result<()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn error(id: Option<&str>, code: &str) -> std::io::Result<()> {
    emit(&json!({"type":"error","id":id,"error":code}))
}

fn directory_from(args: &[std::ffi::OsString]) -> Result<PathBuf, &'static str> {
    match args {
        [] => badi_broker::writing::data_directory().map_err(|_| "model_directory_unavailable"),
        [flag, path] if flag == "--model-directory" && PathBuf::from(path).is_absolute() => {
            Ok(path.into())
        }
        _ => Err("invalid_arguments"),
    }
}

struct LabOptions {
    directory: PathBuf,
    artifact: Option<badi_writing_lab::artifact::ModelArtifactOverride>,
    prefill_batch: badi_writing_lab::PrefillBatch,
    memory_map: bool,
}

fn lab_options_from(args: &[std::ffi::OsString]) -> Result<LabOptions, &'static str> {
    let mut directory = None;
    let mut descriptor = None;
    let mut prefill_batch = None;
    let mut memory_map = None;
    let mut pairs = args.chunks_exact(2);
    for pair in &mut pairs {
        if pair[0] == "--prefill-batch" && prefill_batch.is_none() {
            prefill_batch = Some(badi_writing_lab::PrefillBatch::parse(&pair[1])?);
            continue;
        }
        if pair[0] == "--memory-map" && memory_map.is_none() {
            memory_map = Some(match pair[1].to_str() {
                Some("true") => true,
                Some("false") => false,
                _ => return Err("invalid_arguments"),
            });
            continue;
        }
        let path = PathBuf::from(&pair[1]);
        if !path.is_absolute() {
            return Err("invalid_arguments");
        }
        if pair[0] == "--model-directory" && directory.is_none() {
            directory = Some(path);
        } else if pair[0] == "--model-artifact" && descriptor.is_none() {
            descriptor = Some(path);
        } else {
            return Err("invalid_arguments");
        }
    }
    if !pairs.remainder().is_empty() {
        return Err("invalid_arguments");
    }
    let artifact = descriptor
        .map(|path| badi_writing_lab::artifact::ModelArtifactOverride::read(&path))
        .transpose()?;
    Ok(LabOptions {
        directory: directory.map_or_else(|| directory_from(&[]), Ok)?,
        artifact,
        prefill_batch: prefill_batch.unwrap_or_default(),
        memory_map: memory_map.unwrap_or(true),
    })
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--detect-language") {
        if let Err(code) = detect_languages(&args[1..]) {
            let _ = error(None, code);
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|arg| {
        arg == "--inspect-device" || arg == "--assess-model" || arg == "--rank-models"
    }) {
        if let Err(code) = qualification(&args) {
            let _ = error(None, code);
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|arg| arg == "--context-lookup") {
        if let Err(code) = badi_writing_lab::context_lookup::run_stdio(&args[1..]) {
            let _ = emit(
                &json!({"schema":badi_writing_lab::context_lookup::ERROR_SCHEMA,"error":code}),
            );
            std::process::exit(1);
        }
        return;
    }
    // Dispatch before creating any threads: this process will replace itself
    // with the runtime, retaining the armed Linux parent-death signal.
    #[cfg(target_os = "linux")]
    {
        if args
            .first()
            .is_some_and(|arg| arg == badi_writing_lab::spelling::engine::HELPER_FLAG)
        {
            match badi_writing_lab::spelling::engine::exec_helper(&args[1..]) {
                Ok(never) => match never {},
                Err(code) => {
                    eprintln!("{code}");
                    std::process::exit(1);
                }
            }
        }
        if args
            .first()
            .is_some_and(|arg| arg == badi_broker::writing::EXEC_HELPER_FLAG)
        {
            match badi_broker::writing::exec_runtime_helper(&args[1..]) {
                Ok(never) => match never {},
                Err(code) => {
                    eprintln!("{code}");
                    std::process::exit(1);
                }
            }
        }
    }
    run_worker();
}

/// Content-free results for explicit Lab samples; no runtime is started.
fn detect_languages(args: &[std::ffi::OsString]) -> Result<(), &'static str> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Sample {
        id: String,
        before: String,
    }
    if !args.is_empty() {
        return Err("invalid_arguments");
    }
    let mut input = std::io::stdin().lock();
    loop {
        let mut frame = Vec::new();
        let read = input
            .by_ref()
            .take((MAX_FRAME_BYTES + 1) as u64)
            .read_until(b'\n', &mut frame)
            .map_err(|_| "invalid_frame")?;
        if read == 0 {
            return Ok(());
        }
        if frame.len() > MAX_FRAME_BYTES || frame.last() != Some(&b'\n') {
            return Err("invalid_frame");
        }
        let sample: Sample = serde_json::from_slice(&frame).map_err(|_| "invalid_request")?;
        if sample.id.is_empty() || sample.id.len() > 128 || sample.id.chars().any(char::is_control)
        {
            return Err("invalid_request");
        }
        let started = std::time::Instant::now();
        let language = badi_broker::language::detect_field_language(&sample.before);
        emit(
            &json!({"schema":"badi.field-language.result.v1", "id":sample.id,
            "language":language, "elapsed_us":started.elapsed().as_secs_f64() * 1_000_000.0}),
        )
        .map_err(|_| "output_closed")?;
    }
}

fn qualification(args: &[std::ffi::OsString]) -> Result<(), &'static str> {
    use badi_writing_lab::qualification::{
        CandidateMetadata, QualificationEvidence, QualificationSettings, assess_current,
        inspect_device, rank,
    };
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        candidate: CandidateMetadata,
        settings: QualificationSettings,
        evidence: Option<QualificationEvidence>,
    }
    let cache = match &args[1..] {
        [] => badi_broker::writing::data_directory().map_err(|_| "model_directory_unavailable")?,
        [flag, path] if flag == "--cache-directory" && PathBuf::from(path).is_absolute() => {
            PathBuf::from(path)
        }
        _ => return Err("invalid_arguments"),
    };
    let report = if args[0] == "--inspect-device" {
        serde_json::to_value(inspect_device(&cache)).map_err(|_| "serialization_failed")?
    } else {
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "invalid_frame")?;
        if bytes.len() > 128 * 1024 {
            return Err("invalid_frame");
        }
        if args[0] == "--rank-models" {
            let inputs: Vec<Input> =
                serde_json::from_slice(&bytes).map_err(|_| "invalid_assessment")?;
            if inputs.len() > 32 {
                return Err("invalid_assessment");
            }
            let assessments: Vec<_> = inputs
                .into_iter()
                .map(|input| {
                    assess_current(input.candidate, input.settings, input.evidence, &cache)
                })
                .collect();
            serde_json::to_value(rank(&assessments)).map_err(|_| "serialization_failed")?
        } else {
            let input: Input = serde_json::from_slice(&bytes).map_err(|_| "invalid_assessment")?;
            serde_json::to_value(assess_current(
                input.candidate,
                input.settings,
                input.evidence,
                &cache,
            ))
            .map_err(|_| "serialization_failed")?
        }
    };
    emit(&report).map_err(|_| "output_closed")
}

#[tokio::main]
async fn run_worker() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--spelling-lab") {
        if let Err(code) = badi_writing_lab::spelling::run(&args[1..]).await {
            let _ = error(None, code);
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|arg| arg == "--stop-token-probe") {
        match stop_token_probe(&args[1..]).await {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(code) => {
                let _ = error(None, code);
                std::process::exit(1);
            }
        }
        return;
    }
    if args.first().is_some_and(|arg| arg == "--prefill-probe") {
        match prefill_probe(&args[1..]).await {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(code) => {
                let _ = error(None, code);
                std::process::exit(1);
            }
        }
        return;
    }
    if args.first().is_some_and(|arg| arg == "--revision-lab") {
        if let Err(code) = revision_worker(&args[1..]).await {
            let _ = error(None, code);
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|arg| arg == "--paced-probe") {
        match paced_probe(&args[1..]).await {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(code) => {
                let _ = error(None, code);
                std::process::exit(1);
            }
        }
        return;
    }
    if let Err(code) = worker(&args).await {
        let _ = error(None, code);
        std::process::exit(1);
    }
}

async fn paced_probe(args: &[std::ffi::OsString]) -> Result<bool, &'static str> {
    use badi_writing_lab::paced_probe::{Control, MAX_INPUT_BYTES, ProbeCancellation, ProbeInput};
    let LabOptions {
        directory,
        artifact,
        prefill_batch,
        memory_map,
    } = lab_options_from(args)?;
    let cancellation = ProbeCancellation::default();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "signal_unavailable")?;
    let signal_cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        signal_cancel.cancel(Control::Cancel);
    });
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let input_cancel = cancellation.clone();
    std::thread::spawn(move || {
        let mut input = std::io::BufReader::new(std::io::stdin());
        let read_frame = |input: &mut std::io::BufReader<std::io::Stdin>, limit: usize| {
            let mut frame = Vec::new();
            let count = input
                .by_ref()
                .take((limit + 1) as u64)
                .read_until(b'\n', &mut frame)
                .map_err(|_| "invalid_frame")?;
            if count == 0 {
                return Ok(None);
            }
            if frame.len() > limit || frame.last() != Some(&b'\n') {
                return Err("invalid_frame");
            }
            Ok(Some(frame))
        };
        let frame =
            read_frame(&mut input, MAX_INPUT_BYTES).and_then(|frame| frame.ok_or("invalid_frame"));
        let valid_frame = frame.is_ok();
        if sender.send(frame).is_err() || !valid_frame {
            return;
        }
        // EOF after the single trace is normal. A later control invalidates
        // the whole trace; it can never introduce new writing or model options.
        match read_frame(&mut input, 128) {
            Ok(None) => {}
            Ok(Some(frame)) => match serde_json::from_slice::<Control>(&frame) {
                Ok(control) => input_cancel.cancel(control),
                Err(_) => input_cancel.cancel(Control::Cancel),
            },
            Err(_) => input_cancel.cancel(Control::Cancel),
        }
    });
    let frame = tokio::select! {
        biased;
        () = cancellation.cancelled() => return Err("cancelled"),
        result = tokio::time::timeout(std::time::Duration::from_secs(10), receiver) =>
            result.map_err(|_| "input_deadline")?.map_err(|_| "input_closed")??,
    };
    let input: ProbeInput = serde_json::from_slice(&frame).map_err(|_| "invalid_paced_request")?;
    input.validate()?;
    // Direct await keeps owned runtime creation on main's block_on thread.
    let report = badi_writing_lab::paced_probe::run_with_options(
        directory,
        artifact,
        prefill_batch,
        memory_map,
        input,
        cancellation,
        |ready| emit(&serde_json::to_value(ready).map_err(|_| ())?).map_err(|_| ()),
    )
    .await?;
    let complete = report.complete;
    emit(&json!({"type":"paced_report","report":report})).map_err(|_| "output_closed")?;
    Ok(complete)
}

async fn stop_token_probe(args: &[std::ffi::OsString]) -> Result<bool, &'static str> {
    let artifact = badi_writing_lab::stop_token_probe::artifact_from(args)?;
    let directory = directory_from(&[])?;
    let cancellation = CancellationToken::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "signal_unavailable")?;
    let signal_cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        signal_cancel.cancel();
    });
    // Fixed diagnostic: stdin is never read and this future owns startup on
    // main's block_on thread, exactly like the other owned runtime probes.
    let report =
        badi_writing_lab::stop_token_probe::run(directory, artifact, cancellation, |event| {
            emit(event).map_err(|_| ())
        })
        .await;
    let complete = report.complete;
    emit(&serde_json::to_value(report).map_err(|_| "serialization_failed")?)
        .map_err(|_| "output_closed")?;
    Ok(complete)
}

async fn prefill_probe(args: &[std::ffi::OsString]) -> Result<bool, &'static str> {
    let directory = directory_from(args)?;
    let cancellation = CancellationToken::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "signal_unavailable")?;
    let signal_cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        signal_cancel.cancel();
    });
    // Direct await retains the same main-thread parent-death invariant as the
    // interactive Lab worker. Stdin never supplies diagnostic prompts.
    let report = badi_writing_lab::prefill_probe::run(directory, cancellation).await;
    let complete = report.complete;
    emit(&serde_json::to_value(report).map_err(|_| "serialization_failed")?)
        .map_err(|_| "output_closed")?;
    Ok(complete)
}

async fn worker(args: &[std::ffi::OsString]) -> Result<(), &'static str> {
    let LabOptions {
        directory,
        artifact,
        prefill_batch,
        memory_map,
    } = lab_options_from(args)?;
    let cancellation = CancellationToken::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "signal_unavailable")?;
    let signal_cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        signal_cancel.cancel();
    });
    let mut receiver = frame_reader(cancellation.clone());
    // Linux parent-death signalling tracks the thread that spawns the child.
    // Keep activate directly polled by main's block_on until shutdown; never
    // move the spawn into a temporary thread or a spawn_blocking task.
    let runtime = tokio::select! {
        biased;
        () = cancellation.cancelled() => return Ok(()),
        runtime = badi_writing_lab::activate_with_options(directory, artifact, prefill_batch, memory_map) => runtime.map_err(|_| "runtime_start_failed")?,
    };
    emit(
        &json!({"type":"ready","identity":runtime.identity(),"runtime_pid":runtime.process_id(),
        "schema":"badi.prediction-lab.worker.v1"}),
    )
    .map_err(|_| "output_closed")?;
    loop {
        let frame = tokio::select! {
            biased;
            () = cancellation.cancelled() => break,
            frame = receiver.recv() => frame,
        };
        let Some(frame) = frame else {
            break;
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(code) => {
                error(None, code).map_err(|_| "output_closed")?;
                break;
            }
        };
        let request: Request = if let Ok(request) = serde_json::from_slice(&frame) {
            request
        } else {
            error(None, "invalid_request").map_err(|_| "output_closed")?;
            continue;
        };
        let id = request.id.clone();
        let result = badi_writing_lab::run(&runtime, request, cancellation.clone()).await;
        if cancellation.is_cancelled() {
            break;
        }
        match result {
            Ok(result) => emit(&serde_json::to_value(result).map_err(|_| "serialization_failed")?)
                .map_err(|_| "output_closed")?,
            Err(failure) => error(
                (id.len() <= 128 && !id.chars().any(char::is_control)).then_some(id.as_str()),
                &failure.to_string(),
            )
            .map_err(|_| "output_closed")?,
        }
    }
    let cleanup = runtime.shutdown().map_err(|_| "runtime_shutdown_failed")?;
    let _ = emit(&json!({"type":"stopped","cleanup":cleanup}));
    Ok(())
}

/// Bounded stdin request lines; end of input cancels the session.
fn frame_reader(
    cancellation: CancellationToken,
) -> tokio::sync::mpsc::Receiver<Result<Vec<u8>, &'static str>> {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Vec<u8>, &'static str>>(1);
    // A normal thread avoids Tokio's uninterruptible stdin blocking task keeping
    // its runtime alive after SIGTERM. Process exit need not join this reader.
    std::thread::spawn(move || {
        let mut input = std::io::BufReader::new(std::io::stdin());
        loop {
            let mut frame = Vec::new();
            let read = input
                .by_ref()
                .take((MAX_FRAME_BYTES + 1) as u64)
                .read_until(b'\n', &mut frame);
            match read {
                Ok(0) => {
                    cancellation.cancel();
                    return;
                }
                Ok(_) if frame.len() <= MAX_FRAME_BYTES && frame.last() == Some(&b'\n') => {
                    if sender.blocking_send(Ok(frame)).is_err() {
                        return;
                    }
                }
                _ => {
                    let _ = sender.blocking_send(Err("invalid_frame"));
                    return;
                }
            }
        }
    });
    receiver
}

/// The revision lane: the installed revision model in the broker's revision
/// launch, one request line in and one result line out.
async fn revision_worker(args: &[std::ffi::OsString]) -> Result<(), &'static str> {
    use badi_writing_lab::revision::{self, RevisionRequest};
    let directory = directory_from(args)?;
    let cancellation = CancellationToken::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "signal_unavailable")?;
    let signal_cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        signal_cancel.cancel();
    });
    let mut receiver = frame_reader(cancellation.clone());
    // As in `worker`: the contained runtime is spawned from main's block_on.
    let runtime = tokio::select! {
        biased;
        () = cancellation.cancelled() => return Ok(()),
        runtime = revision::activate(directory) => runtime.map_err(|_| "runtime_start_failed")?,
    };
    let timeout_ms = u64::try_from(badi_broker::engine::REVISION_TIMEOUT.as_millis())
        .map_err(|_| "invalid_timeout")?;
    emit(
        &json!({"type":"ready","identity":runtime.identity(),"runtime_pid":runtime.process_id(),
        "schema":"badi.prediction-lab.worker.v1","request_timeout_ms":timeout_ms}),
    )
    .map_err(|_| "output_closed")?;
    loop {
        let frame = tokio::select! {
            biased;
            () = cancellation.cancelled() => break,
            frame = receiver.recv() => frame,
        };
        let Some(frame) = frame else {
            break;
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(code) => {
                error(None, code).map_err(|_| "output_closed")?;
                break;
            }
        };
        let Ok(request) = serde_json::from_slice::<RevisionRequest>(&frame) else {
            error(None, "invalid_request").map_err(|_| "output_closed")?;
            continue;
        };
        let result = tokio::select! {
            biased;
            () = cancellation.cancelled() => break,
            result = revision::run(&runtime, &request) => result,
        };
        match result {
            Ok(record) => emit(&serde_json::to_value(record).map_err(|_| "serialization_failed")?)
                .map_err(|_| "output_closed")?,
            Err(failure) => error(
                (request.id.len() <= 128 && !request.id.chars().any(char::is_control))
                    .then_some(request.id.as_str()),
                &failure.to_string(),
            )
            .map_err(|_| "output_closed")?,
        }
    }
    let cleanup = runtime.shutdown().map_err(|_| "runtime_shutdown_failed")?;
    let _ = emit(&json!({"type":"stopped","cleanup":cleanup}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::lab_options_from;
    use badi_writing_lab::PrefillBatch;
    use std::ffi::OsString;

    #[test]
    fn prefill_batch_is_a_strict_launch_option_with_unchanged_default() {
        let directory = std::env::temp_dir();
        let mut args = vec![
            OsString::from("--model-directory"),
            directory.into_os_string(),
        ];
        assert_eq!(
            lab_options_from(&args).unwrap().prefill_batch,
            PrefillBatch::Tokens16
        );
        assert!(lab_options_from(&args).unwrap().memory_map);
        let mut copied = args.clone();
        copied.extend(["--memory-map".into(), "false".into()]);
        assert!(!lab_options_from(&copied).unwrap().memory_map);
        copied.extend(["--memory-map".into(), "true".into()]);
        assert!(lab_options_from(&copied).is_err());
        let mut invalid_loading = args.clone();
        invalid_loading.extend(["--memory-map".into(), "0".into()]);
        assert!(lab_options_from(&invalid_loading).is_err());
        for (value, expected) in [
            ("16", PrefillBatch::Tokens16),
            ("64", PrefillBatch::Tokens64),
        ] {
            let mut selected = args.clone();
            selected.extend(["--prefill-batch".into(), value.into()]);
            assert_eq!(lab_options_from(&selected).unwrap().prefill_batch, expected);
        }
        for invalid in [
            "", "0", "4", "32", "128", "064", "+64", "64.0", " 64", "64\n",
        ] {
            let mut selected = args.clone();
            selected.extend(["--prefill-batch".into(), invalid.into()]);
            assert!(lab_options_from(&selected).is_err());
        }
        args.extend(["--prefill-batch".into(), "64".into()]);
        args.extend(["--prefill-batch".into(), "16".into()]);
        assert!(lab_options_from(&args).is_err());
    }
}
