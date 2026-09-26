// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lowering one operation parameter to a contract field.

use crate::ecosystem::ParameterSource;
use crate::fhir::OperationParameter;
use crate::lower::{Cardinality, VersionModule};
use crate::naming::{field_name, type_name};
use crate::operations::ContractField;
use crate::operations::FieldKind;
use crate::operations::OPEN_TYPE_ENUM;
use crate::operations::OperationError;
use crate::snapshot::Max;

pub(super) fn lower_field(
    parameter: &OperationParameter,
    owner: &str,
    operation: &str,
    module: &VersionModule,
    defaultable_types: &std::collections::BTreeSet<String>,
) -> Result<ContractField, OperationError> {
    let max = parse_max(&parameter.max).ok_or_else(|| OperationError::InvalidMax {
        operation: operation.to_owned(),
        name: parameter.name.clone(),
        max: parameter.max.clone(),
    })?;
    let part_struct =
        (!parameter.part.is_empty()).then(|| format!("{owner}{}", pascal(&parameter.name)));
    let mut kind = FieldKind::Parts;
    let rust_type = match (&parameter.type_name, &part_struct) {
        (_, Some(nested)) => nested.clone(),
        (Some(code), None) if code == "Element" => {
            kind = FieldKind::OpenType;
            format!("super::super::parameters::{OPEN_TYPE_ENUM}")
        }
        (Some(code), None) => {
            let name = type_name(code);
            let ty = module
                .types
                .get(&name)
                .ok_or_else(|| OperationError::UnknownType {
                    operation: operation.to_owned(),
                    name: parameter.name.clone(),
                    code: code.clone(),
                })?;
            kind = if ty.is_resource {
                FieldKind::Resource(name.clone())
            } else if code == "Resource" {
                FieldKind::AnyResource
            } else {
                FieldKind::Value(name.clone())
            };
            format!("super::super::{}::{name}", ty.module)
        }
        (None, None) => {
            return Err(OperationError::Untyped {
                operation: operation.to_owned(),
                name: parameter.name.clone(),
            });
        }
    };
    let mut parts = Vec::new();
    if let Some(nested) = &part_struct {
        for part in &parameter.part {
            parts.push(lower_field(
                part,
                nested,
                operation,
                module,
                defaultable_types,
            )?);
        }
    }
    let defaultable = match &kind {
        FieldKind::Value(name) | FieldKind::Resource(name) => defaultable_types.contains(name),
        FieldKind::OpenType | FieldKind::AnyResource => false,
        FieldKind::Parts => derives_default(&parts),
    };
    let accepts = match &kind {
        FieldKind::Value(name) => {
            let mut both = specializations(module, name);
            for wider in generalizations(module, name) {
                if !both.contains(&wider) {
                    both.push(wider);
                }
            }
            both.sort();
            both
        }
        _ => Vec::new(),
    };
    let boxed = match &kind {
        FieldKind::Value(name) => !module.types.get(name).is_some_and(|ty| ty.is_primitive),
        _ => false,
    };
    Ok(ContractField {
        name: field_name(&parameter.name),
        fhir_name: parameter.name.clone(),
        documentation: parameter.documentation.clone(),
        usage: parameter.usage,
        min: parameter.min,
        max,
        type_code: parameter.type_name.clone(),
        rust_type,
        scope: parameter.scope.clone(),
        parts,
        part_struct,
        kind,
        defaultable,
        source: ParameterSource::Version,
        accepts,
        boxed,
    })
}

/// The primitives of `module` that specialize `name` (through their
/// `baseDefinition` chain) and carry the same scalar, so a value sent as one
/// of them reads as `name`: a `code` is a `string`, a `canonical` is a `uri`
/// (<https://hl7.org/fhir/R5/datatypes.html#primitive>).
fn specializations(module: &VersionModule, name: &str) -> Vec<String> {
    let Some(base) = module.types.get(name).filter(|t| t.is_primitive) else {
        return Vec::new();
    };
    let code = |rust: &str| {
        let mut chars = rust.chars();
        chars
            .next()
            .map(|c| c.to_ascii_lowercase().to_string() + chars.as_str())
            .unwrap_or_default()
    };
    let scalar = crate::lower::scalar_for(&code(&base.name));
    module
        .types
        .values()
        .filter(|t| t.is_primitive && t.name != name)
        .filter(|t| {
            let mut current = t.base.as_deref();
            let mut hops = 0;
            while let Some(step) = current {
                if step == name {
                    return true;
                }
                hops += 1;
                if hops > 8 {
                    break;
                }
                current = module.types.get(step).and_then(|b| b.base.as_deref());
            }
            false
        })
        .filter(|t| crate::lower::scalar_for(&code(&t.name)) == scalar)
        .map(|t| t.name.clone())
        .collect()
}

