// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Data types and their components.

use crate::roots::{V2_COMPLEX_BASE, V2_PRIMITIVE_BASE};
use crate::v2::corpus::Corpus;
use crate::v2::definition::Element;
use crate::v2::lower::Component;
use crate::v2::lower::DataType;
use crate::v2::lower::Input;
use crate::v2::lower::LowerError;
use crate::v2::lower::Optionality;
use crate::v2::lower::V2_CANONICAL;
use crate::v2::lower::extension::element_extensions;
use crate::v2::lower::invalid;
use crate::v2::lower::segment::cardinality;
use crate::v2::lower::segment::elements;
use crate::v2::lower::segment::position;
use crate::v2::lower::segment::single_type;
use crate::v2::lower::segment::table;

pub(super) fn lower_data_type(
    corpus: &Corpus,
    sourced: &Input<'_>,
) -> Result<DataType, LowerError> {
    let definition = sourced.definition;
    let id = definition.id.as_str();
    let primitive = definition.base_definition.as_deref() == Some(V2_PRIMITIVE_BASE);
    let base = if primitive {
        V2_PRIMITIVE_BASE
    } else {
        V2_COMPLEX_BASE
    };
    let elements = elements(sourced, base)?;
    if definition.type_name != definition.url {
        return Err(invalid(
            sourced,
            id,
            format!("type {} is not the canonical URL", definition.type_name),
        ));
    }
    let Some((root, components)) = elements.split_first() else {
        return Err(invalid(sourced, id, "the differential has no elements"));
    };
    if root.types.is_some() || root.binding.is_some() || !root.extension.is_empty() {
        return Err(invalid(
            sourced,
            &root.id,
            "the root element carries a type, a binding or an extension",
        ));
    }
    let name = root
        .short
        .clone()
        .ok_or_else(|| invalid(sourced, &root.id, "the root element has no short name"))?;
    match (primitive, components.is_empty()) {
        (true, false) => {
            return Err(invalid(
                sourced,
                id,
                "a primitive data type with components",
            ));
        }
        (false, true) => {
            return Err(invalid(
                sourced,
                id,
                "a complex data type with no components",
            ));
        }
        _ => {}
    }
    let mut lowered = Vec::with_capacity(components.len());
    for (index, element) in components.iter().enumerate() {
        lowered.push(lower_component(corpus, sourced, element, index)?);
    }
    Ok(DataType {
        code: id.to_owned(),
        url: definition.url.clone(),
        name,
        components: lowered,
    })
}

fn lower_component(
    corpus: &Corpus,
    sourced: &Input<'_>,
    element: &Element,
    index: usize,
) -> Result<Component, LowerError> {
    let data_type = sourced.definition.id.as_str();
    let digits = element
        .id
        .strip_prefix(data_type)
        .and_then(|rest| rest.strip_prefix('.'))
        .ok_or_else(|| {
            invalid(
                sourced,
                &element.id,
                format!("not a component of {data_type}"),
            )
        })?;
    let position = position(sourced, &element.id, digits)?;
    if usize::from(position) != index + 1 {
        return Err(invalid(
            sourced,
            &element.id,
            format!("component {} out of order", index + 1),
        ));
    }
    if element.content_reference.is_some() {
        return Err(invalid(
            sourced,
            &element.id,
            "a component carries a contentReference",
        ));
    }
    let name = element
        .short
        .clone()
        .ok_or_else(|| invalid(sourced, &element.id, "no short name"))?;
    let extensions = element_extensions(sourced, element, false)?;
    let withdrawn = extensions.optionality == Optionality::W;
    let code = match (single_type(sourced, element)?, withdrawn) {
        (None, true) => None,
        (None, false) => {
            return Err(invalid(
                sourced,
                &element.id,
                "an untyped component whose optionality is not W",
            ));
        }
        (Some(_), true) => {
            return Err(invalid(
                sourced,
                &element.id,
                "a typed component whose optionality is W",
            ));
        }
        (Some(url), false) => {
            let Some(code) = url.strip_prefix(V2_CANONICAL) else {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("type {url} is not a v2 canonical URL"),
                ));
            };
            if !corpus.data_types().contains_key(url) {
                return Err(invalid(
                    sourced,
                    &element.id,
                    format!("type {url} is no data type definition"),
                ));
            }
            Some(code.to_owned())
        }
    };
    let cardinality = if withdrawn && element.min.is_none() && element.max.is_none() {
        None
    } else {
        Some(cardinality(sourced, element)?)
    };
    let table = table(sourced, element)?;
    Ok(Component {
        id: element.id.clone(),
        position,
        name,
        data_type: code,
        cardinality,
        optionality: extensions.optionality,
        length: extensions.length,
        conformance_length: extensions.conformance_length,
        table,
    })
}
