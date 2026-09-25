// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! MLLP framing and the acknowledgment on the wire, against a plain
//! `TcpStream` sender (MLLP Release 1 block format; HL7 v2.5.1 chapter 2
//! §2.9.2.2 for the codes).

use std::sync::Arc;
use std::time::Duration;

use ferrobridge_hl7v2::mllp::{Codec, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::fixtures;
use crate::support::Face;

/// A running listener with its address and the switch that stops it.
struct Listener {
    address: std::net::SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<std::io::Result<()>>,
}

impl Listener {
    async fn start(codec: Codec) -> Self {
        let socket = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port");
        let address = socket.local_addr().expect("an address");
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(serve(socket, Arc::new(Face), codec, async {
            let _signal = stopped.await;
        }));
        Self {
            address,
            stop: Some(stop),
            task,
        }
    }

    async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            stop.send(()).expect("the listener is running");
        }
        let result = tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .expect("the listener stops");
        result
            .expect("the task joins")
            .expect("the listener ends cleanly");
    }
}

/// Wraps a message in the MLLP envelope.
fn frame(message: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x0B];
    bytes.extend_from_slice(message);
    bytes.extend_from_slice(&[0x1C, 0x0D]);
    bytes
}

/// Reads one framed acknowledgment and returns its text.
async fn read_ack(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut byte))
            .await
            .expect("an answer in time")
            .expect("the read succeeds");
        assert_eq!(read, 1, "the connection closed before the frame ended");
        buffer.push(byte[0]);
        if buffer.ends_with(&[0x1C, 0x0D]) {
            break;
        }
    }
    assert_eq!(
        buffer.first(),
        Some(&0x0B),
        "the answer opens with the start block"
    );
    String::from_utf8(buffer[1..buffer.len() - 2].to_vec()).expect("an ASCII answer")
}

/// Asserts that the peer closes the connection.
async fn assert_closed(stream: &mut TcpStream) {
    let mut rest = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut rest))
        .await
        .expect("the listener closes in time")
        .expect("the read succeeds");
    assert_eq!(read, 0, "nothing follows the answer");
}

/// The MSA segment of an acknowledgment.
fn msa(ack: &str) -> &str {
    ack.split('\r')
        .find(|segment| segment.starts_with("MSA|"))
        .expect("an MSA segment")
}

#[tokio::test]
async fn a_committed_message_is_answered_aa_echoing_msh_10() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    stream
        .write_all(&frame(&fixtures::adt_a01()))
        .await
        .expect("the frame is sent");
    let ack = read_ack(&mut stream).await;
    assert!(
        ack.starts_with("MSH|^~\\&|EHR|SOUTHCLINIC|ADMIT|NORTHHOSP|20260925143001+0200||ACK^A01^ACK|ACK00001|P|2.5.1"),
        "{ack}"
    );
    assert_eq!(msa(&ack), "MSA|AA|MSG00002");
    drop(stream);
    listener.stop().await;
}

#[tokio::test]
async fn a_frame_split_across_two_writes_is_read_whole() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    let bytes = frame(&fixtures::adt_a01());
    let (first, second) = bytes.split_at(60);
    stream
        .write_all(first)
        .await
        .expect("the first half is sent");
    stream.flush().await.expect("flushed");
    tokio::time::sleep(Duration::from_millis(50)).await;
    stream
        .write_all(second)
        .await
        .expect("the second half is sent");
    assert_eq!(msa(&read_ack(&mut stream).await), "MSA|AA|MSG00002");
    drop(stream);
    listener.stop().await;
}

#[tokio::test]
async fn the_trailer_split_from_its_end_block_still_closes_the_frame() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    let bytes = frame(&fixtures::adt_a01());
    let (body, trailer) = bytes.split_at(bytes.len() - 1);
    stream
        .write_all(body)
        .await
        .expect("the body and end block are sent");
    stream.flush().await.expect("flushed");
    tokio::time::sleep(Duration::from_millis(50)).await;
    stream
        .write_all(trailer)
        .await
        .expect("the carriage return is sent");
    assert_eq!(msa(&read_ack(&mut stream).await), "MSA|AA|MSG00002");
    drop(stream);
    listener.stop().await;
}

#[tokio::test]
async fn two_frames_in_one_write_are_answered_in_order() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    let mut bytes = frame(&fixtures::adt_a01());
    bytes.extend(frame(&fixtures::mdm_t02()));
    stream
        .write_all(&bytes)
        .await
        .expect("both frames are sent");
    assert_eq!(msa(&read_ack(&mut stream).await), "MSA|AA|MSG00002");
    assert_eq!(msa(&read_ack(&mut stream).await), "MSA|AA|MSG00003");
    drop(stream);
    listener.stop().await;
}