/// The primitives `name` specializes (through its own `baseDefinition` chain)
/// that carry the same scalar, so a value sent as one of them reads as `name`:
/// a `uri` reads as a `canonical`
/// (<https://hl7.org/fhir/R5/datatypes.html#primitive>).
///
/// `canonical` and `uri` are distinct types that "are never substituted for
/// each other" (<https://hl7.org/fhir/R4B/datatypes.html#uri>), so this is a
/// decision rather than a reading of the type system: their value spaces are
/// identical, no clause requires a server to refuse the wider spelling, and
/// the GET form of an operation delivers the value with no type marker at all,
/// so the server already applies the declared type by parsing
/// (<https://hl7.org/fhir/R4B/operations.html>). Recorded on #352.
fn generalizations(module: &VersionModule, name: &str) -> Vec<String> {
    let Some(declared) = module.types.get(name).filter(|t| t.is_primitive) else {
        return Vec::new();
    };
    let code = |rust: &str| {
        let mut chars = rust.chars();
        chars
            .next()
            .map(|c| c.to_ascii_lowercase().to_string() + chars.as_str())
            .unwrap_or_default()
    };
    let scalar = crate::lower::scalar_for(&code(&declared.name));
    let mut out = Vec::new();
    let mut current = declared.base.as_deref();
    let mut hops = 0;
    while let Some(step) = current {
        let Some(base) = module.types.get(step) else {
            break;
        };
        if base.is_primitive && crate::lower::scalar_for(&code(&base.name)) == scalar {
            out.push(base.name.clone());
        }
        hops += 1;
        if hops > 8 {
            break;
        }
        current = base.base.as_deref();
    }
    out
}

/// Whether a struct of `fields` derives `Default`: every required field's
/// type does (the rule `render::defaultable` applies to the model types).
pub(super) fn derives_default(fields: &[ContractField]) -> bool {
    fields
        .iter()
        .all(|field| cardinality(field.min, field.max) != Cardinality::One || field.defaultable)
}

/// Keeps field names unique within one struct: when two parameters reduce to
/// the same Rust name (R6's `ValueSet/$validate-code` declares both
/// `systemVersion` and `system-version`), the hyphenated one gets its type
/// code as a suffix (`system_version_canonical`). No spec governs the Rust
/// names: our own rule.
pub(super) fn disambiguate(fields: &mut [ContractField]) {
    let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for field in fields.iter() {
        *seen.entry(field.name.clone()).or_insert(0) += 1;
    }
    for field in fields.iter_mut() {
        if seen.get(&field.name).copied().unwrap_or(0) > 1
            && field.fhir_name.contains('-')
            && let Some(code) = &field.type_code
        {
            field.name = format!("{}_{}", field.name, field_name(code));
        }
        disambiguate(&mut field.parts);
    }
}

fn parse_max(max: &str) -> Option<Max> {
    if max == "*" {
        Some(Max::Unbounded)
    } else {
        max.parse().ok().map(Max::Bounded)
    }
}

/// `validate-code` to `ValidateCode`, `find-matches` to `FindMatches`.
pub(super) fn pascal(code: &str) -> String {
    code.split(['-', '_']).map(type_name).collect()
}

pub(super) fn cardinality(min: u32, max: Max) -> Cardinality {
    match (min, max) {
        (_, Max::Unbounded) => Cardinality::Many,
        (_, Max::Bounded(n)) if n > 1 => Cardinality::Many,
        (0, _) => Cardinality::Optional,
        (_, _) => Cardinality::One,
    }
}

#[cfg(test)]
mod tests {
    use super::pascal;

    #[test]
    fn codes_become_pascal_case() {
        assert_eq!(pascal("lookup"), "Lookup");
        assert_eq!(pascal("validate-code"), "ValidateCode");
        assert_eq!(pascal("find-matches"), "FindMatches");
    }
}
