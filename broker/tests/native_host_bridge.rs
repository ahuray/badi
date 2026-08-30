use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use omatype_broker::ipc::{read_envelope, write_envelope};
use omatype_broker::native_host::{bridge_streams, read_chrome_envelope, write_chrome_envelope};
use omatype_broker::protocol::{EmptyPayload, MessageType, WireEnvelope};
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
