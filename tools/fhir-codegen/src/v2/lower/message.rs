// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Message definitions, each linked to its structure by name.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::fhir::{Derivation, StructureKind};
use crate::roots::V2_MESSAGE_BASE;
use crate::v2::corpus::SourcedMessage;
use crate::v2::definition::MessageElement;
use crate::v2::lower::Defect;
use crate::v2::lower::LowerError;
use crate::v2::lower::Message;
use crate::v2::lower::MessageStatus;
use crate::v2::lower::Structure;
use crate::v2::lower::V2_CANONICAL;
use crate::v2::lower::defect_in;
use crate::v2::lower::invalid_in;

// The element ids a message definition constrains.
const MESSAGE_TYPE: &str = "Message.messageType";

const TRIGGER_EVENT: &str = "Message.triggerEvent";

const MESSAGE_STRUCTURE: &str = "Message.structure";

const MESSAGE_STATUS: &str = "Message.status";

const ACKNOWLEDGEMENTS: [&str; 3] = [
    "Message.acknowledgementChoreography.originalModeResponse",
    "Message.acknowledgementChoreography.enhancedModeImmediateResponse",
    "Message.acknowledgementChoreography.enhancedModeApplicationResponse",
];

/// The profile URL prefix `Message.structure` targets.
const STRUCTURE_PROFILE: &str = "http://hl7.org/fhir/StructureDefinition/MessageStructure/";

pub(super) fn lower_message(
    sourced: &SourcedMessage,
    structures: &BTreeMap<String, Structure>,
    hits: &RefCell<BTreeSet<(Defect, String)>>,
) -> Result<Message, LowerError> {
    let file = sourced.file.as_str();
    let definition = &sourced.definition;
    let id = definition.id.as_str();
    if definition.kind != StructureKind::Logical
        || definition.is_abstract
        || definition.type_name != "Message"
        || definition.derivation != Some(Derivation::Constraint)
        || definition.base_definition.as_deref() != Some(V2_MESSAGE_BASE)
    {
        return Err(invalid_in(
            file,
            id,
            format!("not a concrete logical constraint on {V2_MESSAGE_BASE}"),
        ));
    }
    if definition.url != format!("{V2_CANONICAL}Message/{id}") {
        return Err(invalid_in(
            file,
            id,
            format!("canonical URL {} does not end in its id", definition.url),
        ));
    }
    let Some(differential) = &definition.differential else {
        return Err(invalid_in(file, id, "no differential"));
    };
    let (codes, structure_element) = message_elements(file, &differential.element)?;
    let Some(code) = codes.get(MESSAGE_TYPE).copied() else {
        return Err(invalid_in(file, id, "no Message.messageType"));
    };
    let Some(id_event) = id
        .strip_prefix(code)
        .and_then(|rest| rest.strip_prefix('-'))
        .filter(|event| !event.is_empty())
    else {
        return Err(invalid_in(
            file,
            id,
            format!("the id is not {code}-<event>"),
        ));
    };
    // NOTE: no specification governs this: our own design; message--message.json gives
    // Message.triggerEvent `min` 0, and a definition that states none names its event in its id.
    let event = match codes.get(TRIGGER_EVENT) {
        Some(stated) if *stated != id_event => {
            return Err(invalid_in(
                file,
                TRIGGER_EVENT,
                format!("the trigger event {stated} is not the {id_event} the id names"),
            ));
        }
        _ => id_event,
    };
    let status = match codes.get(MESSAGE_STATUS).copied() {
        Some("active") => MessageStatus::Active,
        Some("withdrawn") => MessageStatus::Withdrawn,
        other => {
            return Err(invalid_in(
                file,
                MESSAGE_STATUS,
                format!("status {other:?}"),
            ));
        }
    };
    let structure = match structure_element {
        None => {
            defect_in(file, hits, id, Defect::MessageWithoutStructure)?;
            None
        }
        Some(element) => Some(message_structure(file, element, structures, hits)?),
    };
    Ok(Message {
        id: id.to_owned(),
        url: definition.url.clone(),
        code: code.to_owned(),
        event: event.to_owned(),
        structure,
        status,
    })
}

/// The fixed codes of a message definition by element id, and its
/// `Message.structure` element, each checked for its shape and given once.
type MessageElements<'a> = (BTreeMap<&'a str, &'a str>, Option<&'a MessageElement>);

fn message_elements<'a>(
    file: &str,
    elements: &'a [MessageElement],
) -> Result<MessageElements<'a>, LowerError> {
    let mut codes: BTreeMap<&str, &str> = BTreeMap::new();
    let mut structure_element = None;
    for element in elements {
        if element.path != element.id {
            return Err(invalid_in(
                file,
                &element.id,
                format!("path {} differs from the id", element.path),
            ));
        }
        let element_id = element.id.as_str();
        let is_code = [MESSAGE_TYPE, TRIGGER_EVENT, MESSAGE_STATUS].contains(&element_id);
        let is_reference =
            element_id == MESSAGE_STRUCTURE || ACKNOWLEDGEMENTS.contains(&element_id);
        if !is_code && !is_reference {
            return Err(invalid_in(
                file,
                element_id,
                "an element the lowering does not know",
            ));
        }
        let shape_ok = if is_code {
            element.pattern_code.is_some() && element.types.is_none()
        } else {
            element.pattern_code.is_none() && element.types.is_some()
        };
        if !shape_ok {
            return Err(invalid_in(
                file,
                element_id,
                "a code element without a patternCode or a reference element without a type",
            ));
        }
        let duplicate = if let Some(code) = &element.pattern_code {
            codes.insert(element_id, code.as_str()).is_some()
        } else if element_id == MESSAGE_STRUCTURE {
            structure_element.replace(element).is_some()
        } else {
            false
        };
        if duplicate {
            return Err(invalid_in(file, element_id, "the id is given twice"));
        }
    }
    Ok((codes, structure_element))
}

/// The id of the structure `Message.structure` names: the one structure whose
/// id, with every `_` written `-`, is the last step of the target profile.
fn message_structure(
    file: &str,
    element: &MessageElement,
    structures: &BTreeMap<String, Structure>,
    hits: &RefCell<BTreeSet<(Defect, String)>>,
) -> Result<String, LowerError> {
    let profile = match element.types.as_deref() {
        Some([only]) if only.code == "Reference" => match only.target_profile.as_slice() {
            [profile] => profile.as_str(),
            _ => {
                return Err(invalid_in(
                    file,
                    &element.id,
                    "not exactly one target profile",
                ));
            }
        },
        _ => {
            return Err(invalid_in(
                file,
                &element.id,
                "not exactly one Reference type",
            ));
        }
    };
    let Some(name) = profile.strip_prefix(STRUCTURE_PROFILE) else {
        return Err(invalid_in(
            file,
            &element.id,
            format!("target profile {profile} names no message structure"),
        ));
    };
    defect_in(file, hits, &element.id, Defect::StructureProfileName)?;
    let mut matches = structures
        .keys()
        .filter(|candidate| candidate.replace('_', "-") == name);
    match (matches.next(), matches.next()) {
        (Some(only), None) => Ok(only.clone()),
        (None, _) => Err(invalid_in(
            file,
            &element.id,
            format!("target profile {profile} matches no message structure by name"),
        )),
        (Some(first), Some(second)) => Err(invalid_in(
            file,
            &element.id,
            format!("target profile {profile} matches both {first} and {second}"),
        )),
    }
}
