// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lowering one resolved structure to its type definitions.

use std::collections::{BTreeMap, BTreeSet};

use crate::closure::STRUCTURAL_TYPES;
use crate::lower::Cardinality;
use crate::lower::Docs;
use crate::lower::Field;
use crate::lower::FieldType;
use crate::lower::LowerError;
use crate::lower::Placement;
use crate::lower::RESOURCE_ENUM;
use crate::lower::Target;
use crate::lower::TypeDef;
use crate::lower::TypeKind;
use crate::lower::Variant;
use crate::lower::VersionModule;
use crate::lower::anchored_lexical_form;
use crate::lower::compile_lexical_form;
use crate::lower::scalar_for;
use crate::naming::{backbone_name, field_name, module_name, type_name};
use crate::roots::RootScope;
use crate::snapshot::{ElementShape, Max, ResolvedElement, ResolvedStructure, TypeRef};

/// Whether `target` names a type in the component `own`, the caller's cycle.
fn in_component(
    component_of: &BTreeMap<String, usize>,
    own: Option<usize>,
    target: &Target,
) -> bool {
    match target {
        Target::Named(name) => own.is_some() && component_of.get(name).copied() == own,
        Target::Inline(_) => false,
    }
}

/// Boxes every `T` or `Option<T>` field whose target shares the type's cycle.
pub(super) fn box_cyclic_fields(
    fields: &mut [Field],
    own: Option<usize>,
    component_of: &BTreeMap<String, usize>,
) {
    for field in fields {
        if field.ty.card != Cardinality::Many && in_component(component_of, own, &field.ty.target) {
            field.ty.boxed = true;
        }
    }
}

/// Boxes every choice variant whose target shares the enum's cycle.
pub(super) fn box_cyclic_variants(
    variants: &mut [Variant],
    own: Option<usize>,
    component_of: &BTreeMap<String, usize>,
) {
    for variant in variants {
        if in_component(component_of, own, &variant.target) {
            variant.boxed = true;
        }
    }
}

/// Collects the types `kind` contains directly into `out`.
pub(super) fn contained_types(kind: &TypeKind, out: &mut BTreeSet<String>) {
    match kind {
        TypeKind::Struct { fields } => {
            for field in fields {
                if field.ty.card != Cardinality::Many
                    && let Target::Named(target) = &field.ty.target
                {
                    out.insert(target.clone());
                }
            }
        }
        TypeKind::Choice { variants, .. } => {
            for variant in variants {
                if let Target::Named(target) = &variant.target {
                    out.insert(target.clone());
                }
            }
        }
        TypeKind::ResourceEnum { resources } => {
            out.extend(resources.iter().cloned());
        }
        TypeKind::UnknownResource => {}
    }
}

/// Lowers the base `Element` structure as a table-only entry.
///
/// The entry exists so a table-driven consumer resolves the `id` and
/// `extension` of a primitive's `_name` sibling
/// (<https://hl7.org/fhir/R4/json.html>) against the definition's own elements
/// (<https://hl7.org/fhir/R4/element.html>). It carries the terminology scope
/// because every enabled feature set needs it, and no Rust type is emitted for
/// it: an element typed `Element` lowers to its own nested struct instead.
pub(super) fn lower_element_table_entry(
    model: &mut VersionModule,
    structure: &ResolvedStructure,
) -> Result<(), LowerError> {
    let name = type_name(&structure.name);
    let module = module_name(&name);
    let placement = Placement {
        name: &name,
        module: &module,
        scope: RootScope::Terminology,
        is_primitive: false,
        is_resource: false,
        table_only: true,
    };
    lower_struct(model, structure, &structure.name, placement)
}

