// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The lowering from a positioned YAML tree to the file model.

use openehr_mapping_core::loader::load_str;
use openehr_mapping_core::position::Position;

use fhirconnect::model::ast::keyword::ConditionOperator;
use fhirconnect::model::ast::keyword::DataType;
use fhirconnect::model::ast::keyword::Direction;
use fhirconnect::model::error::ModelCode;
use fhirconnect::model::parse::lower_context;
use fhirconnect::model::parse::lower_model;

/// The smallest model mapping file the grammar admits, plus `mappings`.
fn model(body: &str) -> String {
    format!(
        "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: EVALUATION.test.v1\n  \
         version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  openEhrConfig:\n    \
         archetype: openEHR-EHR-EVALUATION.test.v1\n{body}"
    )
}

#[test]
fn a_minimal_model_file_lowers() {
    let document = load_str(
        "model.yml",
        &model(
            "mappings:\n  - name: \"dateTime\"\n    with:\n      fhir: \"$resource.onset\"\n \
             \x20    openehr: \"$archetype/data[at0001]/items[at0077]\"\n",
        ),
    )
    .expect("a well-formed header");
    let file = lower_model(&document).expect("a well-formed model file");
    assert_eq!(file.mappings().len(), 1);
    let mapping = file.mappings().first().expect("one mapping");
    assert_eq!(mapping.name.value(), "dateTime");
    let with = mapping.with.as_ref().expect("a with block");
    assert_eq!(
        with.openehr.as_ref().map(|p| p.value().as_str()),
        Some("$archetype/data[at0001]/items[at0077]")
    );
}

#[test]
fn the_singular_and_plural_condition_spellings_lower_alike() {
    let singular = load_str(
        "singular.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
             openehr: \"$archetype\"\n    fhirCondition:\n      targetRoot: \
             \"$resource.code\"\n      targetAttribute: \"coding.code\"\n      operator: \"one \
             of\"\n      criteria: \"x\"\n",
        ),
    )
    .expect("a well-formed header");
    let plural = load_str(
        "plural.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
             openehr: \"$archetype\"\n    fhirCondition:\n      targetRoot: \
             \"$resource.code\"\n      targetAttributes:\n        - \"coding.code\"\n      \
             operator: \"one of\"\n      criterias:\n        - \"x\"\n",
        ),
    )
    .expect("a well-formed header");
    let singular = lower_model(&singular).expect("the singular spelling lowers");
    let plural = lower_model(&plural).expect("the plural spelling lowers");
    let left = singular
        .mappings()
        .first()
        .and_then(|m| m.fhir_condition.as_ref())
        .expect("a condition");
    let right = plural
        .mappings()
        .first()
        .and_then(|m| m.fhir_condition.as_ref())
        .expect("a condition");
    let attributes = |c: &fhirconnect::model::ast::Condition| {
        c.target_attributes
            .iter()
            .map(|a| a.value().clone())
            .collect::<Vec<String>>()
    };
    let criteria = |c: &fhirconnect::model::ast::Condition| {
        c.criteria
            .iter()
            .map(|a| a.value().clone())
            .collect::<Vec<String>>()
    };
    assert_eq!(attributes(left), attributes(right));
    assert_eq!(criteria(left), criteria(right));
    assert_eq!(*left.operator.value(), ConditionOperator::OneOf);
}

#[test]
fn a_two_attribute_condition_keeps_both_attributes() {
    let document = load_str(
        "two.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
             openehr: \"$archetype\"\n    openehrCondition:\n      targetRoot: \
             \"$archetype\"\n      targetAttributes:\n        - \"items[at0071]\"\n        - \
             \"items[at0115]\"\n      operator: \"empty\"\n",
        ),
    )
    .expect("a well-formed header");
    let file = lower_model(&document).expect("a well-formed model file");
    let condition = file
        .mappings()
        .first()
        .and_then(|m| m.openehr_condition.as_ref())
        .expect("a condition");
    let attributes: Vec<&str> = condition
        .target_attributes
        .iter()
        .map(|a| a.value().as_str())
        .collect();
    assert_eq!(attributes, vec!["items[at0071]", "items[at0115]"]);
}

