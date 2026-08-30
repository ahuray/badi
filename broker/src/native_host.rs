use std::io;
use std::path::{Component, Path};

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::ipc::{
    FrameError, read_envelope, verify_peer_uid, verify_socket_metadata, write_envelope,
};
use crate::protocol::{MAX_FRAME_BYTES, WireEnvelope};

pub const NATIVE_HOST_NAME: &str = "io.omatype.broker";
pub const DEVELOPMENT_EXTENSION_ID: &str = "ckkiehcjbclcjckkkajohopoikeejkoa";

/// Chrome permits up to 64 MiB from Chrome to a native host.
pub const CHROME_TO_HOST_TRANSPORT_MAX_BYTES: usize = 64 * 1024 * 1024;
/// Chrome permits up to 1 MiB from a native host to Chrome.
pub const HOST_TO_CHROME_TRANSPORT_MAX_BYTES: usize = 1024 * 1024;

const _: () = assert!(MAX_FRAME_BYTES < CHROME_TO_HOST_TRANSPORT_MAX_BYTES);
const _: () = assert!(MAX_FRAME_BYTES < HOST_TO_CHROME_TRANSPORT_MAX_BYTES);

#[must_use]
pub fn development_extension_origin() -> String {
    format!("chrome-extension://{DEVELOPMENT_EXTENSION_ID}/")
}

pub fn validate_development_caller_origin(origin: &str) -> Result<(), NativeHostError> {
    if origin == development_extension_origin() {
        Ok(())
    } else {
        Err(NativeHostError::CallerOrigin)
    }
}

pub async fn read_chrome_frame<R>(reader: &mut R) -> Result<Option<Vec<u8>>, NativeHostError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 4];
    let first = reader.read(&mut header[..1]).await?;
    if first == 0 {
        return Ok(None);
    }
    reader
        .read_exact(&mut header[1..])
        .await
        .map_err(map_truncated)?;
    let length = usize::try_from(u32::from_ne_bytes(header)).unwrap_or(usize::MAX);
    if length == 0 {
        return Err(NativeHostError::ChromeFrameEmpty);
    }
    // The protocol limit is intentionally far stricter than Chrome's 64 MiB
    // inbound transport allowance, and is checked before allocating the body.
    if length > MAX_FRAME_BYTES {
        return Err(NativeHostError::ChromeFrameOversized(length));
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).await.map_err(map_truncated)?;
    Ok(Some(body))
}

pub async fn write_chrome_frame<W>(writer: &mut W, body: &[u8]) -> Result<(), NativeHostError>
where
    W: AsyncWrite + Unpin,
{
    if body.is_empty() {
        return Err(NativeHostError::ChromeFrameEmpty);
    }
    // This also remains below Chrome's 1 MiB host-to-Chrome allowance.
    if body.len() > MAX_FRAME_BYTES {
        return Err(NativeHostError::ChromeFrameOversized(body.len()));
    }
    let length =
        u32::try_from(body.len()).map_err(|_| NativeHostError::ChromeFrameOversized(body.len()))?;
    writer.write_all(&length.to_ne_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_chrome_envelope<R>(
    reader: &mut R,
) -> Result<Option<WireEnvelope>, NativeHostError>
where
    R: AsyncRead + Unpin,
{
    let Some(frame) = read_chrome_frame(reader).await? else {
        return Ok(None);
    };
    let envelope: WireEnvelope = serde_json::from_slice(&frame)?;
    envelope.validate_shape()?;
    Ok(Some(envelope))
}

pub async fn write_chrome_envelope<W>(
    writer: &mut W,
    envelope: &WireEnvelope,
) -> Result<(), NativeHostError>
where
    W: AsyncWrite + Unpin,
{
    envelope.validate_shape()?;
    let body = serde_json::to_vec(envelope)?;
    write_chrome_frame(writer, &body).await
}

pub async fn connect_and_bridge<R, W>(
    socket_path: &Path,
    chrome_input: R,
    chrome_output: W,
) -> Result<(), NativeHostError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    verify_socket_metadata(socket_path).map_err(NativeHostError::BrokerFrame)?;
    let broker = UnixStream::connect(socket_path)
        .await
        .map_err(NativeHostError::BrokerConnect)?;
    verify_peer_uid(&broker).map_err(NativeHostError::BrokerFrame)?;
    bridge_streams(chrome_input, chrome_output, broker).await
}

pub async fn bridge_streams<R, W>(
    mut chrome_input: R,
    mut chrome_output: W,
    broker: UnixStream,
) -> Result<(), NativeHostError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut broker_reader, mut broker_writer) = broker.into_split();
    let chrome_to_broker = async {
        loop {
            let Some(envelope) = read_chrome_envelope(&mut chrome_input).await? else {
                broker_writer
                    .shutdown()
                    .await
                    .map_err(NativeHostError::BrokerConnect)?;
                return Ok::<(), NativeHostError>(());
            };
            write_envelope(&mut broker_writer, &envelope)
                .await
                .map_err(NativeHostError::BrokerFrame)?;
        }
    };
    let broker_to_chrome = async {
        loop {
            let Some(envelope) = read_envelope(&mut broker_reader)
                .await
                .map_err(NativeHostError::BrokerFrame)?
            else {
                chrome_output.shutdown().await?;
                return Ok::<(), NativeHostError>(());
            };
            write_chrome_envelope(&mut chrome_output, &envelope).await?;
        }
    };
    tokio::pin!(chrome_to_broker);
    tokio::pin!(broker_to_chrome);

    tokio::select! {
        inbound = &mut chrome_to_broker => {
            inbound?;
            normalize_chrome_output(broker_to_chrome.await)
        }
        outbound = &mut broker_to_chrome => normalize_chrome_output(outbound),
    }
}