pub(super) fn lower_struct(
    model: &mut VersionModule,
    structure: &ResolvedStructure,
    path: &str,
    placement: Placement<'_>,
) -> Result<(), LowerError> {
    let Placement {
        name,
        module,
        scope,
        is_primitive,
        is_resource,
        table_only,
    } = placement;
    let root_docs = structure.element(path).map(docs_of).unwrap_or_default();
    let value_regex = if is_primitive {
        value_lexical_form(structure, path)?
    } else {
        None
    };
    let mut fields = Vec::new();
    for element in structure.children_of(path).cloned().collect::<Vec<_>>() {
        // NOTE: a max of 0 prohibits the element (https://hl7.org/fhir/R4B/conformance-rules.html#cardinality),
        // so it has no field; xhtml prohibits extension this way.
        if element.max == Max::Bounded(0) {
            continue;
        }
        let card = card_of(&element);
        let target = match &element.shape {
            ElementShape::Root => continue,
            ElementShape::ContentReference {
                structure: _,
                path: target_path,
            } => Target::Named(backbone_name(target_path)),
            ElementShape::Choice(types) => {
                let enum_name = format!(
                    "{name}{}",
                    type_name(element.choice_stem().unwrap_or(element.name()))
                );
                let variants = choice_variants(types, &element.path)?;
                model.insert(
                    TypeDef {
                        name: enum_name.clone(),
                        path: element.path.clone(),
                        module: module.to_owned(),
                        docs: Docs {
                            short: Some(format!("The `{}` choice of `{name}`.", element.name())),
                            definition: element.definition.clone(),
                        },
                        kind: TypeKind::Choice {
                            element_path: element.path.clone(),
                            variants,
                        },
                        is_primitive: false,
                        base: None,
                        is_resource: false,
                        table_only: placement.table_only,
                        scope,
                        value_regex: None,
                    },
                    &element.path,
                )?;
                Target::Named(enum_name)
            }
            ElementShape::Typed(types) => {
                lower_typed(model, structure, &element, types, placement)?
            }
        };
        fields.push(Field {
            name: field_name(element.name()),
            fhir_name: element.name().to_owned(),
            path: element.path.clone(),
            docs: docs_of(&element),
            ty: FieldType {
                card,
                target,
                boxed: false,
            },
            min: element.min,
            max: element.max,
            types: type_codes(&element.shape),
            content_reference: content_reference_of(&element.shape),
            is_summary: element.is_summary,
            is_modifier: element.is_modifier,
        });
    }
    model.insert(
        TypeDef {
            name: name.to_owned(),
            path: path.to_owned(),
            module: module.to_owned(),
            docs: root_docs,
            kind: TypeKind::Struct { fields },
            is_primitive,
            is_resource,
            table_only,
            base: is_primitive
                .then_some(structure.base_definition.as_deref())
                .flatten()
                .and_then(|url| url.rsplit('/').next())
                .map(type_name),
            scope,
            value_regex,
        },
        path,
    )
}

/// The lexical form of a primitive's `value` element, checked to compile.
///
/// The form is the `regex` extension the package puts on the value element's
/// type (<https://hl7.org/fhir/R5/datatypes.html#primitive>), so it is read
/// per version and never copied between them.
fn value_lexical_form(
    structure: &ResolvedStructure,
    path: &str,
) -> Result<Option<String>, LowerError> {
    let Some(pattern) = structure
        .children_of(path)
        .find(|element| element.name() == "value")
        .and_then(|element| match &element.shape {
            ElementShape::Typed(types) => types.first().and_then(|first| first.regex.clone()),
            ElementShape::Root
            | ElementShape::ContentReference { .. }
            | ElementShape::Choice(_) => None,
        })
    else {
        return Ok(None);
    };
    compile_lexical_form(&anchored_lexical_form(&pattern)).map_err(|error| {
        LowerError::InvalidLexicalForm {
            path: format!("{path}.value"),
            reason: error.to_string(),
        }
    })?;
    Ok(Some(pattern))
}

/// The type codes an element lists, in the definition's order.
///
/// A content reference carries its children's types on the element it names
/// (<https://hl7.org/fhir/R4/elementdefinition.html>), so it lists none here.
fn type_codes(shape: &ElementShape) -> Vec<String> {
    match shape {
        ElementShape::Root | ElementShape::ContentReference { .. } => Vec::new(),
        ElementShape::Typed(types) | ElementShape::Choice(types) => {
            types.iter().map(|type_ref| type_ref.code.clone()).collect()
        }
    }
}

