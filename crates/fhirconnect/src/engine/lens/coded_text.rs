// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! `DV_CODED_TEXT` against FHIR `CodeableConcept` and `Coding`.
//!
//! The cell is
//! `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/DV_CODED_TEXT.adoc`.
//! Its `CodeableConcept` table puts one `coding` into `defining_code` and
//! every further one into `mappings`, pairs `coding.display` with
//! `defining_code/preferred_term` and `text` with `value`, and gives `value`
//! two fallbacks marked "not for openEHR->FHIR".
//!
//! Which coding becomes the defining one is the page's own rule: the engine
//! takes the coding whose system matches the terminology the openEHR element
//! is bound to, "if yes, this one should be transformed into the `defining_code`
//! instead of the first one", and otherwise the first.

use fhir_types::r4::codeable_concept::CodeableConcept;
use fhir_types::r4::coding::Coding;
use fhir_types::r4::primitives::String as FhirString;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::engine::lens::Fallback;
use crate::engine::lens::Lens;
use crate::engine::lens::LensError;
use crate::engine::lens::code_phrase::CodePhraseLens;
use crate::engine::lens::code_phrase::text_of;
use crate::engine::lens::term_mapping::coding_of;
use crate::engine::lens::term_mapping::mapping_of;
use crate::model::ast::Direction;

/// The name the `CodeableConcept` cell carries in a diagnostic.
pub const CODEABLE_CONCEPT_CELL: &str = "DV_CODED_TEXT against CodeableConcept";

/// The name the `Coding` cell carries in a diagnostic.
pub const CODING_CELL: &str = "DV_CODED_TEXT against Coding";

/// `DV_CODED_TEXT` against `CodeableConcept` and against `Coding`.
///
/// The lens carries the terminology the openEHR element is bound to, when the
/// Web Template node declares one, because that is what decides which coding
/// of a `CodeableConcept` becomes the `defining_code`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodedTextLens {
    binding: Option<String>,
}

impl CodedTextLens {
    /// Creates a lens for an element the template binds to no terminology.
    #[must_use]
    pub const fn unbound() -> Self {
        Self { binding: None }
    }

    /// Creates a lens for an element bound to the terminology `system`.
    #[must_use]
    pub fn bound(system: impl Into<String>) -> Self {
        Self {
            binding: Some(system.into()),
        }
    }

    /// Returns the terminology the element is bound to.
    #[must_use]
    pub fn binding(&self) -> Option<&str> {
        self.binding.as_deref()
    }

    /// Returns the position of the coding that becomes the `defining_code`.
    ///
    /// The bound system wins when the list carries it, and the first coding
    /// otherwise.
    fn defining(&self, codings: &[Coding]) -> Option<usize> {
        let bound = self.binding.as_deref().and_then(|system| {
            codings.iter().position(|coding| {
                text_of(coding.system.as_ref().map(|uri| &uri.value)) == Some(system)
            })
        });
        bound.or_else(|| codings.first().map(|_first| 0))
    }
}

