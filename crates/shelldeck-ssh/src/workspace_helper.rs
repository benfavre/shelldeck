//! Fixed-subsystem protocol for an SSH workspace terminal.
//!
//! The protocol deliberately has no command string. The client asks sshd for
//! [`WORKSPACE_SUBSYSTEM`], then sends bounded, typed frames. A trusted remote
//! helper retains the opened workspace directory until the client either
//! releases it or resumes an interactive shell.

use crate::{Result, SshError};
use russh::client;
use russh::{Channel, ChannelMsg, Pty};
use std::io::Cursor;
use uuid::Uuid;

pub const WORKSPACE_SUBSYSTEM: &str = "shelldeck-workspace-v1";
pub const MAX_FRAME_BYTES: usize = 8 * 1024;
const VERSION: u8 = 1;
const TOKEN_BYTES: usize = 32;

const OP_PREPARE: u8 = 1;
const OP_RESUME: u8 = 2;
const OP_RELEASE: u8 = 3;
const OP_PREPARED: u8 = 0x81;
const OP_READY: u8 = 0x82;
const OP_RELEASED: u8 = 0x83;
const OP_ERROR: u8 = 0xff;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePrepareRequest {
    pub operation: Uuid,
    pub workspace: Uuid,
    pub remote_root: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePreparedReceipt {
    token: [u8; TOKEN_BYTES],
    pub operation: Uuid,
    pub workspace: Uuid,
    pub directory_device: u64,
    pub directory_inode: u64,
    pub head_oid: String,
    pub branch: String,
}

impl WorkspacePreparedReceipt {
    #[must_use]
    pub fn matches(&self, operation: Uuid, workspace: Uuid) -> bool {
        self.operation == operation && self.workspace == workspace
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum WorkspaceHelperErrorCode {
    InvalidRequest = 1,
    UnauthorizedRoot = 2,
    WorkspaceUnavailable = 3,
    RepositoryUnavailable = 4,
    DirtyWorkspace = 5,
    StaleReceipt = 6,
    ShellUnavailable = 7,
    Internal = 255,
}

impl WorkspaceHelperErrorCode {
    fn decode(value: u16) -> Option<Self> {
        Some(match value {
            1 => Self::InvalidRequest,
            2 => Self::UnauthorizedRoot,
            3 => Self::WorkspaceUnavailable,
            4 => Self::RepositoryUnavailable,
            5 => Self::DirtyWorkspace,
            6 => Self::StaleReceipt,
            7 => Self::ShellUnavailable,
            255 => Self::Internal,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceHelperFailure {
    pub code: WorkspaceHelperErrorCode,
    pub retryable: bool,
}

impl std::fmt::Display for WorkspaceHelperFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "workspace helper refused request ({:?})",
            self.code
        )
    }
}

impl std::error::Error for WorkspaceHelperFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RequestFrame {
    Prepare(WorkspacePrepareRequest),
    Resume([u8; TOKEN_BYTES]),
    Release([u8; TOKEN_BYTES]),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ResponseFrame {
    Prepared(WorkspacePreparedReceipt),
    Ready([u8; TOKEN_BYTES]),
    Released([u8; TOKEN_BYTES]),
    Error(WorkspaceHelperFailure),
}

pub struct WorkspaceHelperChannel {
    channel: Channel<client::Msg>,
    buffered: Vec<u8>,
}

impl WorkspaceHelperChannel {
    pub(crate) fn new(channel: Channel<client::Msg>) -> Self {
        Self {
            channel,
            buffered: Vec::new(),
        }
    }

    pub async fn prepare(
        &mut self,
        request: WorkspacePrepareRequest,
    ) -> Result<WorkspacePreparedReceipt> {
        self.send_request(&RequestFrame::Prepare(request.clone()))
            .await?;
        match self.read_response().await? {
            ResponseFrame::Prepared(receipt)
                if receipt.matches(request.operation, request.workspace) =>
            {
                Ok(receipt)
            }
            ResponseFrame::Error(error) => Err(helper_failure(error)),
            _ => Err(protocol_error("mismatched prepare response")),
        }
    }

    pub async fn resume(
        mut self,
        receipt: &WorkspacePreparedReceipt,
    ) -> Result<crate::session::SshChannel> {
        self.send_request(&RequestFrame::Resume(receipt.token))
            .await?;
        match self.read_response().await? {
            ResponseFrame::Ready(token) if token == receipt.token => Ok(
                crate::session::SshChannel::from_workspace_helper(self.channel),
            ),
            ResponseFrame::Error(error) => Err(helper_failure(error)),
            _ => Err(protocol_error("mismatched resume response")),
        }
    }

    pub async fn release(mut self, receipt: &WorkspacePreparedReceipt) -> Result<()> {
        self.send_request(&RequestFrame::Release(receipt.token))
            .await?;
        match self.read_response().await? {
            ResponseFrame::Released(token) if token == receipt.token => {
                self.channel
                    .eof()
                    .await
                    .map_err(|error| SshError::Channel(error.to_string()))?;
                Ok(())
            }
            ResponseFrame::Error(error) => Err(helper_failure(error)),
            _ => Err(protocol_error("mismatched release response")),
        }
    }

    async fn send_request(&self, request: &RequestFrame) -> Result<()> {
        let frame = encode_request(request).map_err(protocol_io)?;
        self.channel
            .data(Cursor::new(frame))
            .await
            .map_err(|error| SshError::Channel(error.to_string()))
    }

    async fn read_response(&mut self) -> Result<ResponseFrame> {
        loop {
            if let Some(payload) = take_buffered_frame(&mut self.buffered).map_err(protocol_io)? {
                return decode_response(&payload).map_err(protocol_io);
            }
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    if self.buffered.len().saturating_add(data.len()) > MAX_FRAME_BYTES + 4 {
                        return Err(protocol_error("workspace helper response exceeded limit"));
                    }
                    self.buffered.extend_from_slice(&data);
                }
                Some(ChannelMsg::Failure) => {
                    return Err(protocol_error("workspace subsystem was refused"));
                }
                Some(ChannelMsg::ExtendedData { .. }) => {
                    return Err(protocol_error("workspace helper wrote diagnostic data"));
                }
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => {
                    return Err(SshError::SessionClosed);
                }
                _ => {}
            }
        }
    }
}

fn helper_failure(error: WorkspaceHelperFailure) -> SshError {
    SshError::Channel(error.to_string())
}

fn protocol_error(message: &str) -> SshError {
    SshError::Channel(message.to_owned())
}

fn protocol_io(error: std::io::Error) -> SshError {
    SshError::Channel(error.to_string())
}

fn frame(payload: Vec<u8>) -> std::io::Result<Vec<u8>> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(invalid_data());
    }
    let mut output = Vec::with_capacity(payload.len() + 4);
    output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    output.extend_from_slice(&payload);
    Ok(output)
}

fn take_buffered_frame(buffer: &mut Vec<u8>) -> std::io::Result<Option<Vec<u8>>> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let length = u32::from_be_bytes(buffer[..4].try_into().expect("four byte prefix")) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(invalid_data());
    }
    if buffer.len() < length + 4 {
        return Ok(None);
    }
    let payload = buffer[4..length + 4].to_vec();
    buffer.drain(..length + 4);
    Ok(Some(payload))
}

