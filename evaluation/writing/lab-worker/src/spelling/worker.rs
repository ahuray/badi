use super::{MAX_FRAME_BYTES, Request, artifact::Artifact, engine::OwnedEngine};
use serde_json::json;
use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
use tokio_util::sync::CancellationToken;

fn emit(value: &impl serde::Serialize) -> Result<(), &'static str> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value).map_err(|_| "output_closed")?;
    output
        .write_all(b"\n")
        .and_then(|()| output.flush())
        .map_err(|_| "output_closed")
}
fn error(id: Option<&str>, code: &'static str) -> Result<(), &'static str> {
    emit(&json!({"type":"error","id":id,"error":code}))
}

pub async fn run(args: &[OsString]) -> Result<(), &'static str> {
    let [manifest_flag, path, language_flag, language] = args else {
        return Err("invalid_arguments");
    };
    if manifest_flag != "--spelling-manifest" || language_flag != "--language" {
        return Err("invalid_arguments");
    }
    let artifact = Artifact::read(
        std::path::Path::new(path),
        language.to_str().ok_or("invalid_arguments")?,
    )?;
    let cancellation = CancellationToken::new();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "signal_unavailable")?;
    let signal_cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::select! { _ = terminate.recv()=>{}, _=tokio::signal::ctrl_c()=>{} }
        signal_cancel.cancel();
    });
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let input_cancel = cancellation.clone();
    std::thread::spawn(move || {
        let mut input = std::io::BufReader::new(std::io::stdin());
        loop {
            let mut bytes = Vec::new();
            match input
                .by_ref()
                .take((MAX_FRAME_BYTES + 1) as u64)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) => {
                    input_cancel.cancel();
                    return;
                }
                Ok(_) if bytes.len() <= MAX_FRAME_BYTES && bytes.last() == Some(&b'\n') => {
                    if sender.blocking_send(Ok(bytes)).is_err() {
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
    let mut engine = OwnedEngine::start(artifact, cancellation.clone()).await?;
    let operation = async {
        emit(&json!({"type":"ready","schema":"badi.spelling-lab.worker.v1","engine_pid":engine.process_id(),
            "identity":engine.identity(),"identity_sha256":engine.identity().sha256()}))?;
        loop {
            let frame = tokio::select! { biased;
                ()=cancellation.cancelled()=>break,
                value=receiver.recv()=>value,
            };
            let Some(frame)=frame else { break; };
            let frame = match frame { Ok(frame)=>frame, Err(code)=>{ error(None,code)?; break; } };
            let request:Request = if let Ok(request) = serde_json::from_slice(&frame) { request } else { error(None,"invalid_request")?; continue; };
            let id = (request.id.len()<=128 && !request.id.chars().any(char::is_control)).then(||request.id.clone());
            match super::check(&mut engine,request,cancellation.clone()).await {
                Ok(result)=>{ let deadline=result.outcome=="deadline"; emit(&result)?; if deadline { break; } }
                Err("invalid_request")=>error(id.as_deref(),"invalid_request")?,
                Err("cancelled")=>break,
                Err(code)=>{ error(id.as_deref(),code)?; break; }
            }
        }
        Ok::<(),&'static str>(())
    }.await;
    let cleanup = engine.shutdown()?;
    let stopped = emit(&json!({"type":"stopped","cleanup":cleanup}));
    operation.and(stopped)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_unknown_fields_and_invalid_protected_words_are_rejected() {
        let base = json!({"schema":super::super::REQUEST_SCHEMA,"id":"fixture","before":"Das Fahrad ","language":"de","protected_words":["Badi"]});
        let request: Request = serde_json::from_value(base.clone()).unwrap();
        request.validate("de").unwrap();
        for field in ["expected", "replacement", "context", "style", "budget_ms"] {
            let mut changed = base.clone();
            changed[field] = json!("private");
            assert!(serde_json::from_value::<Request>(changed).is_err());
        }
        for (field, value) in [
            ("protected_words", json!(["two words"])),
            ("protected_words", json!(vec!["Badi"; 33])),
            ("protected_words", json!(["a".repeat(25)])),
            ("before", json!("a".repeat(2049))),
            ("language", json!("fa")),
            ("before", json!("می‌ ")),
        ] {
            let mut changed = base.clone();
            changed[field] = value;
            assert!(
                serde_json::from_value::<Request>(changed)
                    .unwrap()
                    .validate("de")
                    .is_err()
            );
        }
    }
}