impl Lens<DvCodedText, CodeableConcept> for CodedTextLens {
    fn cell(&self) -> &'static str {
        CODEABLE_CONCEPT_CELL
    }

    fn get(&self, source: &DvCodedText) -> Result<CodeableConcept, LensError> {
        let mut defining = CodePhraseLens.get(&source.defining_code)?;
        defining.display = source
            .defining_code
            .preferred_term
            .clone()
            .map(FhirString::from);
        let mut codings = vec![defining];
        for mapping in source.mappings.iter().flatten() {
            codings.push(coding_of(mapping, CODEABLE_CONCEPT_CELL)?);
        }
        Ok(CodeableConcept {
            coding: codings,
            text: Some(FhirString::from(source.value.clone())),
            ..CodeableConcept::default()
        })
    }

    fn put(
        &self,
        view: &CodeableConcept,
        existing: Option<&DvCodedText>,
    ) -> Result<DvCodedText, LensError> {
        let absent = || LensError::Missing {
            cell: CODEABLE_CONCEPT_CELL,
            attribute: "DV_CODED_TEXT.defining_code",
            direction: Direction::FhirToOpenehr,
        };
        let position = self.defining(&view.coding).ok_or_else(absent)?;
        let defining = view.coding.get(position).ok_or_else(absent)?;
        let held = existing.and_then(|value| value.mappings.as_deref());
        let mut mappings = Vec::new();
        for (index, coding) in view
            .coding
            .iter()
            .enumerate()
            .filter(|&(index, _coding)| index != position)
            .map(|(_index, coding)| coding)
            .enumerate()
        {
            mappings.push(mapping_of(coding, held.and_then(|held| held.get(index)))?);
        }
        let value = value_of(
            text_of(view.text.as_ref().map(|text| &text.value)),
            defining,
            CODEABLE_CONCEPT_CELL,
        )?;
        Ok(DvCodedText {
            value: String::from(value),
            hyperlink: existing.and_then(|held| held.hyperlink.clone()),
            formatting: existing.and_then(|held| held.formatting.clone()),
            mappings: NonEmptyVec::new(mappings).ok(),
            language: existing.and_then(|held| held.language.clone()),
            encoding: existing.and_then(|held| held.encoding.clone()),
            defining_code: CodePhraseLens.put(defining, None)?,
        })
    }

    fn fallbacks(&self, view: &CodeableConcept) -> Vec<Fallback> {
        if text_of(view.text.as_ref().map(|text| &text.value)).is_some() {
            return Vec::new();
        }
        vec![Fallback::new(CODEABLE_CONCEPT_CELL, "DV_TEXT.value")]
    }
}

impl Lens<DvCodedText, Coding> for CodedTextLens {
    fn cell(&self) -> &'static str {
        CODING_CELL
    }

    fn get(&self, source: &DvCodedText) -> Result<Coding, LensError> {
        let mut coding = CodePhraseLens.get(&source.defining_code)?;
        // NOTE: DV_CODED_TEXT.adoc §Coding pairs `display` with `value`, so in
        // this cell the rubric travels and `preferred_term` has no counterpart.
        coding.display = Some(FhirString::from(source.value.clone()));
        Ok(coding)
    }

    fn put(&self, view: &Coding, existing: Option<&DvCodedText>) -> Result<DvCodedText, LensError> {
        let defining = CodePhraseLens.put(view, None)?;
        let value = match text_of(view.display.as_ref().map(|display| &display.value)) {
            Some(display) => String::from(display),
            None => defining.code_string.clone(),
        };
        Ok(DvCodedText {
            value,
            hyperlink: existing.and_then(|held| held.hyperlink.clone()),
            formatting: existing.and_then(|held| held.formatting.clone()),
            mappings: existing.and_then(|held| held.mappings.clone()),
            language: existing.and_then(|held| held.language.clone()),
            encoding: existing.and_then(|held| held.encoding.clone()),
            defining_code: CodePhrase {
                preferred_term: existing.and_then(|held| held.defining_code.preferred_term.clone()),
                ..defining
            },
        })
    }

    fn fallbacks(&self, view: &Coding) -> Vec<Fallback> {
        if text_of(view.display.as_ref().map(|display| &display.value)).is_some() {
            return Vec::new();
        }
        vec![Fallback::new(CODING_CELL, "DV_TEXT.value")]
    }
}

/// Returns the rubric a `DV_CODED_TEXT` carries, taking the page's fallbacks.
///
/// `value` is `1..1` in the reference model, so when FHIR carries no `text`
/// the table takes `coding.display` and then `coding.code`. Both rows are
/// marked "not for openEHR->FHIR", so each is recorded as a fallback.
fn value_of<'view>(
    text: Option<&'view str>,
    defining: &'view Coding,
    cell: &'static str,
) -> Result<&'view str, LensError> {
    text.or_else(|| text_of(defining.display.as_ref().map(|display| &display.value)))
        .or_else(|| text_of(defining.code.as_ref().map(|code| &code.value)))
        .ok_or(LensError::Missing {
            cell,
            attribute: "DV_TEXT.value",
            direction: Direction::FhirToOpenehr,
        })
}

