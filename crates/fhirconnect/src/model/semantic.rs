// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The validation rules a JSON Schema cannot express.
//!
//! A schema sees one file at a time and one key at a time. These rules see the
//! whole loaded set and the relations between keys: a condition whose
//! `targetRoot` does not align with the `with` path it filters, a `criteria`
//! written under an operator that takes none, a cross-file reference that
//! names no loaded mapping, an extension method in a file that is not an
//! extension, a `reference` mapping that does not write `openehr:
//! "$reference"`, and a `mappingCode` naming no registered function.
//!
//! Resolving each path against the operational template and the FHIR element
//! table is a further rule, and it belongs to the resolver rather than here.

use core::fmt;
use core::str::FromStr;
use std::collections::BTreeSet;
use std::path::Path;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::header::MappingType;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::position::Position;

use crate::model::ast::Condition;
use crate::model::ast::ContextMappingFile;
use crate::model::ast::Mapping;
use crate::model::ast::ModelMappingFile;
use crate::model::ast::Variable;
use crate::model::ast::With;
use crate::model::error::ModelCode;
use crate::model::load::MappingSet;

/// The functions a PROGRAMMED mapping may name.
///
/// "In this mapping an external bit of code is retrieved and executed by the
/// engine. The way how this code is provided and where is up to the vendor"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/concept-mappings.adoc`,
/// §Programmed mappings), so the specification fixes no set and the engine
/// supplies one.
pub trait MappingCodeRegistry: fmt::Debug {
    /// Whether a function of this name is registered.
    fn contains(&self, code: &str) -> bool;
}

/// A registry over a fixed set of names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticMappingCodes(BTreeSet<String>);

impl StaticMappingCodes {
    /// Creates a registry holding the given names.
    #[must_use]
    pub fn new<I, S>(codes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(codes.into_iter().map(Into::into).collect())
    }

    /// Returns the registered names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

impl MappingCodeRegistry for StaticMappingCodes {
    fn contains(&self, code: &str) -> bool {
        self.0.contains(code)
    }
}

/// Applies every semantic rule to a loaded set.
///
/// The returned list is empty when the set is valid, and otherwise carries one
/// positioned diagnostic per broken rule, in file and document order.
#[must_use]
pub fn validate(set: &MappingSet, codes: &dyn MappingCodeRegistry) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for model in set.models() {
        validate_model(set, codes, model, &mut diagnostics);
    }
    for context in set.contexts() {
        validate_context(set, context, &mut diagnostics);
    }
    diagnostics
}

/// The one file a rule is about, and everything the rule reads beside it.
#[derive(Debug, Clone, Copy)]
struct FileRules<'a> {
    set: &'a MappingSet,
    codes: &'a dyn MappingCodeRegistry,
    file: &'a ModelMappingFile,
    name: &'a MappingName,
    is_extension: bool,
}

/// Applies the rules that hold for one model or extension mapping file.
fn validate_model(
    set: &MappingSet,
    codes: &dyn MappingCodeRegistry,
    file: &ModelMappingFile,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let rules = FileRules {
        set,
        codes,
        file,
        name: file.header().name().value(),
        is_extension: file.header().mapping_type().map(|t| *t.value())
            == Some(MappingType::Extension),
    };
    let root = ModelPath::root();

    if let Some(ref extends) = file.spec().extends {
        require_model(
            set,
            file.file(),
            rules.name,
            extends,
            &root.field("spec").field("extends"),
            diagnostics,
        );
    }

    if let Some(preprocessor) = file.preprocessor() {
        let path = root.field("preprocessor");
        check_condition(
            file.file(),
            rules.name,
            preprocessor.fhir_condition.as_ref(),
            None,
            &path.field("fhirCondition"),
            diagnostics,
        );
        check_condition(
            file.file(),
            rules.name,
            preprocessor.openehr_condition.as_ref(),
            None,
            &path.field("openehrCondition"),
            diagnostics,
        );
    }

    for (mapping, path) in walk_paths(file.mappings(), &root) {
        validate_mapping(rules, mapping, &path, diagnostics);
    }
}