fn encode_request(request: &RequestFrame) -> std::io::Result<Vec<u8>> {
    let mut payload = vec![VERSION];
    match request {
        RequestFrame::Prepare(request) => {
            payload.push(OP_PREPARE);
            payload.extend_from_slice(request.operation.as_bytes());
            payload.extend_from_slice(request.workspace.as_bytes());
            put_string(&mut payload, &request.remote_root)?;
        }
        RequestFrame::Resume(token) => {
            payload.push(OP_RESUME);
            payload.extend_from_slice(token);
        }
        RequestFrame::Release(token) => {
            payload.push(OP_RELEASE);
            payload.extend_from_slice(token);
        }
    }
    frame(payload)
}

fn decode_request(payload: &[u8]) -> std::io::Result<RequestFrame> {
    let mut decoder = Decoder::new(payload);
    decoder.version()?;
    let request = match decoder.byte()? {
        OP_PREPARE => RequestFrame::Prepare(WorkspacePrepareRequest {
            operation: decoder.uuid()?,
            workspace: decoder.uuid()?,
            remote_root: decoder.string(4096)?,
        }),
        OP_RESUME => RequestFrame::Resume(decoder.token()?),
        OP_RELEASE => RequestFrame::Release(decoder.token()?),
        _ => return Err(invalid_data()),
    };
    decoder.finish()?;
    Ok(request)
}

