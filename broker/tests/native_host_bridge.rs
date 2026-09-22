use std::io::{self, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use badi_broker::ipc::{read_envelope, write_envelope};
use badi_broker::native_host::{bridge_streams, read_chrome_envelope, write_chrome_envelope};
use badi_broker::protocol::{EmptyPayload, MessageType, WireEnvelope};
use tokio::io::{AsyncWrite, AsyncWriteExt as _, duplex};
use tokio::net::UnixStream;
use tokio::time::timeout;

fn health_request(mono_ms: u64) -> WireEnvelope {
    WireEnvelope::global(
        MessageType::HealthRequest,
        mono_ms,
        &EmptyPayload::default(),
    )
    .expect("valid health request")
}

#[tokio::test]
async fn native_process_exits_on_broker_eof_while_chrome_keeps_stdin_open() {
    struct OwnedHost(std::process::Child);
    impl Drop for OwnedHost {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let directory = tempfile::tempdir().expect("private directory");
    let socket = directory.path().join("broker.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("private listener");
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
        .expect("private socket");
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_badi-native-host"))
        .arg(badi_broker::native_host::development_extension_origin())
        .arg("--socket")
        .arg(&socket)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("native host process");
    let mut child = OwnedHost(child);
    let mut input = child.0.stdin.take().expect("Chrome pipe");
    let (mut broker, _) = timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("native connection deadline")
        .expect("native connection");
    let body = serde_json::to_vec(&health_request(1)).expect("Chrome frame");
    input
        .write_all(
            &u32::try_from(body.len())
                .expect("small frame")
                .to_ne_bytes(),
        )
        .expect("Chrome header");
    input.write_all(&body).expect("Chrome body");
    read_envelope(&mut broker)
        .await
        .expect("broker frame")
        .expect("one request");
    // A second read from still-open Chrome stdin must not keep stdout or the
    // process alive after the broker connection ends.
    tokio::time::sleep(Duration::from_millis(30)).await;
    drop(broker);
    let status = timeout(Duration::from_secs(1), async {
        loop {
            if let Some(status) = child.0.try_wait().expect("owned host status") {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("native host exits without Chrome closing stdin");
    assert!(status.success());
    drop(input);
}

#[tokio::test]
async fn relays_validated_envelopes_in_both_directions() {
    let request = health_request(7);
    let response = health_request(8);
    let (host_broker, mut fake_broker) = UnixStream::pair().expect("UDS pair");
    let (mut chrome_sender, host_input) = duplex(1_024);
    let (host_output, mut chrome_receiver) = duplex(1_024);

    let bridge = tokio::spawn(bridge_streams(host_input, host_output, host_broker));
    let expected_request = request.clone();
    let fake_broker_task = tokio::spawn(async move {
        let received = read_envelope(&mut fake_broker)
            .await
            .expect("broker frame")
            .expect("one broker envelope");
        assert_eq!(received, expected_request);
        write_envelope(&mut fake_broker, &response)
            .await
            .expect("broker reply");
        assert!(
            read_envelope(&mut fake_broker)
                .await
                .expect("broker EOF")
                .is_none()
        );
        fake_broker.shutdown().await.expect("broker shutdown");
        response
    });

    write_chrome_envelope(&mut chrome_sender, &request)
        .await
        .expect("Chrome request");
    chrome_sender.shutdown().await.expect("Chrome input EOF");
    let expected_response = fake_broker_task.await.expect("fake broker task");
    let received = read_chrome_envelope(&mut chrome_receiver)
        .await
        .expect("Chrome response frame")
        .expect("one Chrome response");
    assert_eq!(received, expected_response);
    assert!(
        read_chrome_envelope(&mut chrome_receiver)
            .await
            .expect("Chrome output EOF")
            .is_none()
    );
    timeout(Duration::from_secs(1), bridge)
        .await
        .expect("bridge termination")
        .expect("bridge task")
        .expect("clean bridge shutdown");
}

#[tokio::test]
async fn treats_a_broken_chrome_output_pipe_as_clean_disconnect() {
    let (chrome_sender, host_input) = duplex(64);
    let (host_broker, mut fake_broker) = UnixStream::pair().expect("UDS pair");
    let bridge = tokio::spawn(bridge_streams(host_input, BrokenPipeWriter, host_broker));

    write_envelope(&mut fake_broker, &health_request(9))
        .await
        .expect("broker reply");
    timeout(Duration::from_secs(1), bridge)
        .await
        .expect("bridge termination")
        .expect("bridge task")
        .expect("broken Chrome pipe is a clean disconnect");
    drop(chrome_sender);
}

struct BrokenPipeWriter;

impl AsyncWrite for BrokenPipeWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        _buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "closed Chrome pipe",
        )))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
