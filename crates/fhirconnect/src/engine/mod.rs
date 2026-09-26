// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The interpreter: one traversal of a compiled program, both directions.
//!
//! [`crate::resolve`] compiles a context into an immutable program with every
//! path already resolved; this module runs it. There is one traversal, and
//! the direction enters it in three places only (our own design, over the
//! rules the specification gives):
//!
//! - conditions are evaluated on the input side, so `fhirCondition` runs when
//!   FHIR is the input and `openehrCondition` when openEHR is
//!   (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
//!   "conditions are always applied on the input data");
//! - a mapping whose `unidirectional` names the other direction is skipped,
//!   and the skip is a recorded outcome rather than a log line;
//! - the composition defaults apply going into openEHR only
//!   (`docs/specs/fhirconnect/modules/ROOT/pages/engine/defaults-for-fields.adoc`).
//!
//! Everything else is written once. The data-type conversions are the lenses
//! of [`lens`], each run both ways and tested against the well-behaved-lens
//! laws, so a cell cannot disagree with itself.
//!
//! The direction is [`crate::model::ast::keyword::Direction`], the type the
//! `unidirectional` key already parses into: it names the same two
//! directions, and a second enum beside it would only need converting.
//!
//! The engine makes no call of its own. What a run asks of its caller, the
//! `mappingCode` functions, the resource a reference points at and the id of
//! a resource the run creates, is the [`seam`] it is handed.

pub mod cell;

pub mod condition;

pub mod context;

pub mod family;

pub mod fhir;

pub mod lens;

pub mod origin;

pub mod outcome;

pub mod recurrence;

pub mod rm;

pub mod seam;

pub mod traverse;
