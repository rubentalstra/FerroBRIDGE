// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FerroBRIDGE openEHR ITS-REST 1.1.0 client with typed outcomes per status.
//!
//! Every call answers with an outcome enum whose variants are the statuses the
//! governing ITS-REST 1.1.0 operation documents, each carrying what the
//! service sent. A status the operation does not document, a refused
//! connection, a timeout and an undecodable body are typed errors that carry
//! the upstream status and body; none of them becomes an empty value.
//!
//! The wire is canonical JSON, the mandatory composition representation of the
//! specification, and the model types come from the published `openehr-rm`,
//! `openehr-its` and `openehr-am` crates.
//!
//! openEHR is a registered trademark of the openEHR Foundation.
#![doc(test(attr(deny(warnings))))]

pub mod client;
pub mod commit;
pub mod composition;
pub mod config;
pub mod contribution;
mod decode;
pub mod ehr;
pub mod error;
pub mod ids;
pub mod prefer;
pub mod query;
pub mod template;

/// The openEHR ITS-REST release this crate speaks.
///
/// The release is published at <https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/>.
pub const ITS_REST_VERSION: &str = "1.1.0";
