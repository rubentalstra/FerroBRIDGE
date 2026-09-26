// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Structure trees, owned or linked to an earlier static.

use std::collections::BTreeSet;
use std::fmt::Write;

use crate::v2::legacy::lower::Owner;
use crate::v2::lower::{GroupKind, Node, SegmentStatus, Structure};
use crate::v2::render::RenderError;
use crate::v2::render::names;
use crate::v2::render::segment::cardinality;
use crate::v2::render::segment::uses_line;
use crate::v2::render::version_module;

/// Resolves a segment id to the path of its `static`, from a structure module.
type SegmentPath<'a> = &'a dyn Fn(&str) -> Option<String>;

struct TreeWriter<'a> {
    structure: &'a str,
    segments: SegmentPath<'a>,
    uses: BTreeSet<&'static str>,
}

impl TreeWriter<'_> {
    fn nodes(&mut self, nodes: &[Node], out: &mut String) -> Result<(), RenderError> {
        out.push_str("&[");
        for node in nodes {
            self.node(node, out)?;
            out.push(',');
        }
        out.push(']');
        Ok(())
    }

    fn node(&mut self, node: &Node, out: &mut String) -> Result<(), RenderError> {
        self.uses.extend(["Cardinality", "Max", "Node"]);
        match node {
            Node::Segment {
                id,
                position,
                segment,
                cardinality: count,
                status,
            } => {
                self.uses.insert("SegmentRef");
                let Some(path) = (self.segments)(segment) else {
                    return Err(RenderError::MissingSegment {
                        structure: self.structure.to_owned(),
                        segment: segment.clone(),
                    });
                };
                let status = match status {
                    None => "None",
                    Some(SegmentStatus::A) => "Some(SegmentStatus::A)",
                    Some(SegmentStatus::B) => "Some(SegmentStatus::B)",
                    Some(SegmentStatus::D) => "Some(SegmentStatus::D)",
                };
                if status != "None" {
                    self.uses.insert("SegmentStatus");
                }
                write!(
                    out,
                    "Node::Segment(SegmentRef {{ id: {id:?}, position: {position}, segment: &{path}, cardinality: {}, status: {status} }})",
                    cardinality(*count)
                )?;
            }
            Node::Group {
                id,
                position,
                name,
                cardinality: count,
                kind,
                children,
            } => {
                self.uses.insert("Group");
                self.uses.insert("GroupKind");
                let kind = match kind {
                    GroupKind::Sequence => "GroupKind::Sequence",
                    GroupKind::Choice => "GroupKind::Choice",
                };
                write!(
                    out,
                    "Node::Group(Group {{ id: {id:?}, position: {position}, name: {name:?}, cardinality: {}, kind: {kind}, children: ",
                    cardinality(*count)
                )?;
                self.nodes(children, out)?;
                out.push_str(" })");
            }
            Node::Placeholder {
                id,
                position,
                cardinality: count,
            } => {
                self.uses.insert("Placeholder");
                write!(
                    out,
                    "Node::Placeholder(Placeholder {{ id: {id:?}, position: {position}, cardinality: {} }})",
                    cardinality(*count)
                )?;
            }
        }
        Ok(())
    }
}

pub(super) fn render_structure(
    banner: &str,
    name: &str,
    structure: &Structure,
    segments: SegmentPath<'_>,
    import_segment: bool,
) -> Result<String, RenderError> {
    let mut writer = TreeWriter {
        structure: &structure.id,
        segments,
        uses: ["Node", "Structure"].into(),
    };
    let mut tree = String::from("[");
    for node in &structure.nodes {
        writer.node(node, &mut tree)?;
        tree.push(',');
    }
    tree.push(']');
    let mut out = String::from(banner);
    let heading = structure_heading(structure);
    writeln!(out, "//! {heading}.\n")?;
    out.push_str(&uses_line(&writer.uses));
    if import_segment && writer.uses.contains("SegmentRef") {
        out.push_str("use crate::segment;\n");
    }
    let (doc, url) = match &structure.url {
        Some(url) => (
            format!(
                "The `{}` message structure definition, `{url}`.",
                structure.id
            ),
            format!("Some({url:?})"),
        ),
        None => (format!("{heading}."), String::from("None")),
    };
    let withdrawn = structure.withdrawn_as_of.as_ref().map_or_else(
        || String::from("None"),
        |version| format!("Some({version:?})"),
    );
    writeln!(
        out,
        "\n/// The top-level nodes of [`{name}`], one `static` so a structure with the same tree links to it.\npub static {name}_NODES: [Node; {}] = {tree};\n\n/// {doc}\npub static {name}: Structure = Structure {{\n    id: {:?},\n    url: {url},\n    version: {:?},\n    withdrawn_as_of: {withdrawn},\n    nodes: &{name}_NODES,\n}};",
        structure.nodes.len(),
        structure.id,
        structure.version
    )?;
    Ok(out)
}

/// The heading of a structure's module and `static`.
fn structure_heading(structure: &Structure) -> String {
    match (&structure.withdrawn_as_of, &structure.url) {
        (Some(withdrawn), _) => format!(
            "The `{}` message structure of the {} tables, withdrawn as of {withdrawn}",
            structure.id, structure.version
        ),
        (None, None) => format!(
            "The `{}` message structure of the {} tables",
            structure.id, structure.version
        ),
        (None, Some(_)) => format!("The `{}` message structure", structure.id),
    }
}

/// A structure of a version whose tree is identical to the tree `owner`
/// holds, and links to it.
pub(super) fn render_linked_structure(
    banner: &str,
    name: &str,
    structure: &Structure,
    owner: &Owner,
) -> Result<String, RenderError> {
    let (file, _) = names(&structure.id);
    let (owner, from) = match owner {
        Owner::Current => (
            format!("crate::structure::{file}::{name}"),
            String::from("v2.9.1 definitions"),
        ),
        Owner::Version(version) => (
            format!(
                "crate::legacy::{}::structure::{file}::{name}",
                version_module(version)
            ),
            format!("{version} tables"),
        ),
    };
    let heading = structure_heading(structure);
    let withdrawn = structure.withdrawn_as_of.as_ref().map_or_else(
        || String::from("None"),
        |version| format!("Some({version:?})"),
    );
    let mut out = String::from(banner);
    writeln!(
        out,
        "//! {heading}.\n//!\n//! The tables give it the tree of the {from}, which it links to.\n\nuse crate::model::Structure;\n\n/// {heading}.\npub static {name}: Structure = Structure {{\n    id: {:?},\n    url: None,\n    version: {:?},\n    withdrawn_as_of: {withdrawn},\n    nodes: &{owner}_NODES,\n}};",
        structure.id, structure.version
    )?;
    Ok(out)
}
