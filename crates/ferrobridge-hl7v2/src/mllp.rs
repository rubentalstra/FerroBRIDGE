// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The MLLP Release 1 framing and the listener that answers each frame.
//!
//! A frame is `<SB> dddd <EB> <CR>`: the start block `0x0B`, the message
//! bytes, the end block `0x1C` and a carriage return `0x0D` (HL7 Version 3
//! Standard, Transport Specification, MLLP Release 1, §Block format). The
//! message bytes may hold neither block character, so a start block inside a
//! frame, an end block not followed by the carriage return, bytes before the
//! start block, and a frame past the configured ceiling are each a typed
//! refusal ([`FrameError`]). Release 2's reliable delivery (the commit
//! acknowledgment `<SB><ACK><EB><CR>`) is not spoken.
//!
//! A connection that sits between frames past its idle timeout is closed, and
//! a frame that does not complete within its frame timeout is refused as
//! [`Malformed::Stalled`] ([`Timeouts`]; no specification governs either
//! bound: our own design).

use core::future::Future;
use core::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::{Buf, BufMut, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio_util::codec::{Decoder, Encoder};
use tracing::Instrument;

/// The start block, `0x0B`.
pub const START_BLOCK: u8 = 0x0B;

/// The end block, `0x1C`.
pub const END_BLOCK: u8 = 0x1C;

/// The carriage return that closes a frame, `0x0D`.
pub const CARRIAGE_RETURN: u8 = 0x0D;

/// How many bytes one read of a connection asks for.
const READ_CHUNK: usize = 8 * 1024;

/// Why a byte stream is no MLLP frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    /// A byte other than the start block opens the frame.
    NoStartBlock,
    /// A start block occurs inside the frame.
    StartBlockInFrame,
    /// The end block is followed by a byte other than the carriage return.
    MissingTrailer,
    /// The frame runs past the configured ceiling before its end block.
    TooLarge {
        /// The ceiling, in message bytes.
        limit: usize,
    },
    /// The peer closed the connection inside a frame.
    Truncated,
    /// The frame did not complete within the frame timeout.
    Stalled {
        /// The frame timeout.
        limit: Duration,
    },
}

impl core::fmt::Display for Malformed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoStartBlock => f.write_str("the frame does not open with the start block 0x0B"),
            Self::StartBlockInFrame => f.write_str("a start block 0x0B occurs inside the frame"),
            Self::MissingTrailer => {
                f.write_str("the end block 0x1C is not followed by the carriage return 0x0D")
            }
            Self::TooLarge { limit } => {
                write!(f, "the frame runs past the ceiling of {limit} bytes")
            }
            Self::Truncated => f.write_str("the connection closed inside a frame"),
            Self::Stalled { limit } => write!(
                f,
                "the frame did not complete within {} ms",
                limit.as_millis()
            ),
        }
    }
}

/// A refusal of the frame codec.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The bytes are no MLLP frame.
    #[error("malformed MLLP frame: {kind}")]
    Malformed {
        /// Why.
        kind: Malformed,
        /// The message bytes read before the refusal, from which a header may
        /// still be read for the reject acknowledgment.
        partial: Vec<u8>,
    },
    /// A message handed to the encoder holds a block character.
    #[error("the message holds a block character and cannot be framed")]
    Unframable,
    /// The connection failed.
    #[error("the MLLP connection failed")]
    Io(#[from] std::io::Error),
}

/// The MLLP frame codec over the `tokio-util` codec traits.
#[derive(Debug, Clone, Copy)]
pub struct Codec {
    limit: usize,
}

impl Codec {
    /// Creates a codec refusing a frame of more than `limit` message bytes.
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self { limit }
    }

    /// Returns the frame ceiling, in message bytes.
    #[must_use]
    pub const fn limit(self) -> usize {
        self.limit
    }
}

impl Default for Codec {
    fn default() -> Self {
        // NOTE: no specification governs this: our own design; one MiB holds
        // any laboratory result and bounds what one connection can buffer.
        Self { limit: 1 << 20 }
    }
}