/// Walks a mapping tree, parents first, with the document path of each method.
///
/// The nesting a mapping can carry is `followedBy.mappings` and
/// `reference.mappings`, so both are walked in document order and every
/// diagnostic names the place the file writes rather than a name-keyed path
/// the document does not have.
fn walk_paths<'a>(mappings: &'a [Mapping], parent: &ModelPath) -> Vec<(&'a Mapping, ModelPath)> {
    let at = parent.field("mappings");
    let mut found = Vec::new();
    for (index, mapping) in mappings.iter().enumerate() {
        let path = at.index(index);
        found.push((mapping, path.clone()));
        if let Some(ref reference) = mapping.reference {
            found.extend(walk_paths(&reference.mappings, &path.field("reference")));
        }
        if let Some(ref followed) = mapping.followed_by {
            found.extend(walk_paths(&followed.mappings, &path.field("followedBy")));
        }
    }
    found
}

/// Applies the condition rules to every entry of a `manual` mapping.
///
/// A manual entry's conditions select the entry rather than filter a `with`
/// path, so only the `criteria` rules apply to them.
fn check_manual_conditions(
    file: &Path,
    name: &MappingName,
    mapping: &Mapping,
    path: &ModelPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (index, entry) in mapping.manual.iter().enumerate() {
        let entry_path = path.field("manual").index(index);
        check_condition(
            file,
            name,
            entry.fhir_condition.as_ref(),
            None,
            &entry_path.field("fhirCondition"),
            diagnostics,
        );
        check_condition(
            file,
            name,
            entry.openehr_condition.as_ref(),
            None,
            &entry_path.field("openehrCondition"),
            diagnostics,
        );
    }
}

/// Applies the rules that hold for one mapping method.
fn validate_mapping(
    rules: FileRules<'_>,
    mapping: &Mapping,
    path: &ModelPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let file = rules.file.file();
    let name = rules.name;
    let side = |pick: fn(&With) -> Option<&Located<String>>| {
        mapping
            .with
            .as_ref()
            .and_then(pick)
            .map(Located::value)
            .map(String::as_str)
    };
    check_condition(
        file,
        name,
        mapping.fhir_condition.as_ref(),
        side(|with| with.fhir.as_ref()),
        &path.field("fhirCondition"),
        diagnostics,
    );
    check_condition(
        file,
        name,
        mapping.openehr_condition.as_ref(),
        side(|with| with.openehr.as_ref()),
        &path.field("openehrCondition"),
        diagnostics,
    );
    check_manual_conditions(file, name, mapping, path, diagnostics);

    if let Some(ref slot) = mapping.slot_archetype {
        require_model(
            rules.set,
            file,
            name,
            slot,
            &path.field("slotArchetype"),
            diagnostics,
        );
    }

    if let Some(ref extension) = mapping.extension
        && !rules.is_extension
    {
        diagnostics.push(error(
            file,
            name,
            ModelCode::ExtensionMethodOutsideExtensionFile,
            extension.position(),
            &path.field("extension"),
            format!(
                "`extension: {}` is an extension method, and the `type` of this file is not \
                 `extension`",
                extension.value()
            ),
        ));
    }

    if let Some(ref reference) = mapping.reference {
        let is_reference_variable = side(|with| with.openehr.as_ref()).is_some_and(|value| {
            Variable::from_str(value).is_ok_and(|variable| variable == Variable::Reference)
        });
        if !is_reference_variable {
            diagnostics.push(error(
                file,
                name,
                ModelCode::ReferenceWithoutReferenceVariable,
                reference.position,
                &path.field("with").field("openehr"),
                "a `reference` mapping initializes a resource, so its `with.openehr` is \
                 `$reference`",
            ));
        }
    }

    if let Some(ref code) = mapping.mapping_code
        && !rules.codes.contains(code.value())
    {
        diagnostics.push(error(
            file,
            name,
            ModelCode::UnknownMappingCode,
            code.position(),
            &path.field("mappingCode"),
            format!(
                "`{}` names no function the engine has registered",
                code.value()
            ),
        ));
    }
}