#[cfg(test)]
mod tests {
    use super::CODEABLE_CONCEPT_CELL;
    use super::CodedTextLens;
    use crate::engine::lens::Fallback;
    use crate::engine::lens::Lens;
    use crate::engine::lens::code_phrase::terminology_id;
    use crate::engine::lens::code_phrase::tests::code_phrase;
    use crate::engine::lens::code_phrase::tests::coding;
    use fhir_types::r4::codeable_concept::CodeableConcept;
    use fhir_types::r4::coding::Coding;
    use fhir_types::r4::primitives::Code;
    use fhir_types::r4::primitives::String as FhirString;
    use fhir_types::r4::primitives::Uri;
    use openehr_base::containers::NonEmptyVec;
    use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
    use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use openehr_rm::v1_2::data_types::text::term_mapping::TermMapping;
    use proptest::collection::vec;
    use proptest::prop_compose;
    use proptest::proptest;

    prop_compose! {
        /// A `DV_CODED_TEXT` inside the subset the two cells carry losslessly.
        pub(crate) fn coded_text()(
            value in "[A-Za-z ]{1,12}",
            defining in code_phrase(),
            targets in vec(code_phrase(), 0..3),
        ) -> DvCodedText {
            DvCodedText {
                value,
                hyperlink: None,
                formatting: None,
                mappings: NonEmptyVec::new(
                    targets
                        .into_iter()
                        .map(|target| TermMapping { r#match: '=', purpose: None, target })
                        .collect::<Vec<TermMapping>>(),
                )
                .ok(),
                language: None,
                encoding: None,
                defining_code: defining,
            }
        }
    }

    prop_compose! {
        /// A `Coding` that carries a display.
        ///
        /// The `Coding` cell fills the mandatory `value` from `display`, and
        /// falls back to `code` when there is none. The fallback is one-way,
        /// so the law is stated over the codings that carry a display.
        fn displayed_coding()(
            system in "[a-z]{1,8}://[a-z.]{1,12}",
            version in proptest::option::of("[0-9]{4}"),
            code in "[A-Za-z0-9-]{1,10}",
            display in "[A-Za-z ]{1,12}",
        ) -> Coding {
            Coding {
                system: Some(Uri::from(system)),
                version: version.map(FhirString::from),
                code: Some(Code::from(code)),
                display: Some(FhirString::from(display)),
                ..Coding::default()
            }
        }
    }

    prop_compose! {
        /// A `CodeableConcept` inside the subset the cell carries losslessly.
        pub(crate) fn codeable_concept()(
            codings in vec(coding(), 1..4),
            text in "[A-Za-z ]{1,12}",
        ) -> CodeableConcept {
            CodeableConcept {
                coding: codings,
                text: Some(FhirString::from(text)),
                ..CodeableConcept::default()
            }
        }
    }

    proptest! {
        #[test]
        fn codeable_concept_get_put(source in coded_text()) {
            let lens = CodedTextLens::unbound();
            let view: CodeableConcept = lens.get(&source).expect("the generated subset is carried");
            let back = lens.put(&view, Some(&source)).expect("the view is complete");
            assert_eq!(back, source, "GetPut failed for {source:?}");
        }

        #[test]
        fn codeable_concept_put_get(view in codeable_concept()) {
            let lens = CodedTextLens::unbound();
            let source: DvCodedText = lens.put(&view, None).expect("the generated subset is carried");
            let back: CodeableConcept = lens.get(&source).expect("the value is complete");
            assert_eq!(back, view, "PutGet failed for {view:?}");
        }

        #[test]
        fn coding_get_put(source in coded_text()) {
            let lens = CodedTextLens::unbound();
            let view: Coding = lens.get(&source).expect("the generated subset is carried");
            let back = lens.put(&view, Some(&source)).expect("the view is complete");
            assert_eq!(back, source, "GetPut failed for {source:?}");
        }

        #[test]
        fn coding_put_get(view in displayed_coding()) {
            let lens = CodedTextLens::unbound();
            let source: DvCodedText = lens.put(&view, None).expect("the generated subset is carried");
            let back: Coding = lens.get(&source).expect("the value is complete");
            assert_eq!(back, view, "PutGet failed for {view:?}");
        }
    }

    /// A coding of `system` carrying `code`.
    fn code_of(system: &str, code: &str) -> Coding {
        Coding {
            system: Some(Uri::from(system)),
            code: Some(Code::from(code)),
            ..Coding::default()
        }
    }

    #[test]
    fn the_bound_terminology_becomes_the_defining_code() {
        let view = CodeableConcept {
            coding: vec![
                code_of("http://loinc.org", "LP74849-8"),
                code_of("http://snomed.info/sct", "386661006"),
            ],
            text: Some(FhirString::from("Fever")),
            ..CodeableConcept::default()
        };
        let lens = CodedTextLens::bound("http://snomed.info/sct");
        let source: DvCodedText = lens.put(&view, None).expect("two complete codings");
        assert_eq!(
            source.defining_code.code_string, "386661006",
            "the coding of the bound terminology is the defining code"
        );
        assert_eq!(
            source
                .mappings
                .as_deref()
                .and_then(<[TermMapping]>::first)
                .map(|mapping| mapping.target.code_string.clone()),
            Some(String::from("LP74849-8")),
            "every further coding becomes a term mapping"
        );
    }

    #[test]
    fn the_first_coding_wins_when_no_binding_matches() {
        let view = CodeableConcept {
            coding: vec![
                code_of("http://loinc.org", "LP74849-8"),
                code_of("http://snomed.info/sct", "386661006"),
            ],
            text: Some(FhirString::from("Fever")),
            ..CodeableConcept::default()
        };
        let lens = CodedTextLens::bound("http://example.org/other");
        let source: DvCodedText = lens.put(&view, None).expect("two complete codings");
        assert_eq!(
            source.defining_code.code_string, "LP74849-8",
            "the first coding is the defining code when the binding is absent"
        );
    }

    #[test]
    fn an_absent_text_falls_back_to_the_display_and_is_recorded() {
        let view = CodeableConcept {
            coding: vec![Coding {
                display: Some(FhirString::from("Fever")),
                ..code_of("http://loinc.org", "LP74849-8")
            }],
            ..CodeableConcept::default()
        };
        let lens = CodedTextLens::unbound();
        let source: DvCodedText = lens.put(&view, None).expect("one complete coding");
        assert_eq!(
            source.value, "Fever",
            "the display fills the mandatory value"
        );
        assert_eq!(
            Lens::<DvCodedText, CodeableConcept>::fallbacks(&lens, &view),
            vec![Fallback::new(CODEABLE_CONCEPT_CELL, "DV_TEXT.value")],
            "the one-way row is recorded"
        );
    }

    #[test]
    fn an_absent_text_and_display_falls_back_to_the_code() {
        let view = CodeableConcept {
            coding: vec![code_of("http://loinc.org", "LP74849-8")],
            ..CodeableConcept::default()
        };
        let source: DvCodedText = CodedTextLens::unbound()
            .put(&view, None)
            .expect("one complete coding");
        assert_eq!(
            source.value, "LP74849-8",
            "the code fills the mandatory value when the display is absent"
        );
    }

    #[test]
    fn a_concept_with_no_coding_is_refused() {
        let view = CodeableConcept {
            text: Some(FhirString::from("Fever")),
            ..CodeableConcept::default()
        };
        let error = <CodedTextLens as Lens<DvCodedText, CodeableConcept>>::put(
            &CodedTextLens::unbound(),
            &view,
            None,
        )
        .expect_err("defining_code is mandatory");
        assert!(
            error.to_string().contains("defining_code"),
            "the refusal names the mandatory attribute: {error}"
        );
    }

    #[test]
    fn a_narrower_term_mapping_refuses_the_element() {
        let source = DvCodedText {
            value: String::from("Fever"),
            hyperlink: None,
            formatting: None,
            mappings: NonEmptyVec::new(vec![TermMapping {
                r#match: '<',
                purpose: None,
                target: CodePhrase {
                    terminology_id: terminology_id("http://snomed.info/sct", ""),
                    code_string: String::from("386661006"),
                    preferred_term: None,
                },
            }])
            .ok(),
            language: None,
            encoding: None,
            defining_code: CodePhrase {
                terminology_id: terminology_id("http://loinc.org", ""),
                code_string: String::from("LP74849-8"),
                preferred_term: None,
            },
        };
        let error = <CodedTextLens as Lens<DvCodedText, CodeableConcept>>::get(
            &CodedTextLens::unbound(),
            &source,
        )
        .expect_err("a narrower relation cannot travel");
        assert!(
            error.to_string().contains("TERM_MAPPING.match"),
            "the refusal names the operator: {error}"
        );
    }
}
