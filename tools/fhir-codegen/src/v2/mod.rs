// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The HL7 v2 root set: the fetched v2ig definitions in, `hl7v2-types` out.
//!
//! The pipeline is [`corpus::Corpus`] (read the manifest-less definitions
//! tree), [`crate::roots::V2RootSet`] (select every message structure and segment),
//! [`lower::Model`] (the group trees and the field table of every segment the
//! set holds), [`render`] (source text), and [`emit`] (write or check).
//! [`legacy`] adds the structures the v2.9.1 definitions no longer carry,
//! from the tables of the earlier versions that did.

pub mod corpus;
pub mod definition;
pub mod emit;
pub mod legacy;
pub mod lower;
pub mod render;
