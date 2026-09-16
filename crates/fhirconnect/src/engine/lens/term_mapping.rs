// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `TERM_MAPPING` against FHIR `Coding`, the shared half of the text cells.
//!
//! `DV_CODED_TEXT` and `DV_TEXT` both carry their extra codings in
//! `mappings`, and both tables point at the same row set
//! (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/TERM_MAPPING.adoc`):
//! `match` is "hard coded to `=`, other operators not mappable to FHIR", and
//! `target` is a `CODE_PHRASE` read through the `Coding` cell.

use fhir_types::r4::coding::Coding;
use openehr_rm::v1_2::data_types::text::term_mapping::TermMapping;

use crate::engine::lens::Lens;
use crate::engine::lens::LensError;
use crate::engine::lens::code_phrase::CodePhraseLens;
use crate::model::ast::Direction;

/// The `match` operator a mapping FHIR can carry writes.
pub const EQUIVALENT: char = '=';

/// Reads one `TERM_MAPPING` into the `Coding` it stands for.
///
/// A `match` other than `=` states a relation FHIR's `coding` list cannot
/// express, so carrying it would assert an equivalence the data denies.
///
/// # Errors
///
/// Returns [`LensError::Shape`] for a `match` other than `=`, and whatever
/// the `Coding` cell returns for the target.
pub fn coding_of(mapping: &TermMapping, cell: &'static str) -> Result<Coding, LensError> {
    if mapping.r#match != EQUIVALENT {
        return Err(LensError::Shape {
            cell,
            attribute: "TERM_MAPPING.match",
            found: mapping.r#match.to_string(),
            reason: "FHIR carries no relation beside a coding, so only `=` maps",
            direction: Direction::OpenehrToFhir,
        });
    }
    CodePhraseLens.get(&mapping.target)
}

/// Writes one `Coding` back into the `TERM_MAPPING` it came from.
///
/// `purpose` has no FHIR counterpart, so it is taken from the mapping the
/// target already holds at this position.
///
/// # Errors
///
/// Returns whatever the `Coding` cell returns for the target.
pub fn mapping_of(
    coding: &Coding,
    existing: Option<&TermMapping>,
) -> Result<TermMapping, LensError> {
    Ok(TermMapping {
        r#match: EQUIVALENT,
        purpose: existing.and_then(|mapping| mapping.purpose.clone()),
        target: CodePhraseLens.put(coding, existing.map(|mapping| &mapping.target))?,
    })
}

#[cfg(test)]
mod tests {
    use super::coding_of;
    use super::mapping_of;
    use crate::engine::lens::LensError;
    use crate::engine::lens::code_phrase::terminology_id;
    use fhir_types::r4::coding::Coding;
    use fhir_types::r4::primitives::Code;
    use fhir_types::r4::primitives::Uri;
    use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
    use openehr_rm::v1_2::data_types::text::term_mapping::TermMapping;

    /// A mapping carrying `match`, for the two cases below.
    fn mapping(relation: char) -> TermMapping {
        TermMapping {
            r#match: relation,
            purpose: None,
            target: CodePhrase {
                terminology_id: terminology_id("http://snomed.info/sct", ""),
                code_string: String::from("386661006"),
                preferred_term: None,
            },
        }
    }

    #[test]
    fn an_equivalent_mapping_is_one_coding() {
        let view = coding_of(&mapping('='), "test").expect("an equivalent mapping");
        assert_eq!(
            view.code.as_ref().and_then(|code| code.value.as_deref()),
            Some("386661006"),
            "the target code travels into the coding"
        );
    }

    #[test]
    fn a_narrower_mapping_is_refused() {
        let error = coding_of(&mapping('<'), "test").expect_err("a narrower relation");
        assert!(
            matches!(error, LensError::Shape { attribute, .. } if attribute == "TERM_MAPPING.match"),
            "the refusal names the operator: {error}"
        );
    }

    #[test]
    fn a_written_mapping_is_always_equivalent_and_keeps_its_purpose() {
        let existing = TermMapping {
            purpose: Some(
                openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText {
                    value: String::from("billing"),
                    hyperlink: None,
                    formatting: None,
                    mappings: None,
                    language: None,
                    encoding: None,
                    defining_code: CodePhrase {
                        terminology_id: terminology_id("http://example.org/purpose", ""),
                        code_string: String::from("bill"),
                        preferred_term: None,
                    },
                },
            ),
            ..mapping('=')
        };
        let view = Coding {
            system: Some(Uri::from("http://loinc.org")),
            code: Some(Code::from("LP74849-8")),
            ..Coding::default()
        };
        let written = mapping_of(&view, Some(&existing)).expect("the coding is complete");
        assert_eq!(written.r#match, '=', "the match is hard coded to `=`");
        assert_eq!(
            written.purpose, existing.purpose,
            "purpose has no FHIR counterpart and is carried"
        );
    }
}
