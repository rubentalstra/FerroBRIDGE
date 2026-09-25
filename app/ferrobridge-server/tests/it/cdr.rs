// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDR client of the bridge over the generated ITS-REST 1.1.0 client of
//! `openehr-its`: the wire it sends against a stub CDR, the outcome it reads
//! per status, the pinned specification version, and one run against a real
//! CDR.

mod composition;
mod contribution;
mod ehr;
mod end_to_end;
mod pins;
mod query;
mod support;
mod template;
mod transport;
