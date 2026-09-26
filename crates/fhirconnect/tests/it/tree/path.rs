// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR-side expression parser and its anchoring.

use core::num::NonZeroUsize;

use fhirconnect::tree::error::AnchorError;
use fhirconnect::tree::error::ParseError;
use fhirconnect::tree::path::FhirPath;
use fhirconnect::tree::path::Head;
use fhirconnect::tree::path::Ordinal;
use fhirconnect::tree::path::Step;
use fhirconnect::tree::path::TypeForm;
use fhirconnect::tree::path::Writability;

fn parse(text: &str) -> Result<FhirPath, ParseError> {
    text.parse()
}

#[test]
fn the_two_fhir_side_variables_are_the_heads_the_specification_defines() -> Result<(), ParseError> {
    assert_eq!(parse("$resource")?.head(), Head::Resource);
    assert_eq!(parse("$fhirRoot")?.head(), Head::FhirRoot);
    assert_eq!(parse("code")?.head(), Head::Anchor);
    assert_eq!(
        parse("^^.use.coding")?.head(),
        Head::Parent(NonZeroUsize::new(2).unwrap())
    );
    Ok(())
}

#[test]
fn an_openehr_side_variable_is_no_fhir_head() {
    let refused = parse("$archetype/data[at0001]");
    assert!(matches!(
        refused,
        Err(ParseError::UnknownVariable { ref name, at: 0 }) if name == "archetype"
    ));
}

#[test]
fn the_three_fhirpath_forms_the_mapping_library_uses_parse() -> Result<(), ParseError> {
    assert_eq!(
        parse("onset.ofType(Period)")?.steps().last(),
        Some(&Step::Type {
            name: String::from("Period"),
            form: TypeForm::OfType,
        })
    );
    assert_eq!(
        parse("onset.as(DateTime)")?.steps().last(),
        Some(&Step::Type {
            name: String::from("DateTime"),
            form: TypeForm::As,
        })
    );
    assert_eq!(
        parse("$resource.extension('http://example.invalid/x')")?
            .steps()
            .last(),
        Some(&Step::Extension {
            url: String::from("http://example.invalid/x"),
        })
    );
    assert_eq!(
        parse("reasonReference.resolve().as(Condition).code")?
            .steps()
            .first(),
        Some(&Step::Element {
            name: String::from("reasonReference")
        })
    );
    Ok(())
}

#[test]
fn a_namespace_qualifier_on_a_type_specifier_keeps_the_type_name() -> Result<(), ParseError> {
    assert_eq!(
        parse("value.ofType(FHIR.Quantity)")?.steps().last(),
        Some(&Step::Type {
            name: String::from("Quantity"),
            form: TypeForm::OfType,
        })
    );
    Ok(())
}

#[test]
fn a_filter_makes_the_whole_expression_read_only() -> Result<(), ParseError> {
    for (expression, step) in [
        (
            "$resource.category.where(coding.code = 'x').text",
            "where(coding.code = 'x')",
        ),
        ("$resource.category.first().text", "first()"),
        ("$resource.category.last().text", "last()"),
        ("$resource.category[1].text", "[1]"),
        ("$resource.recorder.resolve().name", "resolve()"),
    ] {
        let path = parse(expression)?;
        let Writability::ReadOnly { step: named, .. } = path.writability() else {
            panic!("`{expression}` should have been classified read-only")
        };
        assert_eq!(named, step, "`{expression}` names the wrong offending step");
    }
    Ok(())
}

#[test]
fn an_expression_of_element_names_and_type_filters_is_writable() -> Result<(), ParseError> {
    assert_eq!(
        parse("$resource.onset.ofType(Period).start")?.writability(),
        &Writability::Writable
    );
    Ok(())
}

#[test]
fn an_index_filter_parses_as_its_own_step() -> Result<(), ParseError> {
    let path = parse("category[2].coding")?;
    assert_eq!(
        path.steps(),
        [
            Step::Element {
                name: String::from("category")
            },
            Step::Index(2),
            Step::Element {
                name: String::from("coding")
            },
        ]
    );
    assert_eq!(
        path.steps().get(1).map(ToString::to_string).as_deref(),
        Some("[2]")
    );
    Ok(())
}