/// Refuses a `contentReference` the emitted tree cannot spell.
///
/// An element with a `contentReference` takes the children of the element it
/// names (<https://hl7.org/fhir/R4/elementdefinition.html#ElementDefinition.contentReference>),
/// which the emitter writes as that element's backbone type. The type exists
/// only when its structure is in the closure and its feature is enabled
/// wherever the referencing element is, so the target's scope has to be at
/// least as wide as the referrer's.
pub(super) fn check_content_references(
    structures: &BTreeMap<String, ResolvedStructure>,
    scope_of: impl Fn(&str) -> RootScope,
) -> Result<(), LowerError> {
    for structure in structures.values() {
        let scope = scope_of(&structure.name);
        for element in &structure.elements {
            let ElementShape::ContentReference {
                structure: url,
                path: target_path,
            } = &element.shape
            else {
                continue;
            };
            let reference = url.as_ref().map_or_else(
                || format!("#{target_path}"),
                |url| format!("{url}#{target_path}"),
            );
            let unresolved = || LowerError::UnresolvedContentReference {
                path: element.path.clone(),
                reference: reference.clone(),
            };
            let target = match url {
                None => structure,
                Some(url) => structures
                    .values()
                    .find(|candidate| &candidate.url == url)
                    .ok_or_else(unresolved)?,
            };
            if target.element(target_path).is_none() {
                return Err(unresolved());
            }
            let target_scope = scope_of(&target.name);
            if target_scope > scope {
                return Err(LowerError::ContentReferenceOutOfScope {
                    path: element.path.clone(),
                    reference,
                    feature: scope.feature(),
                    target_feature: target_scope.feature(),
                });
            }
        }
    }
    Ok(())
}

/// The element path a `contentReference` names, without the leading `#`.
fn content_reference_of(shape: &ElementShape) -> Option<String> {
    match shape {
        ElementShape::ContentReference { path, .. } => Some(path.clone()),
        _ => None,
    }
}

fn choice_variants(types: &[TypeRef], path: &str) -> Result<Vec<Variant>, LowerError> {
    if types.is_empty() {
        return Err(LowerError::EmptyChoice {
            path: path.to_owned(),
        });
    }
    Ok(types
        .iter()
        .map(|type_ref| {
            let target = if STRUCTURAL_TYPES.contains(&type_ref.code.as_str()) {
                Target::Named(RESOURCE_ENUM.to_owned())
            } else {
                Target::Named(type_name(&type_ref.code))
            };
            Variant {
                name: type_name(&type_ref.code),
                code: type_ref.code.clone(),
                target,
                boxed: false,
            }
        })
        .collect())
}

fn card_of(element: &ResolvedElement) -> Cardinality {
    match (element.min, element.max) {
        (_, Max::Unbounded) => Cardinality::Many,
        (_, Max::Bounded(n)) if n > 1 => Cardinality::Many,
        (0, _) => Cardinality::Optional,
        (_, _) => Cardinality::One,
    }
}

fn docs_of(element: &ResolvedElement) -> Docs {
    Docs {
        short: element.short.clone(),
        definition: element.definition.clone(),
    }
}