impl Decoder for Codec {
    type Item = Vec<u8>;
    type Error = FrameError;

    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let Some(first) = buffer.first().copied() else {
            return Ok(None);
        };
        if first != START_BLOCK {
            // NOTE: no specification governs this: our own design; the refusal waits for the
            // end block or the ceiling, so the reject can be addressed from the header.
            let end = buffer.iter().position(|byte| *byte == END_BLOCK);
            if end.is_none() && buffer.len() <= self.limit {
                return Ok(None);
            }
            let partial = buffer
                .get(..end.unwrap_or(buffer.len()))
                .unwrap_or_default()
                .to_vec();
            buffer.clear();
            return Err(FrameError::Malformed {
                kind: Malformed::NoStartBlock,
                partial,
            });
        }
        let body = buffer.get(1..).unwrap_or_default();
        let frame_end = body
            .iter()
            .position(|byte| *byte == END_BLOCK)
            .unwrap_or(body.len());
        let frame = body.get(..frame_end).unwrap_or_default();
        if let Some(inner) = frame.iter().position(|byte| *byte == START_BLOCK) {
            let partial = body.get(..inner).unwrap_or_default().to_vec();
            buffer.clear();
            return Err(FrameError::Malformed {
                kind: Malformed::StartBlockInFrame,
                partial,
            });
        }
        let Some(end) = body.iter().position(|byte| *byte == END_BLOCK) else {
            if body.len() > self.limit {
                let partial = body.to_vec();
                buffer.clear();
                return Err(FrameError::Malformed {
                    kind: Malformed::TooLarge { limit: self.limit },
                    partial,
                });
            }
            return Ok(None);
        };
        if end > self.limit {
            let partial = body.get(..end).unwrap_or_default().to_vec();
            buffer.clear();
            return Err(FrameError::Malformed {
                kind: Malformed::TooLarge { limit: self.limit },
                partial,
            });
        }
        match body.get(end.saturating_add(1)).copied() {
            None => Ok(None),
            Some(CARRIAGE_RETURN) => {
                let message = body.get(..end).unwrap_or_default().to_vec();
                buffer.advance(end.saturating_add(3));
                Ok(Some(message))
            }
            Some(_) => {
                let partial = body.get(..end).unwrap_or_default().to_vec();
                buffer.clear();
                Err(FrameError::Malformed {
                    kind: Malformed::MissingTrailer,
                    partial,
                })
            }
        }
    }

    fn decode_eof(&mut self, buffer: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if let Some(message) = self.decode(buffer)? {
            return Ok(Some(message));
        }
        let Some(first) = buffer.first().copied() else {
            return Ok(None);
        };
        let (kind, partial) = if first == START_BLOCK {
            (
                Malformed::Truncated,
                buffer.get(1..).unwrap_or_default().to_vec(),
            )
        } else {
            (Malformed::NoStartBlock, buffer.to_vec())
        };
        buffer.clear();
        Err(FrameError::Malformed { kind, partial })
    }
}

impl Encoder<&[u8]> for Codec {
    type Error = FrameError;

    fn encode(&mut self, message: &[u8], buffer: &mut BytesMut) -> Result<(), Self::Error> {
        if message
            .iter()
            .any(|byte| *byte == START_BLOCK || *byte == END_BLOCK)
        {
            return Err(FrameError::Unframable);
        }
        buffer.reserve(message.len().saturating_add(3));
        buffer.put_u8(START_BLOCK);
        buffer.put_slice(message);
        buffer.put_u8(END_BLOCK);
        buffer.put_u8(CARRIAGE_RETURN);
        Ok(())
    }
}

/// What one connection counts, recorded on its `mllp_connection` span.
///
/// The listener counts the frames it answered; the handler counts the frames
/// it refused by its own policy before handling them (a sender it does not
/// accept, say), so the span says how many of a peer's messages were turned
/// away.
#[derive(Debug, Default)]
pub struct Connection {
    refused: AtomicUsize,
}

impl Connection {
    /// Counts one frame the handler refused by its own policy.
    pub fn refuse(&self) {
        self.refused.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns how many frames the handler refused.
    #[must_use]
    pub fn refused(&self) -> usize {
        self.refused.load(Ordering::Relaxed)
    }
}

/// What the listener hands each frame to.
///
/// The answer is the acknowledgment's bytes, framed by the listener. A
/// handler that cannot answer a frame at all returns `None`, and the listener
/// closes the connection.
pub trait Handler: Send + Sync + 'static {
    /// Answers one frame's message bytes, counting on `connection` what the
    /// connection's span reports.
    fn handle(
        &self,
        message: Vec<u8>,
        connection: &Connection,
    ) -> impl Future<Output = Option<Vec<u8>>> + Send;

    /// Answers a malformed frame from the message bytes read before the
    /// refusal, before the listener closes the connection.
    ///
    /// The default answers nothing; [`crate::ack::reject_frame`] builds the
    /// reject acknowledgment when a header can still be read.
    fn malformed(&self, kind: Malformed, partial: &[u8]) -> Option<Vec<u8>> {
        let (_, _) = (kind, partial);
        None
    }
}

