// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `mappingCode` registry a run is handed.

use core::fmt;

use crate::engine::fhir::FhirValue;
use crate::engine::rm::RmValue;

/// The registry a `mappingCode` resolves against.
///
/// The specification leaves the functions to the engine
/// (`types-of-mappings/concept-type/concept-mappings.adoc`), so the registry
/// is a seam rather than a fixed list, and it ships empty.
pub trait MappingFunctions: fmt::Debug {
    /// Runs the function `code` names over one openEHR value.
    ///
    /// # Errors
    ///
    /// Returns [`MappingCodeError`] when the registry does not hold `code` or
    /// the function refuses its input.
    fn to_fhir(&self, code: &str, source: Option<&RmValue>) -> Result<FhirValue, MappingCodeError>;

    /// Runs the function `code` names over one FHIR element.
    ///
    /// # Errors
    ///
    /// Returns [`MappingCodeError`] when the registry does not hold `code` or
    /// the function refuses its input.
    fn to_openehr(&self, code: &str, view: Option<&FhirValue>)
    -> Result<RmValue, MappingCodeError>;
}

/// The registry this milestone ships.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoMappingFunctions;

impl MappingFunctions for NoMappingFunctions {
    fn to_fhir(
        &self,
        code: &str,
        _source: Option<&RmValue>,
    ) -> Result<FhirValue, MappingCodeError> {
        Err(MappingCodeError::Unknown {
            code: String::from(code),
        })
    }

    fn to_openehr(
        &self,
        code: &str,
        _view: Option<&FhirValue>,
    ) -> Result<RmValue, MappingCodeError> {
        Err(MappingCodeError::Unknown {
            code: String::from(code),
        })
    }
}

/// Why a registered function did not produce a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MappingCodeError {
    /// The registry holds no function of that name.
    #[error("no registered function is named {code}")]
    Unknown {
        /// The name the mapping wrote.
        code: String,
    },
    /// The function refused its input.
    #[error("the function {code} refused its input: {reason}")]
    Refused {
        /// The name the mapping wrote.
        code: String,
        /// What the function refused.
        reason: String,
    },
}
