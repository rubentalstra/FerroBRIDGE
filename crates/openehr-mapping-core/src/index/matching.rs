// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How a mapping path matches a node of a Web Template.

use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::RmPath;

use crate::template::PathError;

/// Parses one `aqlPath` of a Web Template node.
///
/// The root of a template carries the empty path, which the openEHR path
/// grammar does not spell, so it becomes the path with no segments. That is
/// the one place an empty string is a path: `RmPath::from_str("")` refuses
/// it, so a consumer reads a node's path through [`ResolvedNode::rm_path`]
/// rather than parsing the `aqlPath` string itself.
pub(super) fn parse_aql_path(path: &str) -> Result<RmPath, PathError> {
    if path.is_empty() {
        return Ok(RmPath {
            absolute: true,
            segments: Vec::new(),
        });
    }
    core::str::FromStr::from_str(path).map_err(|source| PathError::MalformedAqlPath {
        aql_path: path.to_owned(),
        source,
    })
}

/// Whether `query` names the node at `node_path`.
pub(super) fn matches_node(query: &RmPath, node_path: &RmPath, name: Option<&str>) -> bool {
    query.segments.len() == node_path.segments.len() && matches_prefix(query, node_path, name)
}

/// Whether `query` reaches through the node at `node_path`.
pub(super) fn matches_ancestor(query: &RmPath, node_path: &RmPath, name: Option<&str>) -> bool {
    query.segments.len() >= node_path.segments.len() && matches_prefix(query, node_path, name)
}

/// Whether the leading segments of `query` match the node at `node_path`.
fn matches_prefix(query: &RmPath, node_path: &RmPath, name: Option<&str>) -> bool {
    let last = node_path.segments.len().saturating_sub(1);
    node_path
        .segments
        .iter()
        .enumerate()
        .all(|(position, node_segment)| {
            query.segments.get(position).is_some_and(|query_segment| {
                let carries_name = position == last;
                matches_segment(
                    query_segment,
                    node_segment,
                    carries_name.then_some(name).flatten(),
                )
            })
        })
}

/// Whether one query segment matches one node segment.
fn matches_segment(query: &PathSegment, node: &PathSegment, name: Option<&str>) -> bool {
    if query.descendant || query.attribute != node.attribute {
        return false;
    }
    if let Some(wanted) = query.predicate.archetype_node_id.as_deref()
        && !node
            .predicate
            .archetype_node_id
            .as_deref()
            .is_some_and(|carried| node_id_matches(wanted, carried))
    {
        return false;
    }
    // NOTE: no specification governs this: our own design, a name predicate
    // over a node whose template fixes no name selects an instance at runtime.
    match (query.predicate.name_value.as_deref(), name) {
        (Some(wanted), Some(carried)) => wanted == carried,
        _ => true,
    }
}

/// Whether a node-id predicate matches a node id the template carries.
///
/// An at-code is compared exactly. An archetype id is compared in its
/// interface form, which is what the Simplified Formats specification carries
/// in an `aqlPath` predicate and what a mapping file writes. A mapping
/// language that names an archetype outside a path compares it the same way,
/// which is why this is public.
#[must_use]
pub fn node_id_matches(wanted: &str, carried: &str) -> bool {
    wanted == carried || interface_form(wanted) == interface_form(carried)
}

/// Returns the release version an archetype identifier carries below its
/// major, `None` when it carries only the interface form.
///
/// An ADL 2 archetype identifier ends in the archetype's own release version
/// (`openEHR-EHR-EVALUATION.note.v1.4.1`), whose major is the `vN` the
/// interface form keeps (openEHR AM Release-2.x, §Archetype Identification,
/// <https://specifications.openehr.org/releases/AM/latest/Overview.html>). A
/// template served over the ADL 1.4 route carries the interface form alone and
/// so states no release version.
// TODO(#241): read the identifier through `ArchetypeHrid` once openehr-am
// offers `FromStr` for it (sibling request S2), as the server CDR module does
// through `openehr_adl::hrid::parse_hrid` today.
#[must_use]
pub fn archetype_release_version(id: &str) -> Option<&str> {
    let without_namespace = id.rsplit_once("::").map_or(id, |(_, rest)| rest);
    let (_, version) = without_namespace.rsplit_once(".v")?;
    version.contains('.').then_some(version)
}

/// Returns the interface form of an archetype identifier.
///
/// The interface form drops the namespace and every version part below the
/// major: `org.example::openEHR-EHR-EVALUATION.note.v1.2.3` reads
/// `openEHR-EHR-EVALUATION.note.v1`. A local code carries neither, so it is
/// returned unchanged.
pub(super) fn interface_form(id: &str) -> String {
    let without_namespace = id.rsplit_once("::").map_or(id, |(_, rest)| rest);
    let Some((concept, version)) = without_namespace.rsplit_once(".v") else {
        return without_namespace.to_owned();
    };
    let major = version.split('.').next().unwrap_or(version);
    format!("{concept}.v{major}")
}