/// Applies the rules that hold for one context mapping file.
fn validate_context(
    set: &MappingSet,
    file: &ContextMappingFile,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let name = file.header().name().value().clone();
    let path = ModelPath::root().field("context");
    let context = file.context();
    for (key, list) in [
        ("archetypes", &context.archetypes),
        ("extensions", &context.extensions),
        ("operational", &context.operational),
    ] {
        for (index, reference) in list.iter().enumerate() {
            require_model(
                set,
                file.file(),
                &name,
                reference,
                &path.field(key).index(index),
                diagnostics,
            );
        }
    }
    require_model(
        set,
        file.file(),
        &name,
        &context.start,
        &path.field("start"),
        diagnostics,
    );
}

/// Refuses a cross-file reference that names no loaded model or extension.
fn require_model(
    set: &MappingSet,
    file: &Path,
    owner: &MappingName,
    reference: &Located<MappingName>,
    path: &ModelPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if set.model(reference.value()).is_some() {
        return;
    }
    diagnostics.push(error(
        file,
        owner,
        ModelCode::UnknownMappingReference,
        reference.position(),
        path,
        format!(
            "`{}` is no `metadata.name` of the loaded set",
            reference.value()
        ),
    ));
}

/// Applies the two rules that hold for one condition.
fn check_condition(
    file: &Path,
    owner: &MappingName,
    condition: Option<&Condition>,
    with_path: Option<&str>,
    path: &ModelPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(condition) = condition else {
        return;
    };
    let operator = *condition.operator.value();
    if operator.takes_criteria() && condition.criteria.is_empty() {
        diagnostics.push(error(
            file,
            owner,
            ModelCode::CriteriaMissing,
            condition.operator.position(),
            &path.field("criteria"),
            format!("the `{operator}` operator tests the path against a `criteria`"),
        ));
    }
    if !operator.takes_criteria()
        && let Some(first) = condition.criteria.first()
    {
        diagnostics.push(error(
            file,
            owner,
            ModelCode::CriteriaNotAllowed,
            first.position(),
            &path.field("criteria"),
            format!("the `{operator}` operator does not include a `criteria`"),
        ));
    }
    if let Some(with_path) = with_path
        && is_proper_descendant(condition.target_root.value(), with_path)
    {
        diagnostics.push(error(
            file,
            owner,
            ModelCode::ConditionTargetRootMismatch,
            condition.target_root.position(),
            &path.field("targetRoot"),
            format!(
                "`{}` is a child of the `with` path `{with_path}`, so the filtered element does \
                 not match it; name the child in `targetAttributes` instead",
                condition.target_root.value()
            ),
        ));
    }
}

/// Whether `candidate` walks below `ancestor` in either path syntax.
///
/// A condition may point at an unrelated path, which the specification handles
/// as a plain true or false test, and it may point at an ancestor. What it may
/// not do is name a child of the `with` path: "the condition method would try
/// to filter on `type`, which does not match the element in the `with:`
/// statement. This would result in an error"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §targetRoot).
fn is_proper_descendant(candidate: &str, ancestor: &str) -> bool {
    // NOTE: the `not of` example under §criteria of that page writes the shape
    // §targetRoot calls an error, and the rule is implemented as written
    // (reported on issue #181).
    let Some(tail) = candidate.strip_prefix(ancestor) else {
        return false;
    };
    tail.starts_with(['.', '/'])
}

/// Builds one positioned diagnostic.
fn error(
    file: &Path,
    owner: &MappingName,
    code: ModelCode,
    position: Position,
    path: &ModelPath,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic::error(file.to_path_buf(), code.into(), message)
        .with_position(position)
        .with_mapping_name(owner.clone())
        .with_model_path(path.clone())
}