#[tokio::test]
async fn a_missing_required_field_is_answered_ae_with_its_location() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    stream
        .write_all(&frame(&fixtures::oru_r01_without_patient_name()))
        .await
        .expect("the frame is sent");
    let ack = read_ack(&mut stream).await;
    assert_eq!(msa(&ack), "MSA|AE|MSG00004");
    assert!(
        ack.contains(
            "\rERR||PID^1^5|101^Required field missing^HL70357|E||||PID.5-patientName is required\r"
        ),
        "{ack}"
    );
    drop(stream);
    listener.stop().await;
}

#[tokio::test]
async fn an_undeclared_byte_sequence_is_answered_ar() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    let message = fixtures::message(&[
        b"MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ORU^R01^ORU_R01|MSG00005|P|2.5.1||||||ASCII",
        b"PID|1||PAT-0001^^^NORTHLAB^MR||M\xFCller^J\xF6rg",
    ]);
    stream
        .write_all(&frame(&message))
        .await
        .expect("the frame is sent");
    let ack = read_ack(&mut stream).await;
    assert_eq!(msa(&ack), "MSA|AR|MSG00005");
    assert!(ack.contains("|207^Application error^HL70357|E|"), "{ack}");
    drop(stream);
    listener.stop().await;
}

#[tokio::test]
async fn a_message_type_the_definitions_do_not_carry_is_answered_ar() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    let message = fixtures::message(&[
        b"MSH|^~\\&|LAB|NORTHLAB|EHR|SOUTHCLINIC|20260925143000+0200||ZZZ^Z01^ZZZ_Z01|MSG00006|P|2.5.1",
    ]);
    stream
        .write_all(&frame(&message))
        .await
        .expect("the frame is sent");
    let ack = read_ack(&mut stream).await;
    assert_eq!(msa(&ack), "MSA|AR|MSG00006");
    assert!(
        ack.contains("|200^Unsupported message type^HL70357|E|"),
        "{ack}"
    );
    drop(stream);
    listener.stop().await;
}

// NOTE: HL7 R4 MessageHeader.source is 1..1, written from MSH-3, MSH-24 or the sending
// facility MSH-4, so a message valuing none is refused naming MSH-3.
#[tokio::test]
async fn a_message_naming_no_sender_is_answered_ar_at_msh_3() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    stream
        .write_all(&frame(&fixtures::oru_r01_without_sender()))
        .await
        .expect("the frame is sent");
    let ack = read_ack(&mut stream).await;
    assert_eq!(msa(&ack), "MSA|AR|MSG00007");
    assert!(
        ack.contains("|MSH^1^3|101^Required field missing^HL70357|E|"),
        "{ack}"
    );
    drop(stream);
    listener.stop().await;
}

#[tokio::test]
async fn a_frame_without_its_start_block_is_answered_ar_and_closed() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    let mut bytes = fixtures::adt_a01();
    bytes.extend_from_slice(&[0x1C, 0x0D]);
    stream.write_all(&bytes).await.expect("the bytes are sent");
    let ack = read_ack(&mut stream).await;
    assert_eq!(msa(&ack), "MSA|AR|MSG00002");
    assert!(ack.contains("does not open with the start block"), "{ack}");
    assert_closed(&mut stream).await;
    listener.stop().await;
}

#[tokio::test]
async fn a_frame_past_the_ceiling_is_answered_ar_and_closed() {
    let listener = Listener::start(Codec::new(64)).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    stream
        .write_all(&frame(&fixtures::oru_r01()))
        .await
        .expect("the frame is sent");
    let ack = read_ack(&mut stream).await;
    assert!(msa(&ack).starts_with("MSA|AR|"), "{ack}");
    assert_closed(&mut stream).await;
    listener.stop().await;
}

#[tokio::test]
async fn an_unreadable_malformed_frame_is_closed_without_an_answer() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    stream
        .write_all(b"garbage\x1c\x0d")
        .await
        .expect("the bytes are sent");
    assert_closed(&mut stream).await;
    listener.stop().await;
}

#[tokio::test]
async fn the_listener_stops_on_its_signal_while_a_connection_is_open() {
    let listener = Listener::start(Codec::default()).await;
    let mut stream = TcpStream::connect(listener.address)
        .await
        .expect("a connection");
    stream
        .write_all(&frame(&fixtures::adt_a01()))
        .await
        .expect("the frame is sent");
    assert_eq!(msa(&read_ack(&mut stream).await), "MSA|AA|MSG00002");
    listener.stop().await;
    assert_closed(&mut stream).await;
}
