// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! `CODE_PHRASE` against FHIR `Coding`.
//!
//! The cell is
//! `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/CODE_PHRASE.adoc`.
//! Its table names a FHIR `value` element and an openEHR `value` attribute
//! that neither model has: R4 `Coding` spells the symbol `code`
//! (<https://hl7.org/fhir/R4/datatypes.html#Coding>) and the RM `CODE_PHRASE`
//! carries `terminology_id`, `code_string` and `preferred_term` and no
//! `value` (<https://specifications.openehr.org/releases/RM/Release-1.1.0/data_types.html#_code_phrase_class>).
//! Both misspellings are recorded as an upstream report (issue #100), and the
//! rows are read as the models spell them: `code` against `code_string`, and
//! `display` against `preferred_term`, which the `DV_CODED_TEXT` table pairs
//! with `coding.display` in so many words.

use fhir_types::r4::coding::Coding;
use fhir_types::r4::primitives::Code;
use fhir_types::r4::primitives::String as FhirString;
use fhir_types::r4::primitives::Uri;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;

use crate::engine::lens::Fallback;
use crate::engine::lens::Lens;
use crate::engine::lens::LensError;
use crate::model::ast::Direction;

/// The name this cell carries in a diagnostic.
pub const CELL: &str = "CODE_PHRASE against Coding";

/// `CODE_PHRASE` against `Coding`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CodePhraseLens;

impl Lens<CodePhrase, Coding> for CodePhraseLens {
    fn cell(&self) -> &'static str {
        CELL
    }

    fn get(&self, source: &CodePhrase) -> Result<Coding, LensError> {
        let (name, version) = terminology_parts(&source.terminology_id, CELL)?;
        if name.is_empty() {
            return Err(LensError::Missing {
                cell: CELL,
                attribute: "Coding.system",
                direction: Direction::OpenehrToFhir,
            });
        }
        if source.code_string.is_empty() {
            return Err(LensError::Missing {
                cell: CELL,
                attribute: "Coding.code",
                direction: Direction::OpenehrToFhir,
            });
        }
        Ok(Coding {
            system: Some(Uri::from(name)),
            // NOTE: CODE_PHRASE.adoc, `version` against `terminology_id/version_id`,
            // "populate with emtpy string if not provided": that is the
            // TERMINOLOGY_ID with no parenthesised suffix.
            version: (!version.is_empty()).then(|| FhirString::from(version)),
            code: Some(Code::from(source.code_string.clone())),
            display: source.preferred_term.clone().map(FhirString::from),
            ..Coding::default()
        })
    }

    fn put(&self, view: &Coding, _existing: Option<&CodePhrase>) -> Result<CodePhrase, LensError> {
        let system =
            text_of(view.system.as_ref().map(|uri| &uri.value)).ok_or(LensError::Missing {
                cell: CELL,
                attribute: "CODE_PHRASE.terminology_id",
                direction: Direction::FhirToOpenehr,
            })?;
        let code =
            text_of(view.code.as_ref().map(|code| &code.value)).ok_or(LensError::Missing {
                cell: CELL,
                attribute: "CODE_PHRASE.code_string",
                direction: Direction::FhirToOpenehr,
            })?;
        let version = text_of(view.version.as_ref().map(|version| &version.value)).unwrap_or("");
        Ok(CodePhrase {
            terminology_id: terminology_id(system, version),
            code_string: String::from(code),
            preferred_term: text_of(view.display.as_ref().map(|display| &display.value))
                .map(String::from),
        })
    }

    fn fallbacks(&self, _view: &Coding) -> Vec<Fallback> {
        Vec::new()
    }
}

/// Returns the value a FHIR primitive carries, `None` when it carries none.
///
/// A primitive with no value carries only an `id` or an `extension`
/// (<https://hl7.org/fhir/R4/json.html>), which is not a code.
pub(crate) fn text_of(value: Option<&Option<String>>) -> Option<&str> {
    value
        .and_then(Option::as_deref)
        .filter(|text| !text.is_empty())
}

/// Splits a `TERMINOLOGY_ID` into its name and its version.
///
/// openEHR BASE Release 1.2.0 §Identification gives `TERMINOLOGY_ID.value`
/// the form `name` optionally followed by `(version)`, and derives `name` and
/// `version_id` from it. A value that spells one parenthesis and not the
/// other is neither form, so it is refused rather than silently truncated.
///
/// # Errors
///
/// Returns [`LensError::Shape`] for a value the `name(version)` form does not
/// admit.
pub fn terminology_parts<'value>(
    id: &'value TerminologyId,
    cell: &'static str,
) -> Result<(&'value str, &'value str), LensError> {
    let malformed = || LensError::Shape {
        cell,
        attribute: "CODE_PHRASE.terminology_id",
        found: id.value.clone(),
        reason: "a TERMINOLOGY_ID is a name optionally followed by a parenthesised version",
        direction: Direction::OpenehrToFhir,
    };
    let opens = id.value.contains('(');
    let closes = id.value.ends_with(')');
    if opens != closes {
        return Err(malformed());
    }
    let name = id.name();
    let version = id.version_id();
    if opens && version.is_empty() {
        return Err(malformed());
    }
    Ok((name, version))
}

