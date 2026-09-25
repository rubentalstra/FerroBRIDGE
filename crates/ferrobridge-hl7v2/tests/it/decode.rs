// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! MSH-18 decoding (HL7 table 0211, `CodeSystem/v2-0211`).

use ferrobridge_hl7v2::decode::{Charset, DecodeError, decode, encode};

use crate::fixtures;

#[test]
fn iso_8859_1_decodes_a_latin_1_name() {
    let decoded = decode(&fixtures::oru_r01(), Charset::Utf8).expect("the message decodes");
    assert_eq!(decoded.charset, Charset::Iso8859(1));
    assert!(decoded.declared, "MSH-18 named the set");
    assert!(
        decoded.text.contains("M\u{FC}ller^J\u{F6}rg"),
        "the name reads as Latin-1"
    );
    assert!(
        decoded.text.contains("Hauptstra\u{DF}e"),
        "the street reads as Latin-1"
    );
}

#[test]
fn utf_8_decodes_a_name_outside_ascii() {
    let decoded = decode(&fixtures::adt_a01(), Charset::Ascii).expect("the message decodes");
    assert_eq!(decoded.charset, Charset::Utf8);
    assert!(decoded.text.contains("Zo\u{EB}^Anna"));
}

#[test]
fn an_empty_msh_18_reads_the_connection_default() {
    let decoded = decode(&fixtures::mdm_t02(), Charset::Iso8859(1)).expect("the message decodes");
    assert_eq!(decoded.charset, Charset::Iso8859(1));
    assert!(!decoded.declared, "MSH-18 is empty");
}

#[test]
fn a_utf_8_message_with_an_invalid_sequence_is_refused() {
    let mut bytes = fixtures::adt_a01();
    bytes.extend_from_slice(b"NTE|1||\xC3\x28\r");
    assert!(matches!(
        decode(&bytes, Charset::Ascii),
        Err(DecodeError::Undeclared {
            charset: "UNICODE UTF-8",
            ..
        })
    ));
}

#[test]
fn a_latin_1_byte_in_an_ascii_message_is_refused_never_replaced() {
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|A|B|C|D|20260925||ADT^A01^ADT_A01|MSG1|P|2.5.1||||||ASCII",
        b"PID|1||X||M\xFCller",
    ]);
    let refused = decode(&bytes, Charset::Utf8);
    assert!(
        matches!(
            refused,
            Err(DecodeError::Undeclared {
                charset: "ASCII",
                ..
            })
        ),
        "{refused:?}"
    );
}

#[test]
fn a_c1_control_byte_is_outside_every_iso_8859_part() {
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|A|B|C|D|20260925||ADT^A01^ADT_A01|MSG1|P|2.5.1||||||8859/1",
        b"PID|1||X||\x85",
    ]);
    assert!(matches!(
        decode(&bytes, Charset::Utf8),
        Err(DecodeError::Undeclared {
            charset: "8859/1",
            ..
        })
    ));
}

#[test]
fn iso_8859_1_is_not_read_as_windows_1252() {
    // NOTE: encoding_rs decodes the label iso-8859-1 as windows-1252
    // (<https://docs.rs/encoding_rs/0.8.42/encoding_rs/>); 0x80 is a C1 control in ISO 8859-1.
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|A|B|C|D|20260925||ADT^A01^ADT_A01|MSG1|P|2.5.1||||||8859/1",
        b"PID|1||X||\x80",
    ]);
    assert!(
        decode(&bytes, Charset::Utf8).is_err(),
        "no euro sign is read"
    );
}

#[test]
fn a_character_set_this_crate_does_not_decode_is_refused() {
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|A|B|C|D|20260925||ADT^A01^ADT_A01|MSG1|P|2.5.1||||||UNICODE UTF-16",
    ]);
    assert_eq!(
        decode(&bytes, Charset::Utf8),
        Err(DecodeError::UnsupportedCharset {
            code: String::from("UNICODE UTF-16"),
        })
    );
}

#[test]
fn a_repeating_msh_18_is_refused() {
    let bytes = fixtures::message(&[
        b"MSH|^~\\&|A|B|C|D|20260925||ADT^A01^ADT_A01|MSG1|P|2.5.1||||||8859/1~ISO IR87",
    ]);
    assert_eq!(
        decode(&bytes, Charset::Utf8),
        Err(DecodeError::AlternateCharsets)
    );
}

#[test]
fn iso_8859_2_through_8859_15_decode_their_own_letters() {
    for (code, byte, letter) in [
        ("8859/2", 0xB1u8, '\u{105}'),
        ("8859/5", 0xB0, '\u{410}'),
        ("8859/7", 0xE1, '\u{3B1}'),
        ("8859/15", 0xA4, '\u{20AC}'),
    ] {
        let header =
            format!("MSH|^~\\&|A|B|C|D|20260925||ADT^A01^ADT_A01|MSG1|P|2.5.1||||||{code}");
        let bytes = fixtures::message(&[header.as_bytes(), &[b'P', b'I', b'D', b'|', byte]]);
        let decoded = decode(&bytes, Charset::Utf8).expect("the part decodes");
        assert!(decoded.text.ends_with(&format!("PID|{letter}\r")), "{code}");
        assert_eq!(
            encode(&letter.to_string(), decoded.charset),
            Ok(vec![byte]),
            "{code} encodes back"
        );
    }
}