fn encode_response(response: &ResponseFrame) -> std::io::Result<Vec<u8>> {
    let mut payload = vec![VERSION];
    match response {
        ResponseFrame::Prepared(receipt) => {
            payload.push(OP_PREPARED);
            payload.extend_from_slice(&receipt.token);
            payload.extend_from_slice(receipt.operation.as_bytes());
            payload.extend_from_slice(receipt.workspace.as_bytes());
            payload.extend_from_slice(&receipt.directory_device.to_be_bytes());
            payload.extend_from_slice(&receipt.directory_inode.to_be_bytes());
            put_string(&mut payload, &receipt.head_oid)?;
            put_string(&mut payload, &receipt.branch)?;
        }
        ResponseFrame::Ready(token) => {
            payload.push(OP_READY);
            payload.extend_from_slice(token);
        }
        ResponseFrame::Released(token) => {
            payload.push(OP_RELEASED);
            payload.extend_from_slice(token);
        }
        ResponseFrame::Error(error) => {
            payload.push(OP_ERROR);
            payload.extend_from_slice(&(error.code as u16).to_be_bytes());
            payload.push(u8::from(error.retryable));
        }
    }
    frame(payload)
}

fn decode_response(payload: &[u8]) -> std::io::Result<ResponseFrame> {
    let mut decoder = Decoder::new(payload);
    decoder.version()?;
    let response = match decoder.byte()? {
        OP_PREPARED => ResponseFrame::Prepared(WorkspacePreparedReceipt {
            token: decoder.token()?,
            operation: decoder.uuid()?,
            workspace: decoder.uuid()?,
            directory_device: decoder.u64()?,
            directory_inode: decoder.u64()?,
            head_oid: decoder.string(128)?,
            branch: decoder.string(255)?,
        }),
        OP_READY => ResponseFrame::Ready(decoder.token()?),
        OP_RELEASED => ResponseFrame::Released(decoder.token()?),
        OP_ERROR => ResponseFrame::Error(WorkspaceHelperFailure {
            code: WorkspaceHelperErrorCode::decode(decoder.u16()?).ok_or_else(invalid_data)?,
            retryable: match decoder.byte()? {
                0 => false,
                1 => true,
                _ => return Err(invalid_data()),
            },
        }),
        _ => return Err(invalid_data()),
    };
    decoder.finish()?;
    Ok(response)
}

fn put_string(output: &mut Vec<u8>, value: &str) -> std::io::Result<()> {
    let length = u16::try_from(value.len()).map_err(|_| invalid_data())?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn invalid_data() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid workspace helper frame",
    )
}