#[cfg(test)]
mod tests {
    use openehr_mapping_core::loader::load_str;

    use super::StaticMappingCodes;
    use super::is_proper_descendant;
    use super::validate;
    use crate::model::error::ModelCode;
    use crate::model::load::MappingSet;

    fn model(name: &str, body: &str) -> String {
        format!(
            "grammar: FHIRConnect/v1.0.0\ntype: model\nmetadata:\n  name: {name}\n  version: \
             1.0.0\nspec:\n  system: FHIR\n  version: R4\n  openEhrConfig:\n    archetype: \
             openEHR-EHR-EVALUATION.test.v1\n{body}"
        )
    }

    fn set_of(sources: &[(&str, String)]) -> MappingSet {
        let mut set = MappingSet::new();
        for (file, source) in sources {
            let document = load_str(*file, source).expect("a well-formed header");
            let parsed =
                crate::model::parse::lower_model(&document).expect("a well-formed model file");
            set.insert_model(parsed).expect("a unique mapping name");
        }
        set
    }

    #[test]
    fn a_criteria_under_empty_is_refused() {
        let set = set_of(&[(
            "a.yml",
            model(
                "EVALUATION.a.v1",
                "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
                 openehr: \"$archetype\"\n    fhirCondition:\n      targetRoot: \
                 \"$resource.code\"\n      targetAttribute: \"coding\"\n      operator: \
                 \"empty\"\n      criteria: \"x\"\n",
            ),
        )]);
        let diagnostics = validate(&set, &StaticMappingCodes::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::CriteriaNotAllowed.into());
    }

    #[test]
    fn a_missing_criteria_under_one_of_is_refused() {
        let set = set_of(&[(
            "a.yml",
            model(
                "EVALUATION.a.v1",
                "mappings:\n  - name: \"a\"\n    with:\n      fhir: \"$resource.code\"\n      \
                 openehr: \"$archetype\"\n    fhirCondition:\n      targetRoot: \
                 \"$resource.code\"\n      targetAttribute: \"coding\"\n      operator: \"one \
                 of\"\n",
            ),
        )]);
        let diagnostics = validate(&set, &StaticMappingCodes::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::CriteriaMissing.into());
    }

