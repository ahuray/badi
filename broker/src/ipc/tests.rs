use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};

use super::{FrameError, read_envelope, read_frame, write_frame};
use crate::protocol::MAX_FRAME_BYTES;

#[tokio::test]
async fn framing_is_four_byte_little_endian() {
    let (mut client, mut server) = duplex(64);
    let sender = tokio::spawn(async move { write_frame(&mut client, b"{}".as_slice()).await });
    let mut raw = [0_u8; 6];
    server.read_exact(&mut raw).await.expect("framed bytes");
    sender.await.expect("writer task").expect("write frame");
    assert_eq!(&raw[..4], &[2, 0, 0, 0]);
    assert_eq!(&raw[4..], b"{}");
}

#[tokio::test]
async fn rejects_oversized_length_before_allocating_body() {
    let (mut client, mut server) = duplex(16);
    client
        .write_all(
            &u32::try_from(MAX_FRAME_BYTES + 1)
                .expect("test size fits u32")
                .to_le_bytes(),
        )
        .await
        .expect("header");
    assert!(matches!(
        read_frame(&mut server).await,
        Err(FrameError::Oversized(size)) if size == MAX_FRAME_BYTES + 1
    ));
}

#[tokio::test]
async fn rejects_truncated_header_and_body() {
    let (mut client, mut server) = duplex(16);
    client.write_all(&[4, 0]).await.expect("partial header");
    drop(client);
    assert!(matches!(
        read_frame(&mut server).await,
        Err(FrameError::Truncated)
    ));

    let (mut client, mut server) = duplex(16);
    client
        .write_all(&[4, 0, 0, 0, b'{'])
        .await
        .expect("partial body");
    drop(client);
    assert!(matches!(
        read_frame(&mut server).await,
        Err(FrameError::Truncated)
    ));
}

#[tokio::test]
async fn rejects_malformed_and_invalid_utf8_json() {
    for body in [b"{".as_slice(), &[0xff, 0xfe]] {
        let (mut client, mut server) = duplex(64);
        write_frame(&mut client, body).await.expect("frame");
        assert!(matches!(
            read_envelope(&mut server).await,
            Err(FrameError::Serde(_))
        ));
    }
}
