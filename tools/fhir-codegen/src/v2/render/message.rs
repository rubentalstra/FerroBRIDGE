// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Message definitions.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::v2::lower::{Message, MessageStatus};
use crate::v2::render::RenderError;

pub(super) fn render_message(
    banner: &str,
    name: &str,
    message: &Message,
    structures: &BTreeMap<String, (String, String)>,
) -> Result<String, RenderError> {
    let structure = match &message.structure {
        None => String::from("None"),
        Some(id) => {
            let Some((module, static_name)) = structures.get(id) else {
                return Err(RenderError::MissingStructure {
                    message: message.id.clone(),
                    structure: id.clone(),
                });
            };
            format!("Some(&structure::{module}::{static_name})")
        }
    };
    let status = match message.status {
        MessageStatus::Active => "MessageStatus::Active",
        MessageStatus::Withdrawn => "MessageStatus::Withdrawn",
    };
    let mut out = String::from(banner);
    writeln!(
        out,
        "//! The `{}^{}` message definition.\n",
        message.code, message.event
    )?;
    out.push_str("use crate::model::{Message, MessageStatus};\n");
    if message.structure.is_some() {
        out.push_str("use crate::structure;\n");
    }
    writeln!(
        out,
        "\n/// The `{}` message definition, `{}`.\npub static {name}: Message = Message {{\n    id: {:?},\n    url: {:?},\n    code: {:?},\n    event: {:?},\n    structure: {structure},\n    status: {status},\n}};",
        message.id, message.url, message.id, message.url, message.code, message.event
    )?;
    Ok(out)
}