/// How long one connection may wait on its peer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timeouts {
    /// How long a connection may sit with no frame begun before it is
    /// closed; `None` waits for as long as the peer keeps it open.
    pub idle: Option<Duration>,
    /// How long a frame may take from its first byte to its trailer before it
    /// is refused as [`Malformed::Stalled`]; `None` waits for the trailer.
    pub frame: Option<Duration>,
}

/// Accepts connections on `listener` until `shutdown` completes, answering
/// every frame through `handler`, with no timeout on a connection.
///
/// [`serve_with`] states the timeouts.
///
/// # Errors
///
/// Returns the I/O error of a failed `accept`.
pub async fn serve<H: Handler>(
    listener: TcpListener,
    handler: Arc<H>,
    codec: Codec,
    shutdown: impl Future<Output = ()> + Send,
) -> std::io::Result<()> {
    serve_with(listener, handler, codec, Timeouts::default(), shutdown).await
}

/// Accepts connections on `listener` until `shutdown` completes, answering
/// every frame through `handler` under `timeouts`.
///
/// On shutdown the listener stops accepting, each connection finishes the
/// frame it is answering and closes, and the call returns once every
/// connection has closed. Each connection runs in its own `mllp_connection`
/// span naming the peer.
///
/// # Errors
///
/// Returns the I/O error of a failed `accept`.
pub async fn serve_with<H: Handler>(
    listener: TcpListener,
    handler: Arc<H>,
    codec: Codec,
    timeouts: Timeouts,
    shutdown: impl Future<Output = ()> + Send,
) -> std::io::Result<()> {
    let (stop, stopped) = watch::channel(false);
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                tracing::debug!(peer = %peer, "MLLP connection accepted");
                let handler = Arc::clone(&handler);
                let stopped = stopped.clone();
                let span = tracing::info_span!(
                    "mllp_connection",
                    peer = %peer,
                    messages = tracing::field::Empty,
                    refused = tracing::field::Empty,
                );
                connections.spawn(
                    async move {
                        let ended =
                            connection(stream, handler.as_ref(), codec, timeouts, stopped).await;
                        if let Err(error) = ended {
                            tracing::warn!(error = %error, "MLLP connection closed on an error");
                        }
                    }
                    .instrument(span),
                );
            }
            Some(finished) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = finished {
                    tracing::warn!(error = %error, "an MLLP connection task failed");
                }
            }
        }
    }
    let _sent = stop.send(true);
    while let Some(finished) = connections.join_next().await {
        if let Err(error) = finished {
            tracing::warn!(error = %error, "an MLLP connection task failed");
        }
    }
    Ok(())
}

/// Answers the frames of one connection until the peer closes it, a frame is
/// malformed or stalls, the connection idles past its timeout, or the
/// listener stops.
async fn connection<H: Handler>(
    mut stream: TcpStream,
    handler: &H,
    mut codec: Codec,
    timeouts: Timeouts,
    mut stopped: watch::Receiver<bool>,
) -> Result<(), FrameError> {
    let mut buffer = BytesMut::new();
    let counts = Connection::default();
    let mut messages = 0usize;
    let mut idle_since = Instant::now();
    let mut frame_since: Option<Instant> = None;
    loop {
        let decoded = match codec.decode(&mut buffer) {
            Ok(Some(message)) => Some(message),
            Ok(None) => None,
            Err(FrameError::Malformed { kind, partial }) => {
                return refuse(&mut stream, handler, &mut codec, kind, &partial).await;
            }
            Err(other) => return Err(other),
        };
        if let Some(message) = decoded {
            let answered = handler.handle(message, &counts).await;
            messages = messages.saturating_add(1);
            let span = tracing::Span::current();
            span.record("messages", messages);
            span.record("refused", counts.refused());
            let Some(reply) = answered else {
                return Ok(());
            };
            let mut out = BytesMut::new();
            codec.encode(reply.as_slice(), &mut out)?;
            stream.write_all(&out).await?;
            if *stopped.borrow() {
                return Ok(());
            }
            idle_since = Instant::now();
            frame_since = (!buffer.is_empty()).then_some(idle_since);
            continue;
        }
        let deadline = match frame_since {
            None => timeouts
                .idle
                .and_then(|idle| Some((idle_since.checked_add(idle)?, None))),
            Some(since) => timeouts
                .frame
                .and_then(|frame| Some((since.checked_add(frame)?, Some(frame)))),
        };
        buffer.reserve(READ_CHUNK);
        tokio::select! {
            () = expiry(deadline.map(|(at, _)| at)) => {
                if let Some(limit) = deadline.and_then(|(_, frame)| frame) {
                    let partial = buffer.get(1..).unwrap_or_default().to_vec();
                    let kind = Malformed::Stalled { limit };
                    return refuse(&mut stream, handler, &mut codec, kind, &partial).await;
                }
                tracing::debug!("MLLP connection closed after its idle timeout");
                stream.shutdown().await?;
                return Ok(());
            }
            read = stream.read_buf(&mut buffer) => {
                if frame_since.is_none() && !buffer.is_empty() {
                    frame_since = Some(Instant::now());
                }
                if read? == 0 {
                    return match codec.decode_eof(&mut buffer) {
                        Ok(_) => Ok(()),
                        Err(FrameError::Malformed { kind, partial }) => {
                            refuse(&mut stream, handler, &mut codec, kind, &partial).await
                        }
                        Err(other) => Err(other),
                    };
                }
            }
            changed = stopped.changed() => {
                if changed.is_err() || *stopped.borrow() {
                    return Ok(());
                }
            }
        }
    }
}