#[test]
fn a_null_mappings_is_refused() {
    let document = load_str("null.yml", &model("mappings:\n")).expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("a null mappings");
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::NullMappings.into());
}

#[test]
fn a_root_unidirectional_is_refused_naming_the_schema() {
    let document = load_str(
        "root.yml",
        &model("unidirectional: \"openehr->fhir\"\nmappings: []\n"),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("a root unidirectional");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::RootUnidirectional.into());
    assert!(
        diagnostic.message().contains("model-mapping.schema.json"),
        "{}",
        diagnostic.message()
    );
}

#[test]
fn a_spec_unidirectional_is_accepted() {
    let document = load_str(
        "spec.yml",
        "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: provenance\n  version: \
         1.0.0\nspec:\n  system: FHIR\n  version: R4\n  unidirectional: \
         \"openehr->fhir\"\nmappings: []\n",
    )
    .expect("a well-formed header");
    let file = lower_model(&document).expect("a well-formed operational model file");
    assert_eq!(
        file.spec().unidirectional.as_ref().map(|d| *d.value()),
        Some(Direction::OpenehrToFhir)
    );
}

#[test]
fn an_unknown_path_variable_is_refused() {
    let document = load_str(
        "variable.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
             openehr: \"$archetypeRoot/data\"\n",
        ),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("an unknown variable");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::UnknownPathVariable.into());
}

#[test]
fn a_mis_cased_variable_is_accepted() {
    let document = load_str(
        "cased.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$fhirRoot\"\n      openehr: \
             \"$openEHRRoot/defining_code\"\n",
        ),
    )
    .expect("a well-formed header");
    let file = lower_model(&document).expect("a mis-cased variable is admitted");
    assert_eq!(file.mappings().len(), 1);
}

#[test]
fn an_unknown_mapping_key_is_refused() {
    let document = load_str(
        "unknown.yml",
        &model("mappings:\n  - name: \"a\"\n    hardcodedValue: \"x\"\n"),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("an unknown key");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::UnknownKey.into());
    assert_eq!(
        diagnostic.model_path().to_string(),
        "mappings[0].hardcodedValue"
    );
}

#[test]
fn a_data_type_inside_with_lowers() {
    let document = load_str(
        "type.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource\"\n      openehr: \
             \"$archetype\"\n      type: \"NONE\"\n",
        ),
    )
    .expect("a well-formed header");
    let file = lower_model(&document).expect("a well-formed model file");
    let with = file
        .mappings()
        .first()
        .and_then(|m| m.with.as_ref())
        .expect("a with block");
    assert_eq!(
        with.data_type.as_ref().map(|t| *t.value()),
        Some(DataType::None)
    );
}

#[test]
fn a_context_file_lowers() {
    let document = load_str(
        "context.yml",
        "grammar: FHIRConnect/v1.0.0\ntype: context\nmetadata:\n  name: KDS.context\n  \
         version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\ncontext:\n  profile:\n    url: \
         \"https://example.invalid/StructureDefinition/Procedure\"\n    version: \
         \"2025.0.0\"\n  template:\n    id: \"KDS_Prozedur\"\n    sem_ver: \"10.0.0\"\n  \
         archetypes:\n    - \"ACTION.procedure.v1\"\n  extensions:\n    - \
         \"KDS_procedure.v1\"\n  start: \"ACTION.procedure.v1\"\n",
    )
    .expect("a well-formed header");
    let file = lower_context(&document).expect("a well-formed context file");
    assert_eq!(file.context().archetypes.len(), 1);
    assert_eq!(file.context().start.value().as_str(), "ACTION.procedure.v1");
    assert_eq!(file.context().position, Position::new(10, 3));
}