/// Joins a system and a version into a `TERMINOLOGY_ID`.
///
/// An empty version is the absent `(version)` suffix, which is what makes
/// `version_id` the empty string the `CODE_PHRASE` table asks for.
#[must_use]
pub fn terminology_id(system: &str, version: &str) -> TerminologyId {
    TerminologyId {
        value: if version.is_empty() {
            String::from(system)
        } else {
            format!("{system}({version})")
        },
    }
}

#[cfg(test)]
mod tests {
    use super::CELL;
    use super::CodePhraseLens;
    use super::terminology_id;
    use super::terminology_parts;
    use crate::engine::lens::Lens;
    use crate::engine::lens::LensError;
    use fhir_types::r4::coding::Coding;
    use fhir_types::r4::primitives::Code;
    use fhir_types::r4::primitives::String as FhirString;
    use fhir_types::r4::primitives::Uri;
    use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
    use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
    use proptest::prop_compose;
    use proptest::proptest;

    prop_compose! {
        /// A `CODE_PHRASE` inside the subset this cell carries losslessly.
        pub(crate) fn code_phrase()(
            system in "[a-z]{1,8}://[a-z.]{1,12}",
            version in proptest::option::of("[0-9]{4}"),
            code in "[A-Za-z0-9-]{1,10}",
            preferred in proptest::option::of("[A-Za-z ]{1,12}"),
        ) -> CodePhrase {
            CodePhrase {
                terminology_id: terminology_id(&system, version.as_deref().unwrap_or("")),
                code_string: code,
                preferred_term: preferred,
            }
        }
    }

    prop_compose! {
        /// A `Coding` inside the subset this cell carries losslessly.
        ///
        /// `userSelected`, `id` and `extension` are the attributes the table
        /// marks with no openEHR counterpart, so a generated value leaves
        /// them out and the PutGet law is stated over what remains.
        pub(crate) fn coding()(
            system in "[a-z]{1,8}://[a-z.]{1,12}",
            version in proptest::option::of("[0-9]{4}"),
            code in "[A-Za-z0-9-]{1,10}",
            display in proptest::option::of("[A-Za-z ]{1,12}"),
        ) -> Coding {
            Coding {
                system: Some(Uri::from(system)),
                version: version.map(FhirString::from),
                code: Some(Code::from(code)),
                display: display.map(FhirString::from),
                ..Coding::default()
            }
        }
    }

    proptest! {
        #[test]
        fn get_put(source in code_phrase()) {
            let lens = CodePhraseLens;
            let view = lens.get(&source).expect("the generated subset is carried");
            let back = lens.put(&view, Some(&source)).expect("the view is complete");
            assert_eq!(back, source, "GetPut failed for {source:?}");
        }

        #[test]
        fn put_get(view in coding()) {
            let lens = CodePhraseLens;
            let source = lens.put(&view, None).expect("the generated subset is carried");
            let back = lens.get(&source).expect("the value is complete");
            assert_eq!(back, view, "PutGet failed for {view:?}");
        }
    }

    #[test]
    fn a_version_travels_inside_the_terminology_id() {
        let lens = CodePhraseLens;
        let source = CodePhrase {
            terminology_id: TerminologyId {
                value: String::from("ICD10(2020)"),
            },
            code_string: String::from("A00"),
            preferred_term: None,
        };
        let view = lens.get(&source).expect("a well-formed terminology id");
        assert_eq!(
            view.system.as_ref().and_then(|uri| uri.value.as_deref()),
            Some("ICD10"),
            "the system is the terminology name"
        );
        assert_eq!(
            view.version.as_ref().and_then(|text| text.value.as_deref()),
            Some("2020"),
            "the version is the parenthesised part"
        );
        assert_eq!(
            lens.put(&view, None).expect("the view is complete"),
            source,
            "the terminology id is rebuilt from the two parts"
        );
    }

    #[test]
    fn a_malformed_terminology_id_is_refused() {
        let id = TerminologyId {
            value: String::from("ICD10(2020"),
        };
        let error = terminology_parts(&id, CELL).expect_err("an unbalanced parenthesis");
        assert!(
            matches!(error, LensError::Shape { attribute, .. } if attribute == "CODE_PHRASE.terminology_id"),
            "the refusal names the attribute: {error}"
        );
    }

    #[test]
    fn a_coding_without_a_code_is_refused() {
        let lens = CodePhraseLens;
        let view = Coding {
            system: Some(Uri::from("http://loinc.org")),
            ..Coding::default()
        };
        let error = lens.put(&view, None).expect_err("code_string is mandatory");
        assert!(
            matches!(error, LensError::Missing { attribute, .. } if attribute == "CODE_PHRASE.code_string"),
            "the refusal names the mandatory attribute: {error}"
        );
    }

    #[test]
    fn a_coding_without_a_system_is_refused() {
        let lens = CodePhraseLens;
        let view = Coding {
            code: Some(Code::from("A00")),
            ..Coding::default()
        };
        let error = lens
            .put(&view, None)
            .expect_err("terminology_id is mandatory");
        assert!(
            matches!(error, LensError::Missing { attribute, .. } if attribute == "CODE_PHRASE.terminology_id"),
            "the refusal names the mandatory attribute: {error}"
        );
    }
}