/// Completes at `deadline`, or never when there is none.
async fn expiry(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Sends the handler's answer to a malformed frame, when it has one, and
/// ends the connection.
async fn refuse<H: Handler>(
    stream: &mut TcpStream,
    handler: &H,
    codec: &mut Codec,
    kind: Malformed,
    partial: &[u8],
) -> Result<(), FrameError> {
    tracing::warn!(refusal = %kind, "MLLP frame refused");
    if let Some(reply) = handler.malformed(kind, partial) {
        let mut out = BytesMut::new();
        codec.encode(reply.as_slice(), &mut out)?;
        stream.write_all(&out).await?;
    }
    stream.shutdown().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Codec, FrameError, Malformed};
    use bytes::BytesMut;
    use tokio_util::codec::{Decoder, Encoder};

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn a_frame_round_trips_through_the_codec() -> Result<(), FrameError> {
        let mut codec = Codec::default();
        let mut buffer = BytesMut::new();
        codec.encode(b"MSH|^~\\&|A".as_slice(), &mut buffer)?;
        assert_eq!(
            buffer.as_ref(),
            b"\x0bMSH|^~\\&|A\x1c\x0d",
            "the MLLP envelope"
        );
        assert_eq!(codec.decode(&mut buffer)?, Some(b"MSH|^~\\&|A".to_vec()));
        assert!(buffer.is_empty(), "the frame is consumed whole");
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn an_end_block_without_its_carriage_return_waits_for_the_next_byte() -> Result<(), FrameError>
    {
        let mut codec = Codec::default();
        let mut buffer = BytesMut::from(&b"\x0bMSH\x1c"[..]);
        assert_eq!(codec.decode(&mut buffer)?, None);
        buffer.extend_from_slice(b"\x0d");
        assert_eq!(codec.decode(&mut buffer)?, Some(b"MSH".to_vec()));
        Ok(())
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn two_frames_in_one_buffer_decode_one_after_the_other() -> Result<(), FrameError> {
        let mut codec = Codec::default();
        let mut buffer = BytesMut::from(&b"\x0bA\x1c\x0d\x0bB\x1c\x0d"[..]);
        assert_eq!(codec.decode(&mut buffer)?, Some(b"A".to_vec()));
        assert_eq!(codec.decode(&mut buffer)?, Some(b"B".to_vec()));
        assert_eq!(codec.decode(&mut buffer)?, None);
        Ok(())
    }

    #[test]
    fn a_start_block_inside_a_frame_is_refused() {
        let mut codec = Codec::default();
        let mut buffer = BytesMut::from(&b"\x0bA\x0bB\x1c\x0d"[..]);
        assert!(matches!(
            codec.decode(&mut buffer),
            Err(FrameError::Malformed {
                kind: Malformed::StartBlockInFrame,
                ..
            })
        ));
    }

    #[test]
    fn a_block_character_inside_a_message_cannot_be_framed() {
        let mut codec = Codec::default();
        let mut buffer = BytesMut::new();
        assert!(matches!(
            codec.encode(b"MSH\x1c".as_slice(), &mut buffer),
            Err(FrameError::Unframable)
        ));
    }

    #[test]
    fn a_frame_past_the_ceiling_is_refused_before_its_end_block() {
        let mut codec = Codec::new(4);
        let mut buffer = BytesMut::from(&b"\x0bMSH|^~"[..]);
        assert!(matches!(
            codec.decode(&mut buffer),
            Err(FrameError::Malformed {
                kind: Malformed::TooLarge { limit: 4 },
                ..
            })
        ));
    }
}