#[test]
fn a_predicate_keeps_its_nested_parentheses_and_quotes() -> Result<(), ParseError> {
    let path = parse("extension.where(url = 'a(b)')")?;
    assert_eq!(
        path.steps().last(),
        Some(&Step::Predicate {
            expression: String::from("url = 'a(b)'")
        })
    );
    Ok(())
}

#[test]
fn every_refusal_names_its_token_and_offset() {
    assert!(matches!(parse(""), Err(ParseError::Empty)));
    assert!(matches!(
        parse("$resource.trim()"),
        Err(ParseError::UnknownFunction { ref name, at: 10 }) if name == "trim"
    ));
    assert!(matches!(
        parse("$resource..code"),
        Err(ParseError::Expected { at: 10, .. })
    ));
    assert!(matches!(
        parse("extension('unclosed"),
        Err(ParseError::UnterminatedString { at: 10 })
    ));
    assert!(matches!(
        parse("extension('a\\qb')"),
        Err(ParseError::InvalidEscape {
            escape: 'q',
            at: 12
        })
    ));
    assert!(matches!(
        parse("category.where(url = 'a'"),
        Err(ParseError::UnterminatedCall { ref name, at: 9 }) if name == "where"
    ));
}

#[test]
fn an_ordinal_renders_as_written() {
    assert_eq!(Ordinal::First.to_string(), "first()");
    assert_eq!(Ordinal::Last.to_string(), "last()");
}

#[test]
fn an_anchor_binds_the_relative_heads_to_a_resource_path() -> Result<(), Box<dyn core::error::Error>>
{
    let anchor = parse("$resource.diagnosis.condition")?;
    assert_eq!(
        parse("$fhirRoot.reference")?.anchored(&anchor)?.as_str(),
        "$resource.diagnosis.condition.reference"
    );
    assert_eq!(
        parse("reference")?.anchored(&anchor)?.as_str(),
        "$resource.diagnosis.condition.reference"
    );
    assert_eq!(
        parse("^^.use.coding")?.anchored(&anchor)?.as_str(),
        "$resource.use.coding"
    );
    assert_eq!(
        parse("$resource.code")?.anchored(&anchor)?.as_str(),
        "$resource.code"
    );
    Ok(())
}

#[test]
fn a_parent_run_above_the_resource_is_refused() -> Result<(), ParseError> {
    let anchor = parse("$resource.code")?;
    let refused = parse("^^.use")?.anchored(&anchor);
    assert!(matches!(
        refused,
        Err(AnchorError::AboveResource { hops: 2, .. })
    ));
    let relative = parse("code")?;
    assert!(matches!(
        parse("^.use")?.anchored(&relative),
        Err(AnchorError::RelativeAnchor { .. })
    ));
    Ok(())
}

#[test]
fn a_caret_run_carries_on_in_the_next_anchor() -> Result<(), Box<dyn core::error::Error>> {
    let inside = parse("$resource")?;
    let outside = parse("$resource.diagnosis.condition.reference")?;
    let (bound, index) = parse("^^.use.coding")?.anchored_in(&[&inside, &outside])?;
    assert_eq!(bound.as_str(), "$resource.diagnosis.use.coding");
    assert_eq!(index, 1, "the expression binds in the enclosing resource");
    let (near, index) = parse("^.use")?.anchored_in(&[&inside, &outside])?;
    assert_eq!(near.as_str(), "$resource.diagnosis.condition.use");
    assert_eq!(index, 1);
    Ok(())
}

#[test]
fn a_caret_run_past_the_outermost_anchor_is_refused() -> Result<(), Box<dyn core::error::Error>> {
    let inside = parse("$resource")?;
    let outside = parse("$resource.diagnosis")?;
    let refused = parse("^^^.use")?.anchored_in(&[&inside, &outside]);
    assert!(matches!(
        refused,
        Err(AnchorError::AboveResource { hops: 3, .. })
    ));
    Ok(())
}

#[test]
fn anchoring_re_classifies_the_assembled_expression() -> Result<(), Box<dyn core::error::Error>> {
    let anchor = parse("$resource.category.first()")?;
    let bound = parse("coding.code")?.anchored(&anchor)?;
    assert!(matches!(bound.writability(), Writability::ReadOnly { .. }));
    assert_eq!(bound.as_str(), "$resource.category.first().coding.code");
    Ok(())
}
