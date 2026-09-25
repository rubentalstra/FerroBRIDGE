// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FerroBRIDGE HL7 v2 face: MLLP framing, MSH-18 decoding, positional
//! parsing, the acknowledgment, and the v2-to-FHIR `ConceptMap` interpreter.
//!
//! A message arrives as one MLLP frame ([`mllp`]), is decoded from the
//! character set its MSH-18 names ([`decode`]), split by position and grouped
//! by the message structure the generated `hl7v2-types` tables define
//! ([`parse`]), and answered with an original-mode acknowledgment ([`ack`]);
//! [`inbound`] runs those steps and settles the `AR` and `AE` answers a
//! message owes before it is mapped.
//! [`map`] runs the `ConceptMaps` of the HL7 v2-to-FHIR implementation guide
//! over the parsed message and writes an R4 message `Bundle` through the
//! `fhirconnect` path model.
//!
//! Every segment, field or component the run does not carry, and every
//! condition or target form it cannot evaluate, is a typed outcome on the
//! result, counted and never dropped.
#![doc(test(attr(deny(warnings))))]

pub mod ack;
pub mod decode;
pub mod inbound;
pub mod map;
pub mod mllp;
pub mod parse;

/// The HL7 v2 version the generated definitions are derived from.
///
/// The `hl7v2-types` tables are generated from `HL7/v2ig`, whose definitions
/// are extracted from HL7 V2.9.1; a message of an earlier 2.x version is
/// parsed against them through the standard's backward compatibility.
pub const DEFINITIONS_VERSION: &str = "2.9.1";
