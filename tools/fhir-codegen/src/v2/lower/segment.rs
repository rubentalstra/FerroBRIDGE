// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Segments: the element list, the field positions and cardinalities, and each field.

use crate::fhir::{Derivation, StructureKind};
use crate::roots::V2_SEGMENT_BASE;
use crate::v2::corpus::Corpus;
use crate::v2::definition::{Element, Scalar};
use crate::v2::lower::Cardinality;
use crate::v2::lower::Defect;
use crate::v2::lower::Field;
use crate::v2::lower::Input;
use crate::v2::lower::LowerError;
use crate::v2::lower::NumberError;
use crate::v2::lower::Optionality;
use crate::v2::lower::Segment;
use crate::v2::lower::TABLE_VALUE_SET;
use crate::v2::lower::Table;
use crate::v2::lower::V2_CANONICAL;
use crate::v2::lower::defect;
use crate::v2::lower::extension::element_extensions;
use crate::v2::lower::invalid;
use crate::v2::lower::number;

/// The checks every segment and structure definition passes, returning its elements.
pub(super) fn elements<'a>(sourced: &Input<'a>, base: &str) -> Result<&'a [Element], LowerError> {
    let definition = &sourced.definition;
    let id = definition.id.as_str();
    if definition.kind != StructureKind::Logical
        || definition.is_abstract
        || definition.derivation != Some(Derivation::Specialization)
        || definition.base_definition.as_deref() != Some(base)
    {
        return Err(invalid(
            sourced,
            id,
            format!("not a concrete logical specialization of {base}"),
        ));
    }
    if definition.url != format!("{V2_CANONICAL}{id}") {
        return Err(invalid(
            sourced,
            id,
            format!("canonical URL {} does not end in its id", definition.url),
        ));
    }
    // NOTE: https://hl7.org/fhir/R5/structuredefinition.html states a differential
    // relative to its base; no v2 file ships a snapshot and every base is
    // element-less, so the differential is read as the snapshot.
    if definition.snapshot.is_some() {
        return Err(invalid(
            sourced,
            id,
            "a snapshot is present; the lowering reads the differential as the snapshot",
        ));
    }
    let Some(differential) = &definition.differential else {
        return Err(invalid(sourced, id, "no differential"));
    };
    let elements = differential.element.as_slice();
    let Some((root, _)) = elements.split_first() else {
        return Err(invalid(sourced, id, "the differential has no elements"));
    };
    if root.id != id {
        return Err(invalid(
            sourced,
            &root.id,
            format!("the first element is not the root {id}"),
        ));
    }
    for element in elements {
        if element.path != element.id {
            return Err(invalid(
                sourced,
                &element.id,
                format!("path {} differs from the id", element.path),
            ));
        }
        if element.defintion.is_some() {
            defect(sourced, &element.id, Defect::MisspelledDefinition)?;
        }
    }
    Ok(elements)
}

pub(super) fn cardinality(
    sourced: &Input<'_>,
    element: &Element,
) -> Result<Cardinality, LowerError> {
    let Some(min) = element.min else {
        return Err(invalid(sourced, &element.id, "no min"));
    };
    let max = match &element.max {
        None => return Err(invalid(sourced, &element.id, "no max")),
        Some(Scalar::Text(text)) if text == "*" => None,
        Some(Scalar::Text(text)) => Some(
            text.parse::<u32>()
                .map_err(NumberError::from)
                .map_err(number(sourced, element, text))?,
        ),
        Some(Scalar::Integer(count)) => {
            defect(sourced, &element.id, Defect::NumberMax)?;
            Some(
                u32::try_from(*count)
                    .map_err(NumberError::from)
                    .map_err(number(sourced, element, count))?,
            )
        }
    };
    if let Some(max) = max
        && min > max
    {
        defect(sourced, &element.id, Defect::MinAboveMax)?;
    }
    Ok(Cardinality { min, max })
}

pub(super) fn position(
    sourced: &Input<'_>,
    element: &str,
    digits: &str,
) -> Result<u16, LowerError> {
    let valid = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0');
    let parsed = if valid {
        digits.parse::<u16>().ok()
    } else {
        None
    };
    parsed.ok_or_else(|| {
        invalid(
            sourced,
            element,
            format!("position {digits:?} is not a number from 1"),
        )
    })
}