#[test]
fn a_mapping_without_a_name_is_refused() {
    let document = load_str(
        "nameless.yml",
        &model(
            "mappings:\n  - with:\n      fhir: \"$resource.code\"\n      openehr: \"$archetype\"\n",
        ),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("a mapping with no name");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::MissingKey.into());
    assert_eq!(diagnostic.model_path().to_string(), "mappings[0].name");
}

#[test]
fn a_with_that_is_not_a_block_is_refused() {
    let document = load_str(
        "kind.yml",
        &model("mappings:\n  - name: \"a\"\n    with: \"$resource.code\"\n"),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("a with that is not a block");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::UnexpectedNodeKind.into());
    assert_eq!(diagnostic.model_path().to_string(), "mappings[0].with");
}

#[test]
fn a_direction_outside_the_two_spellings_is_refused() {
    let document = load_str(
        "direction.yml",
        &model("mappings:\n  - name: \"a\"\n    unidirectional: \"sideways\"\n"),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("an unknown direction");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::InvalidDirection.into());
    assert_eq!(
        diagnostic.model_path().to_string(),
        "mappings[0].unidirectional"
    );
}

#[test]
fn an_extension_method_outside_the_three_is_refused() {
    let document = load_str(
        "method.yml",
        "grammar: FHIRConnect/v1.0.0\ntype: extension\nmetadata:\n  name: test_extension\n  \
         version: 1.0.0\nspec:\n  system: FHIR\n  version: R4\n  extends: \
         EVALUATION.test.v1\nmappings:\n  - name: \"a\"\n    extension: \"replace\"\n",
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("an unknown extension method");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::InvalidExtensionMethod.into());
    assert_eq!(diagnostic.model_path().to_string(), "mappings[0].extension");
}

#[test]
fn a_mapping_level_data_type_reaches_the_with() {
    let document = load_str(
        "level.yml",
        &model("mappings:\n  - name: \"a\"\n    type: \"NONE\"\n"),
    )
    .expect("a well-formed header");
    let file = lower_model(&document).expect("a well-formed model file");
    let with = file
        .mappings()
        .first()
        .and_then(|m| m.with.as_ref())
        .expect("the data type reaches a with block of its own");
    assert_eq!(
        with.data_type.as_ref().map(|t| *t.value()),
        Some(DataType::None)
    );
    assert!(with.fhir.is_none());
}

#[test]
fn two_disagreeing_data_types_are_refused() {
    let document = load_str(
        "both.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    type: \"NONE\"\n    with:\n      fhir: \
             \"$resource\"\n      openehr: \"$archetype\"\n      type: \"CODING\"\n",
        ),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("two data types that disagree");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::ConflictingDataType.into());
    assert_eq!(diagnostic.model_path().to_string(), "mappings[0].type");
}

#[test]
fn a_data_type_outside_the_enum_is_refused() {
    let document = load_str(
        "datatype.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource\"\n      openehr: \
             \"$archetype\"\n      type: \"MONEY\"\n",
        ),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("an unknown data type");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::InvalidDataType.into());
    assert_eq!(diagnostic.model_path().to_string(), "mappings[0].with.type");
}

#[test]
fn a_condition_operator_outside_the_five_is_refused() {
    let document = load_str(
        "operator.yml",
        &model(
            "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
             openehr: \"$archetype\"\n    fhirCondition:\n      targetRoot: \
             \"$resource.code\"\n      targetAttribute: \"coding.code\"\n      operator: \
             \"maybe\"\n",
        ),
    )
    .expect("a well-formed header");
    let diagnostics = lower_model(&document).expect_err("an unknown operator");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::InvalidOperator.into());
    assert_eq!(
        diagnostic.model_path().to_string(),
        "mappings[0].fhirCondition.operator"
    );
}

#[test]
fn a_model_file_is_refused_by_the_context_entry_point() {
    let document = load_str("model.yml", &model("mappings: []\n")).expect("a header");
    let diagnostics = lower_context(&document).expect_err("the wrong file type");
    let diagnostic = diagnostics.first().expect("one diagnostic");
    assert_eq!(diagnostic.code(), &ModelCode::WrongMappingFileType.into());
}
