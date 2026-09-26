// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The keyword values of the grammar, each parsed against its documented set.

use core::str::FromStr;

use fhirconnect::model::ast::keyword::ConditionOperator;
use fhirconnect::model::ast::keyword::DataType;
use fhirconnect::model::ast::keyword::Direction;
use fhirconnect::model::ast::keyword::ExtensionMethod;
use fhirconnect::model::ast::keyword::Variable;

#[test]
fn both_documented_direction_spellings_parse() {
    assert_eq!(
        Direction::from_str("openehr->fhir"),
        Ok(Direction::OpenehrToFhir)
    );
    assert_eq!(
        Direction::from_str("fhir->openehr"),
        Ok(Direction::FhirToOpenehr)
    );
}

#[test]
fn a_direction_compares_case_insensitively() {
    assert_eq!(
        Direction::from_str("openEHR->fhir"),
        Ok(Direction::OpenehrToFhir)
    );
    assert_eq!(
        Direction::from_str("fhir->openEHR"),
        Ok(Direction::FhirToOpenehr)
    );
}

#[test]
fn a_direction_outside_the_set_is_refused_naming_the_set() {
    let error = Direction::from_str("openehr<->fhir").expect_err("a value outside the set");
    assert_eq!(error.value, "openehr<->fhir");
    assert_eq!(
        error.to_string(),
        "`openehr<->fhir` is not one of `openehr->fhir`, `fhir->openehr`"
    );
}

#[test]
fn an_extension_method_is_case_exact() {
    assert_eq!(ExtensionMethod::from_str("add"), Ok(ExtensionMethod::Add));
    assert!(ExtensionMethod::from_str("Add").is_err());
}

#[test]
fn a_data_type_is_case_exact() {
    assert_eq!(DataType::from_str("NONE"), Ok(DataType::None));
    assert!(DataType::from_str("none").is_err());
}

#[test]
fn an_operator_is_case_exact_and_knows_whether_it_takes_criteria() {
    assert_eq!(
        ConditionOperator::from_str("not empty"),
        Ok(ConditionOperator::NotEmpty)
    );
    assert!(!ConditionOperator::NotEmpty.takes_criteria());
    assert!(!ConditionOperator::Empty.takes_criteria());
    assert!(ConditionOperator::OneOf.takes_criteria());
    assert!(ConditionOperator::NotOf.takes_criteria());
    assert!(ConditionOperator::Type.takes_criteria());
    assert!(ConditionOperator::from_str("One Of").is_err());
}

#[test]
fn a_variable_compares_case_insensitively_with_or_without_the_sigil() {
    assert_eq!(
        Variable::from_str("$openehrRoot"),
        Ok(Variable::OpenehrRoot)
    );
    assert_eq!(
        Variable::from_str("$openEHRRoot"),
        Ok(Variable::OpenehrRoot)
    );
    assert_eq!(Variable::from_str("archetype"), Ok(Variable::Archetype));
    assert_eq!(Variable::OpenehrRoot.to_string(), "$openehrRoot");
}

#[test]
fn a_variable_outside_the_set_is_refused() {
    let error = Variable::from_str("$openehrParent").expect_err("a name outside the set");
    assert_eq!(error.admitted, Variable::ADMITTED);
}
