// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The legacy half of the v2 root set: the message structures the v2.9.1
//! definitions no longer carry, from the tables of the versions that did.
//!
//! [`source::Tables`] reads the NIST IGAMT export of HL7's v2 database that
//! `scripts/vendor/v2-legacy.sh` fetches (one directory of JSON tables per
//! version, 2.1 to 2.8.2), and [`source::Table0354`] the status of each
//! structure code in HL7 table 0354 of the vendored `hl7.terminology`
//! package. [`lower::LegacyModel`] selects every structure whose code the
//! v2.9.1 definitions lack, lowers its tree and the segments it names per
//! version, and links each segment that agrees with its v2.9.1 definition to
//! that definition. Each defect of the export is tolerated only where it was
//! found ([`lower::LegacyDefect::tolerated_in`]).

pub mod lower;
pub mod source;