fn normalize_chrome_output(result: Result<(), NativeHostError>) -> Result<(), NativeHostError> {
    match result {
        Err(NativeHostError::ChromeIo(error)) if error.kind() == io::ErrorKind::BrokenPipe => {
            Ok(())
        }
        other => other,
    }
}

fn map_truncated(error: io::Error) -> NativeHostError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        NativeHostError::ChromeFrameTruncated
    } else {
        NativeHostError::ChromeIo(error)
    }
}

#[derive(Serialize)]
struct NativeMessagingManifest<'a> {
    name: &'static str,
    description: &'static str,
    path: &'a str,
    #[serde(rename = "type")]
    transport_type: &'static str,
    allowed_origins: [String; 1],
}

pub fn render_native_manifest(host_path: &Path) -> Result<String, ManifestError> {
    let path = validate_manifest_host_path(host_path)?;
    let manifest = NativeMessagingManifest {
        name: NATIVE_HOST_NAME,
        description: "Omatype private local broker bridge",
        path,
        transport_type: "stdio",
        allowed_origins: [development_extension_origin()],
    };
    let mut output = serde_json::to_string_pretty(&manifest)?;
    output.push('\n');
    Ok(output)
}

fn validate_manifest_host_path(path: &Path) -> Result<&str, ManifestError> {
    if !path.is_absolute()
        || path.file_name().is_none()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(ManifestError::HostPath);
    }
    let path = path.to_str().ok_or(ManifestError::HostPath)?;
    if path.chars().any(char::is_control) {
        Err(ManifestError::HostPath)
    } else {
        Ok(path)
    }
}

