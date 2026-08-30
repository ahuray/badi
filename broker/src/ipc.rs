use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::protocol::{MAX_FRAME_BYTES, WireEnvelope};

pub async fn read_frame<R>(reader: &mut R) -> Result<Option<Vec<u8>>, FrameError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; 4];
    let first = reader.read(&mut header[..1]).await?;
    if first == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..]).await.map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            FrameError::Io(error)
        }
    })?;
    let length = usize::try_from(u32::from_le_bytes(header)).unwrap_or(usize::MAX);
    if length == 0 {
        return Err(FrameError::Empty);
    }
    if length > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized(length));
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).await.map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Truncated
        } else {
            FrameError::Io(error)
        }
    })?;
    Ok(Some(body))
}

pub async fn write_frame<W>(writer: &mut W, body: &[u8]) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
{
    if body.is_empty() {
        return Err(FrameError::Empty);
    }
    if body.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized(body.len()));
    }
    let length = u32::try_from(body.len()).map_err(|_| FrameError::Oversized(body.len()))?;
    writer.write_all(&length.to_le_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_envelope<R>(reader: &mut R) -> Result<Option<WireEnvelope>, FrameError>
where
    R: AsyncRead + Unpin,
{
    let Some(frame) = read_frame(reader).await? else {
        return Ok(None);
    };
    let envelope: WireEnvelope = serde_json::from_slice(&frame)?;
    envelope.validate_shape()?;
    Ok(Some(envelope))
}

pub async fn write_envelope<W>(writer: &mut W, envelope: &WireEnvelope) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
{
    envelope.validate_shape()?;
    let body = serde_json::to_vec(envelope)?;
    write_frame(writer, &body).await
}

pub fn default_socket_path() -> Result<PathBuf, FrameError> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or(FrameError::MissingRuntimeDir)?;
    let runtime = PathBuf::from(runtime);
    if !runtime.is_absolute() {
        return Err(FrameError::InvalidRuntimeDir);
    }
    Ok(runtime.join("badi").join("broker.sock"))
}

#[cfg(target_os = "linux")]
pub fn verify_peer_uid(stream: &UnixStream) -> Result<(), FrameError> {
    let peer_uid = stream.peer_cred()?.uid();
    let process_uid = rustix::process::getuid().as_raw();
    if peer_uid == process_uid {
        Ok(())
    } else {
        Err(FrameError::PeerUidMismatch)
    }
}

#[cfg(not(target_os = "linux"))]
pub fn verify_peer_uid(_stream: &UnixStream) -> Result<(), FrameError> {
    Err(FrameError::PeerCredentialsUnavailable)
}

#[cfg(target_os = "linux")]
pub fn verify_socket_metadata(path: &Path) -> Result<(), FrameError> {
    use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};

    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() {
        return Err(FrameError::UnsafeSocket);
    }
    if metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(FrameError::UnsafeSocket);
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn verify_socket_metadata(_path: &Path) -> Result<(), FrameError> {
    Err(FrameError::PeerCredentialsUnavailable)
}

pub fn validate_json_frame(frame: &[u8]) -> Result<Value, FrameError> {
    if frame.is_empty() {
        return Err(FrameError::Empty);
    }
    if frame.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized(frame.len()));
    }
    Ok(serde_json::from_slice(frame)?)
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("empty_frame")]
    Empty,
    #[error("invalid_runtime_dir")]
    InvalidRuntimeDir,
    #[error("io")]
    Io(#[from] io::Error),
    #[error("missing_runtime_dir")]
    MissingRuntimeDir,
    #[error("oversized_frame:{0}")]
    Oversized(usize),
    #[error("peer_credentials_unavailable")]
    PeerCredentialsUnavailable,
    #[error("peer_uid_mismatch")]
    PeerUidMismatch,
    #[error("protocol")]
    Protocol(#[from] crate::protocol::ProtocolError),
    #[error("invalid_json")]
    Serde(#[from] serde_json::Error),
    #[error("truncated_frame")]
    Truncated,
    #[error("unsafe_socket")]
    UnsafeSocket,
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, duplex};

    use super::{FrameError, read_frame, validate_json_frame, write_frame};
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

    #[test]
    fn rejects_malformed_and_invalid_utf8_json() {
        assert!(matches!(
            validate_json_frame(b"{"),
            Err(FrameError::Serde(_))
        ));
        assert!(matches!(
            validate_json_frame(&[0xff, 0xfe]),
            Err(FrameError::Serde(_))
        ));
    }
}