struct Decoder<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    fn version(&mut self) -> std::io::Result<()> {
        if self.byte()? == VERSION {
            Ok(())
        } else {
            Err(invalid_data())
        }
    }

    fn take(&mut self, length: usize) -> std::io::Result<&'a [u8]> {
        let end = self.position.checked_add(length).ok_or_else(invalid_data)?;
        let value = self
            .input
            .get(self.position..end)
            .ok_or_else(invalid_data)?;
        self.position = end;
        Ok(value)
    }

    fn byte(&mut self) -> std::io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> std::io::Result<u16> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("two byte field"),
        ))
    }

    fn u64(&mut self) -> std::io::Result<u64> {
        Ok(u64::from_be_bytes(
            self.take(8)?.try_into().expect("eight byte field"),
        ))
    }

    fn uuid(&mut self) -> std::io::Result<Uuid> {
        Uuid::from_slice(self.take(16)?).map_err(|_| invalid_data())
    }

    fn token(&mut self) -> std::io::Result<[u8; TOKEN_BYTES]> {
        Ok(self
            .take(TOKEN_BYTES)?
            .try_into()
            .expect("fixed receipt token"))
    }

    fn string(&mut self, max: usize) -> std::io::Result<String> {
        let length = self.u16()? as usize;
        if length > max {
            return Err(invalid_data());
        }
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| invalid_data())
    }

    fn finish(&self) -> std::io::Result<()> {
        if self.position == self.input.len() {
            Ok(())
        } else {
            Err(invalid_data())
        }
    }
}

pub(crate) fn raw_workspace_pty_modes() -> Vec<(Pty, u32)> {
    vec![
        (Pty::ISIG, 0),
        (Pty::ICANON, 0),
        (Pty::ECHO, 0),
        (Pty::ECHOE, 0),
        (Pty::ECHOK, 0),
        (Pty::ECHONL, 0),
        (Pty::ECHOCTL, 0),
        (Pty::IEXTEN, 0),
        (Pty::IXON, 0),
        (Pty::IXOFF, 0),
        (Pty::ICRNL, 0),
        (Pty::INLCR, 0),
        (Pty::IGNCR, 0),
        (Pty::OPOST, 0),
        (Pty::CS8, 1),
    ]
}

#[cfg(unix)]
pub mod remote;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_round_trips_exact_coordinates_and_receipt() {
        let request = RequestFrame::Prepare(WorkspacePrepareRequest {
            operation: Uuid::from_u128(1),
            workspace: Uuid::from_u128(2),
            remote_root: "/srv/workspaces/repo".into(),
        });
        let encoded = encode_request(&request).unwrap();
        let mut buffered = encoded;
        let payload = take_buffered_frame(&mut buffered).unwrap().unwrap();
        assert_eq!(decode_request(&payload).unwrap(), request);
        assert!(buffered.is_empty());

        let response = ResponseFrame::Prepared(WorkspacePreparedReceipt {
            token: [7; TOKEN_BYTES],
            operation: Uuid::from_u128(1),
            workspace: Uuid::from_u128(2),
            directory_device: 3,
            directory_inode: 4,
            head_oid: "a".repeat(40),
            branch: "main".into(),
        });
        let encoded = encode_response(&response).unwrap();
        assert_eq!(
            decode_response(&encoded[4..]).unwrap(),
            response,
            "the opaque token and all lineage coordinates stay exact"
        );
    }

    #[test]
    fn protocol_rejects_oversize_unknown_and_trailing_frames() {
        let mut oversize = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec();
        oversize.extend_from_slice(&[0; 2]);
        assert!(take_buffered_frame(&mut oversize).is_err());
        assert!(decode_request(&[VERSION, 99]).is_err());

        let mut valid = encode_request(&RequestFrame::Resume([1; TOKEN_BYTES])).unwrap();
        valid.push(0);
        assert!(decode_request(&valid[4..]).is_err());
    }

    #[test]
    fn errors_expose_only_bounded_codes() {
        let response = ResponseFrame::Error(WorkspaceHelperFailure {
            code: WorkspaceHelperErrorCode::UnauthorizedRoot,
            retryable: false,
        });
        let encoded = encode_response(&response).unwrap();
        assert_eq!(decode_response(&encoded[4..]).unwrap(), response);
        assert!(encoded.len() < 16);
    }
}