/// The target of a single-typed element: an inline scalar for a primitive's
/// value, a nested struct for a backbone, the resource enum, or a named type.
fn lower_typed(
    model: &mut VersionModule,
    structure: &ResolvedStructure,
    element: &ResolvedElement,
    types: &[TypeRef],
    placement: Placement<'_>,
) -> Result<Target, LowerError> {
    let is_primitive = placement.is_primitive;
    let Some(only) = types.first() else {
        return Err(LowerError::EmptyChoice {
            path: element.path.clone(),
        });
    };
    if types.len() > 1 && types.iter().any(|t| t.code != only.code) {
        return Err(LowerError::MultiTyped {
            path: element.path.clone(),
            count: types.len(),
        });
    }
    if only.fhirpath_type.is_some() {
        // NOTE: a primitive's JSON scalar follows the primitive's own name
        // (<https://hl7.org/fhir/R4/json.html#primitive>); the 4.0.1 package
        // tags unsignedInt.value and positiveInt.value as `string`.
        let is_value = element
            .path
            .rsplit_once('.')
            .is_some_and(|(_, last)| last == "value");
        let scalar_name = if is_primitive && is_value {
            structure.name.as_str()
        } else {
            only.code.as_str()
        };
        Ok(Target::Inline(scalar_for(scalar_name)))
    } else if only.code == "BackboneElement" || only.code == "Element" {
        let nested = backbone_name(&element.path);
        lower_struct(model, structure, &element.path, placement.nested(&nested))?;
        Ok(Target::Named(nested))
    } else if only.code == "Resource" {
        Ok(Target::Named(RESOURCE_ENUM.to_owned()))
    } else {
        Ok(Target::Named(type_name(&only.code)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::check_content_references;
    use crate::fhir::StructureKind;
    use crate::lower::LowerError;
    use crate::roots::RootScope;
    use crate::snapshot::{ElementShape, Max, ResolvedElement, ResolvedStructure};

    /// The canonical URL the synthetic structure named `name` carries.
    fn url_of(name: &str) -> String {
        format!("http://example.org/StructureDefinition/{name}")
    }

    fn element(path: &str, shape: ElementShape) -> ResolvedElement {
        ResolvedElement {
            id: None,
            path: path.to_owned(),
            min: 0,
            max: Max::Bounded(1),
            shape,
            binding: None,
            short: None,
            definition: None,
            is_modifier: false,
            is_summary: false,
        }
    }

    /// Two structures: `Narrow` holds one element referencing `reference`, and
    /// `Wide` holds the backbone element `Wide.item` that reference names.
    fn two_structures(reference: ElementShape) -> BTreeMap<String, ResolvedStructure> {
        let structure = |name: &str, elements: Vec<ResolvedElement>| ResolvedStructure {
            url: url_of(name),
            name: name.to_owned(),
            kind: StructureKind::Resource,
            is_abstract: false,
            base_definition: None,
            elements,
        };
        BTreeMap::from([
            (
                String::from("Narrow"),
                structure(
                    "Narrow",
                    vec![
                        element("Narrow", ElementShape::Root),
                        element("Narrow.link", reference),
                    ],
                ),
            ),
            (
                String::from("Wide"),
                structure(
                    "Wide",
                    vec![
                        element("Wide", ElementShape::Root),
                        element("Wide.item", ElementShape::Typed(Vec::new())),
                    ],
                ),
            ),
        ])
    }

    /// `Narrow` is behind `terminology`, `Wide` behind `resources`.
    fn split_scopes(name: &str) -> RootScope {
        if name == "Narrow" {
            RootScope::Terminology
        } else {
            RootScope::Resources
        }
    }

    #[test]
    fn a_content_reference_into_a_narrower_feature_is_refused() {
        let structures = two_structures(ElementShape::ContentReference {
            structure: Some(url_of("Wide")),
            path: String::from("Wide.item"),
        });
        match check_content_references(&structures, split_scopes) {
            Err(LowerError::ContentReferenceOutOfScope {
                path,
                feature,
                target_feature,
                ..
            }) => {
                assert_eq!(path, "Narrow.link");
                assert_eq!(feature, "terminology");
                assert_eq!(target_feature, "resources");
            }
            other => panic!("expected ContentReferenceOutOfScope, got {other:?}"),
        }
    }

    #[test]
    fn a_content_reference_to_a_structure_outside_the_closure_is_refused() {
        let structures = two_structures(ElementShape::ContentReference {
            structure: Some(url_of("Absent")),
            path: String::from("Absent.item"),
        });
        match check_content_references(&structures, split_scopes) {
            Err(LowerError::UnresolvedContentReference { path, reference }) => {
                assert_eq!(path, "Narrow.link");
                assert_eq!(
                    reference,
                    "http://example.org/StructureDefinition/Absent#Absent.item"
                );
            }
            other => panic!("expected UnresolvedContentReference, got {other:?}"),
        }
    }

    #[test]
    fn a_content_reference_to_an_element_the_target_lacks_is_refused() {
        let structures = two_structures(ElementShape::ContentReference {
            structure: Some(url_of("Wide")),
            path: String::from("Wide.nowhere"),
        });
        assert!(matches!(
            check_content_references(&structures, split_scopes),
            Err(LowerError::UnresolvedContentReference { .. })
        ));
    }

    #[test]
    fn a_content_reference_into_a_wider_feature_is_accepted() {
        let structures = two_structures(ElementShape::ContentReference {
            structure: Some(url_of("Wide")),
            path: String::from("Wide.item"),
        });
        let wider = |name: &str| {
            if name == "Narrow" {
                RootScope::Resources
            } else {
                RootScope::Terminology
            }
        };
        assert!(check_content_references(&structures, wider).is_ok());
    }
}