/// The single type code of an element, or `None` when it has no type.
pub(super) fn single_type<'a>(
    sourced: &Input<'_>,
    element: &'a Element,
) -> Result<Option<&'a str>, LowerError> {
    match element.types.as_deref() {
        None => Ok(None),
        Some([only]) => Ok(Some(only.code.as_str())),
        Some(_) => Err(invalid(sourced, &element.id, "not exactly one type")),
    }
}

pub(super) fn lower_segment(corpus: &Corpus, sourced: &Input<'_>) -> Result<Segment, LowerError> {
    let elements = elements(sourced, V2_SEGMENT_BASE)?;
    let id = sourced.definition.id.as_str();
    let Some((root, fields)) = elements.split_first() else {
        return Err(invalid(sourced, id, "the differential has no elements"));
    };
    let name = root
        .short
        .clone()
        .ok_or_else(|| invalid(sourced, &root.id, "the root element has no short name"))?;
    let mut lowered = Vec::with_capacity(fields.len());
    for (index, element) in fields.iter().enumerate() {
        lowered.push(lower_field(corpus, sourced, element, index)?);
    }
    Ok(Segment {
        id: id.to_owned(),
        url: Some(sourced.definition.url.clone()),
        name,
        fields: lowered,
    })
}

/// The data type code of a field: absent exactly when its optionality is `W`.
pub(super) fn data_type(
    corpus: &Corpus,
    sourced: &Input<'_>,
    element: &Element,
    optionality: Optionality,
) -> Result<Option<String>, LowerError> {
    match (single_type(sourced, element)?, optionality) {
        (None, Optionality::W) => Ok(None),
        (None, _) => Err(invalid(
            sourced,
            &element.id,
            "an untyped field whose optionality is not W",
        )),
        (Some(_), Optionality::W) => Err(invalid(
            sourced,
            &element.id,
            "a typed field whose optionality is W",
        )),
        (Some(code), _) => {
            if !corpus
                .data_types()
                .contains_key(&format!("{V2_CANONICAL}{code}"))
            {
                defect(sourced, &element.id, Defect::UndefinedDataType)?;
            }
            Ok(Some(code.to_owned()))
        }
    }
}

/// The table a field's binding names, from its `v2-NNNN` value set URL.
pub(super) fn table(sourced: &Input<'_>, element: &Element) -> Result<Option<Table>, LowerError> {
    let Some(binding) = &element.binding else {
        return Ok(None);
    };
    if binding.strength != "required" {
        return Err(invalid(
            sourced,
            &element.id,
            format!("binding strength {}", binding.strength),
        ));
    }
    let number = binding
        .value_set
        .strip_prefix(TABLE_VALUE_SET)
        .filter(|number| number.len() == 4 && number.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| {
            invalid(
                sourced,
                &element.id,
                format!("binding {} names no v2 table", binding.value_set),
            )
        })?;
    Ok(Some(Table {
        id: number.to_owned(),
        value_set: binding.value_set.clone(),
    }))
}

fn lower_field(
    corpus: &Corpus,
    sourced: &Input<'_>,
    element: &Element,
    index: usize,
) -> Result<Field, LowerError> {
    let segment = sourced.definition.id.as_str();
    let step = element
        .id
        .strip_prefix(segment)
        .and_then(|rest| rest.strip_prefix('.'))
        .ok_or_else(|| invalid(sourced, &element.id, format!("not a field of {segment}")))?;
    let Some((digits, key)) = step.split_once('-') else {
        return Err(invalid(sourced, &element.id, "a field id is SEG.n-key"));
    };
    if key.is_empty() || key.contains('.') {
        return Err(invalid(sourced, &element.id, "a field id is SEG.n-key"));
    }
    let position = position(sourced, &element.id, digits)?;
    if usize::from(position) != index + 1 {
        return Err(invalid(
            sourced,
            &element.id,
            format!("field {} out of order", index + 1),
        ));
    }
    if element.content_reference.is_some() {
        return Err(invalid(
            sourced,
            &element.id,
            "a field carries a contentReference",
        ));
    }
    let name = element
        .short
        .clone()
        .ok_or_else(|| invalid(sourced, &element.id, "no short name"))?;
    let cardinality = cardinality(sourced, element)?;
    let extensions = element_extensions(sourced, element, true)?;
    let data_type = data_type(corpus, sourced, element, extensions.optionality)?;
    let table = table(sourced, element)?;
    Ok(Field {
        id: element.id.clone(),
        position,
        name,
        data_type,
        cardinality,
        optionality: extensions.optionality,
        length: extensions.length,
        conformance_length: extensions.conformance_length,
        table,
        standards_status: extensions.standards_status,
    })
}