    #[test]
    fn a_target_root_below_the_with_path_is_refused() {
        let set = set_of(&[(
            "a.yml",
            model(
                "EVALUATION.a.v1",
                "mappings:\n  - name: \"mapIdentifierForRoom\"\n    with:\n      fhir: \
                 \"$resource.identifier\"\n      openehr: \"$archetype\"\n    fhirCondition:\n    \
                 \x20 targetRoot: \"$resource.identifier.type\"\n      targetAttribute: \
                 \"coding.code\"\n      operator: \"one of\"\n      criteria: \"room\"\n",
            ),
        )]);
        let diagnostics = validate(&set, &StaticMappingCodes::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(
            diagnostic.code(),
            &ModelCode::ConditionTargetRootMismatch.into()
        );
    }

    #[test]
    fn an_unattached_condition_is_accepted() {
        let set = set_of(&[(
            "a.yml",
            model(
                "EVALUATION.a.v1",
                "mappings:\n  - name: \"room\"\n    with:\n      fhir: \
                 \"$resource.location.identifier.value\"\n      openehr: \"$archetype\"\n    \
                 fhirCondition:\n      targetRoot: \"$resource.some.otherPath\"\n      \
                 targetAttribute: \"coding.code\"\n      operator: \"one of\"\n      criteria: \
                 \"someValue\"\n",
            ),
        )]);
        assert!(
            validate(&set, &StaticMappingCodes::default()).is_empty(),
            "an unattached condition is legal"
        );
    }

    #[test]
    fn a_dangling_slot_archetype_is_refused() {
        let set = set_of(&[(
            "a.yml",
            model(
                "EVALUATION.a.v1",
                "mappings:\n  - name: \"slot\"\n    with:\n      fhir: \"$fhirRoot\"\n      \
                 openehr: \"$archetype/data[at0001]\"\n    slotArchetype: \
                 \"CLUSTER.missing.v1\"\n",
            ),
        )]);
        let diagnostics = validate(&set, &StaticMappingCodes::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(
            diagnostic.code(),
            &ModelCode::UnknownMappingReference.into()
        );
        assert!(
            diagnostic.message().contains("CLUSTER.missing.v1"),
            "{}",
            diagnostic.message()
        );
    }

    #[test]
    fn an_extension_method_in_a_model_file_is_refused() {
        let set = set_of(&[(
            "a.yml",
            model(
                "EVALUATION.a.v1",
                "mappings:\n  - name: \"a\"\n    extension: \"add\"\n    with:\n      fhir: \
                 \"$resource\"\n      openehr: \"$archetype\"\n",
            ),
        )]);
        let diagnostics = validate(&set, &StaticMappingCodes::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(
            diagnostic.code(),
            &ModelCode::ExtensionMethodOutsideExtensionFile.into()
        );
    }

    #[test]
    fn a_reference_mapping_without_the_reference_variable_is_refused() {
        let set = set_of(&[(
            "a.yml",
            model(
                "EVALUATION.a.v1",
                "mappings:\n  - name: \"specimen\"\n    with:\n      fhir: \
                 \"$resource.specimen.reference\"\n      openehr: \"$archetype/data[at0003]\"\n   \
                 \x20reference:\n      resourceType: \"Specimen\"\n      mappings:\n        - \
                 name: \"inner\"\n          with:\n            fhir: \"$fhirRoot\"\n            \
                 openehr: \"$archetype\"\n",
            ),
        )]);
        let diagnostics = validate(&set, &StaticMappingCodes::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(
            diagnostic.code(),
            &ModelCode::ReferenceWithoutReferenceVariable.into()
        );
    }

    #[test]
    fn a_reference_mapping_with_the_reference_variable_is_accepted() {
        let set = set_of(&[(
            "a.yml",
            model(
                "EVALUATION.a.v1",
                "mappings:\n  - name: \"specimen\"\n    with:\n      fhir: \
                 \"$resource.specimen.reference\"\n      openehr: \"$reference\"\n    \
                 reference:\n      resourceType: \"Specimen\"\n      mappings:\n        - name: \
                 \"inner\"\n          with:\n            fhir: \"$fhirRoot\"\n            \
                 openehr: \"$archetype\"\n",
            ),
        )]);
        assert!(validate(&set, &StaticMappingCodes::default()).is_empty());
    }

    #[test]
    fn an_unregistered_mapping_code_is_refused_and_a_registered_one_is_not() {
        let source = model(
            "EVALUATION.a.v1",
            "mappings:\n  - name: \"dosageTiming\"\n    with:\n      fhir: \
             \"$fhirRoot.timing\"\n      openehr: \"$archetype\"\n    mappingCode: \
             \"timingToDaily_NonDaily\"\n",
        );
        let set = set_of(&[("a.yml", source)]);
        let diagnostics = validate(&set, &StaticMappingCodes::default());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let diagnostic = diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.code(), &ModelCode::UnknownMappingCode.into());
        let registry = StaticMappingCodes::new(["timingToDaily_NonDaily"]);
        assert_eq!(registry.names().count(), 1);
        assert!(validate(&set, &registry).is_empty());
    }

    #[test]
    fn a_path_below_another_is_a_proper_descendant() {
        assert!(is_proper_descendant(
            "$resource.identifier.type",
            "$resource.identifier"
        ));
        assert!(is_proper_descendant(
            "$archetype/data[at0001]/items[at0002]",
            "$archetype/data[at0001]"
        ));
        assert!(!is_proper_descendant(
            "$resource.identifier",
            "$resource.identifier"
        ));
        assert!(!is_proper_descendant(
            "$resource.identifierType",
            "$resource.identifier"
        ));
        assert!(!is_proper_descendant(
            "$resource.some.otherPath",
            "$resource.location"
        ));
    }
}
