// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The FerroBRIDGE openEHR ITS-REST 1.1.0 client with typed outcomes per status.
//!
//! Version 0.0.0 reserves the crate name; the implementation lands with
//! FerroBRIDGE issue #76 and follows the repository's architecture document.
//!
//! openEHR is a registered trademark of the openEHR Foundation.
#![doc(test(attr(deny(warnings))))]

/// The openEHR ITS-REST release this crate speaks.
///
/// The release is published at <https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/>.
pub const ITS_REST_VERSION: &str = "1.1.0";

// TODO(#76): the implementation this crate name is reserved for.