#[derive(Debug, Error)]
pub enum NativeHostError {
    #[error("broker_connect")]
    BrokerConnect(#[source] io::Error),
    #[error("broker_frame")]
    BrokerFrame(#[source] FrameError),
    #[error("caller_origin")]
    CallerOrigin,
    #[error("chrome_frame_empty")]
    ChromeFrameEmpty,
    #[error("chrome_frame_oversized")]
    ChromeFrameOversized(usize),
    #[error("chrome_frame_truncated")]
    ChromeFrameTruncated,
    #[error("chrome_io")]
    ChromeIo(#[from] io::Error),
    #[error("invalid_json")]
    InvalidJson(#[from] serde_json::Error),
    #[error("invalid_protocol")]
    InvalidProtocol(#[from] crate::protocol::ProtocolError),
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("host_path")]
    HostPath,
    #[error("serialize")]
    Serialize(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};

    use super::{
        DEVELOPMENT_EXTENSION_ID, ManifestError, NativeHostError, development_extension_origin,
        read_chrome_envelope, read_chrome_frame, render_native_manifest,
        validate_development_caller_origin, write_chrome_frame,
    };
    use crate::protocol::{EmptyPayload, MAX_FRAME_BYTES, MessageType, WireEnvelope};

    #[test]
    fn development_origin_is_exact_and_not_broad() {
        assert_eq!(DEVELOPMENT_EXTENSION_ID.len(), 32);
        assert!(
            DEVELOPMENT_EXTENSION_ID
                .bytes()
                .all(|byte| (b'a'..=b'p').contains(&byte))
        );
        let expected = "chrome-extension://ckkiehcjbclcjckkkajohopoikeejkoa/";
        assert_eq!(development_extension_origin(), expected);
        assert!(validate_development_caller_origin(expected).is_ok());
        for rejected in [
            "chrome-extension://ckkiehcjbclcjckkkajohopoikeejkoa/*",
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
            "https://ckkiehcjbclcjckkkajohopoikeejkoa/",
            "*",
        ] {
            assert!(matches!(
                validate_development_caller_origin(rejected),
                Err(NativeHostError::CallerOrigin)
            ));
        }
    }

    #[test]
    fn manifest_is_deterministic_and_print_only_data() {
        let rendered = render_native_manifest(Path::new("/opt/omatype/omatype-native-host"))
            .expect("manifest");
        let expected = "{\n  \"name\": \"io.omatype.broker\",\n  \"description\": \"Omatype private local broker bridge\",\n  \"path\": \"/opt/omatype/omatype-native-host\",\n  \"type\": \"stdio\",\n  \"allowed_origins\": [\n    \"chrome-extension://ckkiehcjbclcjckkkajohopoikeejkoa/\"\n  ]\n}\n";
        assert_eq!(rendered, expected);
        assert_eq!(
            render_native_manifest(Path::new("/opt/omatype/omatype-native-host"))
                .expect("second manifest"),
            rendered
        );
        for rejected in ["relative/host", "/", "/opt/../tmp/host"] {
            assert!(matches!(
                render_native_manifest(Path::new(rejected)),
                Err(ManifestError::HostPath)
            ));
        }
    }

    #[tokio::test]
    async fn reads_fragmented_native_endian_chrome_frame() {
        let envelope =
            WireEnvelope::global(MessageType::HealthRequest, 7, &EmptyPayload::default())
                .expect("envelope");
        let body = serde_json::to_vec(&envelope).expect("JSON");
        let mut framed = Vec::with_capacity(4 + body.len());
        framed.extend_from_slice(
            &u32::try_from(body.len())
                .expect("test frame length")
                .to_ne_bytes(),
        );
        framed.extend_from_slice(&body);
        let (mut sender, mut receiver) = duplex(1);
        let send = tokio::spawn(async move {
            for byte in framed {
                sender.write_all(&[byte]).await.expect("fragment");
                tokio::task::yield_now().await;
            }
        });

        let decoded = read_chrome_envelope(&mut receiver)
            .await
            .expect("framed envelope")
            .expect("one envelope");
        send.await.expect("fragment sender");
        assert_eq!(decoded, envelope);
    }

    #[tokio::test]
    async fn rejects_65537_declared_bytes_before_reading_a_body() {
        let header = u32::try_from(MAX_FRAME_BYTES + 1)
            .expect("test length")
            .to_ne_bytes();
        let mut reader = std::io::Cursor::new(header);
        assert!(matches!(
            read_chrome_frame(&mut reader).await,
            Err(NativeHostError::ChromeFrameOversized(size)) if size == MAX_FRAME_BYTES + 1
        ));
    }

    #[tokio::test]
    async fn rejects_empty_and_truncated_chrome_frames() {
        let mut empty = std::io::Cursor::new(0_u32.to_ne_bytes());
        assert!(matches!(
            read_chrome_frame(&mut empty).await,
            Err(NativeHostError::ChromeFrameEmpty)
        ));

        let mut partial_header = std::io::Cursor::new(vec![1, 0]);
        assert!(matches!(
            read_chrome_frame(&mut partial_header).await,
            Err(NativeHostError::ChromeFrameTruncated)
        ));

        let mut partial_body =
            std::io::Cursor::new([2_u32.to_ne_bytes().as_slice(), b"{"].concat());
        assert!(matches!(
            read_chrome_frame(&mut partial_body).await,
            Err(NativeHostError::ChromeFrameTruncated)
        ));
    }

    #[tokio::test]
    async fn writes_native_endian_and_enforces_internal_limit() {
        let (mut writer, mut reader) = duplex(16);
        let send = tokio::spawn(async move { write_chrome_frame(&mut writer, b"{}").await });
        let mut raw = [0_u8; 6];
        reader.read_exact(&mut raw).await.expect("frame");
        send.await.expect("send task").expect("send frame");
        assert_eq!(&raw[..4], &2_u32.to_ne_bytes());
        assert_eq!(&raw[4..], b"{}");

        let mut sink = tokio::io::sink();
        assert!(matches!(
            write_chrome_frame(&mut sink, &vec![0; MAX_FRAME_BYTES + 1]).await,
            Err(NativeHostError::ChromeFrameOversized(size)) if size == MAX_FRAME_BYTES + 1
        ));
    }
}
