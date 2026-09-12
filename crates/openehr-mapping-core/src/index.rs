// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The `aqlPath` index over a built Web Template.
//!
//! A mapping path alone is not executable: the leaf RM type, the occurrence
//! bounds and the template node identifier come from the Web Template. This
//! module builds one index over every node of a template and resolves a
//! [`MappingPath`] against it, so both mapping engines share one answer to
//! "what is at this path".
//!
//! Two rules of the match are openEHR's. A positional predicate is 1-based
//! (openEHR BASE Release 1.2.0 §Paths and Locators), and it selects an
//! instance rather than a template node, so it never changes which node a path
//! resolves to. A node-id predicate is compared against the template's node
//! id, which the Simplified Formats specification carries in its interface
//! form for an archetype root.

use std::collections::BTreeMap;

use openehr_its::flat::webtemplate::model::WebTemplate;
use openehr_its::flat::webtemplate::model::WebTemplateNode;
use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::RmPath;

use crate::path::MappingPath;
use crate::template::Binding;
use crate::template::BindingKind;
use crate::template::ConstraintBinding;
use crate::template::Generation;
use crate::template::PathError;
use crate::template::TemplateSource;
use crate::template::is_id_code;
use crate::template::web_template;

/// The upper occurrence bound the Web Template writes for an unbounded node
/// (Simplified Formats, §Web Template Metadata).
const UNBOUNDED: i32 = -1;

/// The `aqlPath` of one node of a Web Template.
///
/// The root of a template carries the empty path, and every other node carries
/// an absolute openEHR path from the root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AqlPath(String);

impl AqlPath {
    /// Wraps the `aqlPath` of a Web Template node.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// Returns the path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for AqlPath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The identifier of one node in the simplified formats.
///
/// This is the `/`-joined chain of node ids from the root of the template, the
/// prefix every FLAT key of that node is built from (Simplified Formats,
/// §Field Identifiers). It is unique within a template, which the `aqlPath` is
/// not: the alternatives of a polymorphic `ELEMENT` share one `aqlPath`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlatId(String);

impl FlatId {
    /// Wraps a flat node identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for FlatId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One node of a template, as the mapping engines need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNode {
    /// The `aqlPath` of the node.
    aql_path: AqlPath,
    /// The RM type the template constrains the node to.
    rm_type: String,
    /// The archetype node id, absent where the template constrains no node
    /// identity.
    node_id: Option<String>,
    /// The lower occurrence bound.
    min: Option<u32>,
    /// The upper occurrence bound, absent when the node is unbounded.
    max: Option<u32>,
    /// The identifier of the node in the simplified formats.
    flat_id: FlatId,
    /// The generation of the template the node came from.
    generation: Generation,
    /// The children of the node, in template order.
    children: Vec<FlatId>,
}

impl ResolvedNode {
    /// Returns the `aqlPath` of the node.
    #[must_use]
    pub const fn aql_path(&self) -> &AqlPath {
        &self.aql_path
    }

    /// Returns the RM type the template constrains the node to.
    #[must_use]
    pub fn rm_type(&self) -> &str {
        &self.rm_type
    }

    /// Returns the archetype node id, when the template constrains one.
    #[must_use]
    pub fn node_id(&self) -> Option<&str> {
        self.node_id.as_deref()
    }

    /// Returns the lower occurrence bound, when the template carries one.
    #[must_use]
    pub const fn min(&self) -> Option<u32> {
        self.min
    }

    /// Returns the upper occurrence bound, or `None` when it is unbounded.
    #[must_use]
    pub const fn max(&self) -> Option<u32> {
        self.max
    }

    /// Returns the identifier of the node in the simplified formats.
    #[must_use]
    pub const fn flat_id(&self) -> &FlatId {
        &self.flat_id
    }

    /// Returns the generation of the template the node came from.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Returns the children of the node, in template order.
    #[must_use]
    pub fn children(&self) -> &[FlatId] {
        &self.children
    }

    /// Whether the node may occur more than once.
    ///
    /// This is what decides whether a FLAT key of the node carries an instance
    /// index (Simplified Formats, §Instance Indexing).
    #[must_use]
    pub const fn repeats(&self) -> bool {
        match self.max {
            None => true,
            Some(max) => max > 1,
        }
    }
}

/// A path from one node of a template to a node under it.
///
/// The mapping languages write a child mapping's path relative to its parent,
/// so the engines need the path between two resolved nodes. It carries no
/// leading `/`, because openEHR BASE Release 1.2.0 §Paths and Locators marks
/// an absolute path with one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelativePath(RmPath);

impl RelativePath {
    /// Returns the path as an openEHR RM path.
    #[must_use]
    pub const fn as_rm_path(&self) -> &RmPath {
        &self.0
    }

    /// Whether the path has no segments, which is the path of a node to
    /// itself.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.segments.is_empty()
    }
}

impl core::fmt::Display for RelativePath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Returns the path from `parent` down to `child`.
///
/// The result is `child`'s `aqlPath` with `parent`'s prefix removed, so
/// appending it to `parent`'s path reproduces `child`'s.
///
/// # Errors
///
/// Returns [`PathError::NotADescendant`] when `child` is not under `parent`,
/// and [`PathError::MalformedAqlPath`] when either node carries an `aqlPath`
/// the openEHR path grammar refuses.
pub fn relative(parent: &ResolvedNode, child: &ResolvedNode) -> Result<RelativePath, PathError> {
    let parent_path = parse_aql_path(parent.aql_path.as_str())?;
    let child_path = parse_aql_path(child.aql_path.as_str())?;
    let Some(tail) = child_path.segments.get(parent_path.segments.len()..) else {
        return Err(not_a_descendant(parent, child));
    };
    let head = child_path
        .segments
        .iter()
        .take(parent_path.segments.len())
        .cloned()
        .collect::<Vec<PathSegment>>();
    if head != parent_path.segments {
        return Err(not_a_descendant(parent, child));
    }
    Ok(RelativePath(RmPath {
        absolute: false,
        segments: tail.to_vec(),
    }))
}

/// Returns the refusal of a child that is not under the parent.
fn not_a_descendant(parent: &ResolvedNode, child: &ResolvedNode) -> PathError {
    PathError::NotADescendant {
        parent: parent.aql_path.as_str().to_owned(),
        child: child.aql_path.as_str().to_owned(),
    }
}

/// One node of the index, with what the lookups need beside it.
#[derive(Debug)]
pub(crate) struct Entry {
    /// The node as a consumer sees it.
    node: ResolvedNode,
    /// The node id of this node alone, the last segment of its flat id.
    id: String,
    /// The parsed `aqlPath` of the node.
    path: RmPath,
    /// The index of the parent node, absent for the root.
    parent: Option<usize>,
    /// The runtime name the template fixes for the node, when it fixes one.
    name: Option<String>,
    /// The external terminology bindings of the node.
    bindings: Vec<Binding>,
    /// The constraint bindings of the node.
    constraint_bindings: Vec<ConstraintBinding>,
}

impl Entry {
    /// Returns the node as a consumer sees it.
    pub(crate) const fn node(&self) -> &ResolvedNode {
        &self.node
    }

    /// Returns the node id of this node alone.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// Returns the parsed `aqlPath` of the node.
    pub(crate) const fn path(&self) -> &RmPath {
        &self.path
    }
}

/// Every node of one template, indexed by `aqlPath` and by flat id.
#[derive(Debug)]
pub struct WebTemplateIndex {
    /// The template the index was built from.
    web_template: WebTemplate,
    /// The generation the template was served in.
    generation: Generation,
    /// The nodes, in template order, the root first.
    entries: Vec<Entry>,
    /// The root of the template.
    root: ResolvedNode,
    /// The nodes carrying each `aqlPath`.
    by_aql_path: BTreeMap<AqlPath, Vec<usize>>,
    /// The node carrying each flat id.
    by_flat_id: BTreeMap<FlatId, usize>,
}

impl WebTemplateIndex {
    /// Builds the Web Template of `source` and indexes every node of it.
    ///
    /// The node-id code space is checked here, before any path resolves: a
    /// template that identifies its nodes with ADL 2 id-codes is refused,
    /// because mapping paths are written with at-codes and the ADL 2 builder
    /// copies node ids verbatim. No specification governs the reconciliation
    /// of the two code spaces: this is FerroBRIDGE's own rule.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::TemplateBuild`] when the template does not build,
    /// [`PathError::IdCodedTemplate`] when it is id-coded, and
    /// [`PathError::MalformedOccurrences`] or [`PathError::MalformedAqlPath`]
    /// when a built node carries occurrences or an `aqlPath` this crate cannot
    /// read.
    pub fn build(source: &TemplateSource) -> Result<Self, PathError> {
        let web_template = web_template(source)?;
        Self::over(web_template, source.generation())
    }

    /// Indexes an already-built Web Template of the given generation.
    ///
    /// # Errors
    ///
    /// The errors of [`WebTemplateIndex::build`], less the build itself.
    pub fn over(web_template: WebTemplate, generation: Generation) -> Result<Self, PathError> {
        let mut entries: Vec<Entry> = Vec::new();
        let mut stack: Vec<(&WebTemplateNode, Option<usize>, String)> =
            vec![(&web_template.tree, None, web_template.tree.id.clone())];
        while let Some((node, parent, flat_id)) = stack.pop() {
            let index = entries.len();
            if let Some(node_id) = node.node_id.as_deref()
                && parent.is_some()
                && is_id_code(node_id)
            {
                return Err(PathError::IdCodedTemplate {
                    template_id: web_template.template_id.clone(),
                    example_node_id: node_id.to_owned(),
                });
            }
            let children: Vec<FlatId> = node
                .children
                .iter()
                .map(|child| FlatId::new(format!("{flat_id}/{}", child.id)))
                .collect();
            for (child, child_flat_id) in node.children.iter().zip(children.iter()).rev() {
                stack.push((child, Some(index), child_flat_id.as_str().to_owned()));
            }
            entries.push(Entry {
                node: resolved_node(node, generation, FlatId::new(flat_id), children)?,
                id: node.id.clone(),
                path: parse_aql_path(&node.aql_path)?,
                parent,
                name: node.name.clone(),
                bindings: bindings_of(node, generation),
                constraint_bindings: constraint_bindings_of(node, generation),
            });
        }

        let mut by_aql_path: BTreeMap<AqlPath, Vec<usize>> = BTreeMap::new();
        let mut by_flat_id: BTreeMap<FlatId, usize> = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            by_aql_path
                .entry(entry.node.aql_path.clone())
                .or_default()
                .push(index);
            by_flat_id
                .entry(entry.node.flat_id.clone())
                .or_insert(index);
        }
        let root = entries
            .first()
            .map(|entry| entry.node.clone())
            .ok_or_else(|| PathError::MalformedAqlPath {
                aql_path: web_template.tree.aql_path.clone(),
                source: openehr_rm::v1_2::paths::PathError::Empty,
            })?;
        Ok(Self {
            web_template,
            generation,
            entries,
            root,
            by_aql_path,
            by_flat_id,
        })
    }

    /// Returns the template the index was built from.
    #[must_use]
    pub const fn web_template(&self) -> &WebTemplate {
        &self.web_template
    }

    /// Returns the template identifier the Web Template carries.
    ///
    /// For an ADL 2 template this is the full HRID, and for an ADL 1.4
    /// template the template id of the OPT.
    #[must_use]
    pub fn template_id(&self) -> &str {
        &self.web_template.template_id
    }

    /// Returns the release version of an ADL 2 template.
    ///
    /// An ADL 1.4 template carries none, so this answers `None` for one.
    #[must_use]
    pub fn sem_ver(&self) -> Option<&str> {
        self.web_template.sem_ver.as_deref()
    }

    /// Returns the generation the template was served in.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Returns the root node of the template.
    #[must_use]
    pub const fn root(&self) -> &ResolvedNode {
        &self.root
    }

    /// Returns every node of the template, in template order.
    pub fn nodes(&self) -> impl Iterator<Item = &ResolvedNode> {
        self.entries.iter().map(|entry| &entry.node)
    }

    /// Returns the node carrying `path`.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::UnknownPath`] when no node carries the path and
    /// [`PathError::AmbiguousPath`] when more than one does.
    pub fn node(&self, path: &AqlPath) -> Result<&ResolvedNode, PathError> {
        let found: Vec<&Entry> = self
            .by_aql_path
            .get(path)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter_map(|&index| self.entries.get(index))
            .collect();
        match *found.as_slice() {
            [only] => Ok(&only.node),
            [] => Err(PathError::UnknownPath {
                template_id: self.template_id().to_owned(),
                path: path.as_str().to_owned(),
                nearest_ancestor: None,
            }),
            _ => Err(PathError::AmbiguousPath {
                template_id: self.template_id().to_owned(),
                path: path.as_str().to_owned(),
                count: found.len(),
            }),
        }
    }

    /// Returns the node carrying `flat_id`.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::UnknownNode`] when the template has no such node.
    pub fn node_by_flat_id(&self, flat_id: &FlatId) -> Result<&ResolvedNode, PathError> {
        self.entry(flat_id).map(|entry| &entry.node)
    }

    /// Resolves a mapping path against `anchor` and returns the node it names.
    ///
    /// The `../` steps of the path are resolved against the anchor first
    /// ([`MappingPath::resolve`]), then the openEHR path that remains is
    /// matched against the template. A node-id predicate matches by exact
    /// at-code or by archetype id in interface form; a positional predicate
    /// selects an instance and never a node, and instance selection travels as
    /// structured occurrences ([`crate::composition`]), so a position inside a
    /// mapping path is refused rather than dropped; a `name/value` predicate
    /// refuses a node whose template-fixed name differs and is accepted on a
    /// node that fixes none.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::Anchor`] when the path walks above its anchor,
    /// [`PathError::PositionalPredicate`] when a segment carries a position,
    /// [`PathError::UnknownPath`] naming the nearest matching ancestor when no
    /// node matches, and [`PathError::AmbiguousPath`] when more than one does.
    pub fn resolve(&self, path: &MappingPath, anchor: &RmPath) -> Result<&ResolvedNode, PathError> {
        let target = path.resolve(anchor).map_err(|source| PathError::Anchor {
            path: path.to_string(),
            source,
        })?;
        if target
            .segments
            .iter()
            .any(|segment| segment.predicate.position.is_some())
        {
            return Err(PathError::PositionalPredicate {
                path: target.to_string(),
            });
        }
        self.at_rm_path(&target)
    }

    /// Returns the node an already-resolved openEHR path names.
    ///
    /// # Errors
    ///
    /// The path errors of [`WebTemplateIndex::resolve`], less the anchor.
    pub fn at_rm_path(&self, path: &RmPath) -> Result<&ResolvedNode, PathError> {
        let matched: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|entry| matches_node(path, &entry.path, entry.name.as_deref()))
            .collect();
        match *matched.as_slice() {
            [only] => Ok(&only.node),
            [] => Err(PathError::UnknownPath {
                template_id: self.template_id().to_owned(),
                path: path.to_string(),
                nearest_ancestor: self.nearest_ancestor(path),
            }),
            _ => Err(PathError::AmbiguousPath {
                template_id: self.template_id().to_owned(),
                path: path.to_string(),
                count: matched.len(),
            }),
        }
    }

    /// Returns the external terminology bindings of `node`.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::UnknownNode`] when the node belongs to another
    /// template.
    pub fn bindings(&self, node: &ResolvedNode) -> Result<&[Binding], PathError> {
        self.entry(&node.flat_id)
            .map(|entry| entry.bindings.as_slice())
    }

    /// Returns the constraint bindings of `node`.
    ///
    /// An ADL 2 template carries none, because AOM2 keeps value sets in the
    /// archetype terminology rather than in ac-code bindings, so this is
    /// always empty for one.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::UnknownNode`] when the node belongs to another
    /// template.
    pub fn constraint_bindings(
        &self,
        node: &ResolvedNode,
    ) -> Result<&[ConstraintBinding], PathError> {
        self.entry(&node.flat_id)
            .map(|entry| entry.constraint_bindings.as_slice())
    }

    /// Returns the nodes from the root down to `flat_id`, the root first.
    ///
    /// # Errors
    ///
    /// Returns [`PathError::UnknownNode`] when the template has no such node.
    pub(crate) fn chain(&self, flat_id: &FlatId) -> Result<Vec<&Entry>, PathError> {
        let mut current = Some(self.entry(flat_id)?);
        let mut chain: Vec<&Entry> = Vec::new();
        while let Some(entry) = current {
            chain.push(entry);
            current = entry.parent.and_then(|parent| self.entries.get(parent));
        }
        chain.reverse();
        Ok(chain)
    }

    /// Returns the entry `flat_id` names.
    fn entry(&self, flat_id: &FlatId) -> Result<&Entry, PathError> {
        self.by_flat_id
            .get(flat_id)
            .and_then(|&index| self.entries.get(index))
            .ok_or_else(|| self.unknown_node(flat_id))
    }

    /// Returns the refusal of a node this template does not carry.
    fn unknown_node(&self, flat_id: &FlatId) -> PathError {
        PathError::UnknownNode {
            template_id: self.template_id().to_owned(),
            flat_id: flat_id.as_str().to_owned(),
        }
    }

    /// Returns the `aqlPath` of the deepest node `path` does reach.
    ///
    /// The root of a template carries the empty `aqlPath`, so the answer is
    /// the empty string when nothing deeper matches.
    fn nearest_ancestor(&self, path: &RmPath) -> Option<String> {
        self.entries
            .iter()
            .filter(|entry| matches_ancestor(path, &entry.path, entry.name.as_deref()))
            .max_by_key(|entry| entry.path.segments.len())
            .map(|entry| entry.node.aql_path.as_str().to_owned())
    }
}

/// Returns the node, with its occurrences read as RM multiplicities.
fn resolved_node(
    node: &WebTemplateNode,
    generation: Generation,
    flat_id: FlatId,
    children: Vec<FlatId>,
) -> Result<ResolvedNode, PathError> {
    let malformed = || PathError::MalformedOccurrences {
        aql_path: node.aql_path.clone(),
        min: node.min,
        max: node.max,
    };
    let min = match node.min {
        None => None,
        Some(bound) => match u32::try_from(bound) {
            Ok(bound) => Some(bound),
            Err(_refused) => return Err(malformed()),
        },
    };
    let max = if node.max == UNBOUNDED {
        None
    } else {
        match u32::try_from(node.max) {
            Ok(bound) => Some(bound),
            Err(_refused) => return Err(malformed()),
        }
    };
    Ok(ResolvedNode {
        aql_path: AqlPath::new(node.aql_path.clone()),
        rm_type: node.rm_type.clone(),
        node_id: node.node_id.clone().filter(|id| !id.is_empty()),
        min,
        max,
        flat_id,
        generation,
        children,
    })
}

/// Returns the external terminology bindings of one node.
fn bindings_of(node: &WebTemplateNode, generation: Generation) -> Vec<Binding> {
    node.term_bindings
        .iter()
        .map(|(terminology, bound)| {
            Binding::new(terminology, &bound.value, BindingKind::of(generation))
        })
        .collect()
}

/// Returns the constraint bindings of one node.
fn constraint_bindings_of(
    node: &WebTemplateNode,
    generation: Generation,
) -> Vec<ConstraintBinding> {
    match generation {
        Generation::Adl2 => Vec::new(),
        Generation::Adl14 => node
            .constraint_bindings
            .iter()
            .map(|binding| {
                ConstraintBinding::new(
                    &binding.attr,
                    &binding.ac_code,
                    &binding.terminology,
                    &binding.query_uri,
                )
            })
            .collect(),
    }
}

/// Parses one `aqlPath` of a Web Template node.
///
/// The root of a template carries the empty path, which the openEHR path
/// grammar does not spell, so it becomes the path with no segments.
fn parse_aql_path(path: &str) -> Result<RmPath, PathError> {
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
fn matches_node(query: &RmPath, node_path: &RmPath, name: Option<&str>) -> bool {
    query.segments.len() == node_path.segments.len() && matches_prefix(query, node_path, name)
}

/// Whether `query` reaches through the node at `node_path`.
fn matches_ancestor(query: &RmPath, node_path: &RmPath, name: Option<&str>) -> bool {
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
/// in an `aqlPath` predicate and what a mapping file writes.
fn node_id_matches(wanted: &str, carried: &str) -> bool {
    wanted == carried || interface_form(wanted) == interface_form(carried)
}

/// Returns the interface form of an archetype identifier.
///
/// The interface form drops the namespace and every version part below the
/// major: `org.example::openEHR-EHR-EVALUATION.note.v1.2.3` reads
/// `openEHR-EHR-EVALUATION.note.v1`. A local code carries neither, so it is
/// returned unchanged.
fn interface_form(id: &str) -> String {
    let without_namespace = id.rsplit_once("::").map_or(id, |(_, rest)| rest);
    let Some((concept, version)) = without_namespace.rsplit_once(".v") else {
        return without_namespace.to_owned();
    };
    let major = version.split('.').next().unwrap_or(version);
    format!("{concept}.v{major}")
}

#[cfg(test)]
mod tests {
    use openehr_rm::v1_2::paths::RmPath;

    use super::interface_form;
    use super::matches_node;
    use super::node_id_matches;
    use super::parse_aql_path;

    fn path(rendered: &str) -> RmPath {
        parse_aql_path(rendered).expect("a well-formed path")
    }

    #[test]
    fn an_archetype_id_matches_in_its_interface_form() {
        assert!(node_id_matches(
            "openEHR-EHR-EVALUATION.note.v1.0.0",
            "openEHR-EHR-EVALUATION.note.v1"
        ));
        assert!(node_id_matches(
            "org.example::openEHR-EHR-EVALUATION.note.v1.0.0",
            "openEHR-EHR-EVALUATION.note.v1"
        ));
        assert!(!node_id_matches(
            "openEHR-EHR-EVALUATION.note.v2",
            "openEHR-EHR-EVALUATION.note.v1"
        ));
        assert_eq!(interface_form("at0001"), "at0001");
    }

    #[test]
    fn a_positional_predicate_selects_an_instance_and_not_a_node() {
        // BASE master11-paths, Using Positional Parameters: the position stands
        // alone or joins the node id as a conjunct; it never changes which node
        // a path names, only which instance of it.
        let node = path("/content[openEHR-EHR-EVALUATION.note.v1]/data[at0001]/items[at0002]");
        let alone = path("/content[openEHR-EHR-EVALUATION.note.v1]/data[at0001]/items[2]");
        let joined =
            path("/content[openEHR-EHR-EVALUATION.note.v1]/data[at0001]/items[at0002 and 2]");
        assert!(matches_node(&alone, &node, None));
        assert!(matches_node(&joined, &node, None));
        assert_eq!(
            joined.segments.last().and_then(|s| s.predicate.position),
            Some(2)
        );
    }

    #[test]
    fn a_name_predicate_refuses_a_node_with_another_fixed_name() {
        let node = path("/data[at0001]/items[at0002]");
        let query = path("/data[at0001]/items[at0002 and name/value='systolic']");
        assert!(matches_node(&query, &node, Some("systolic")));
        assert!(!matches_node(&query, &node, Some("diastolic")));
        assert!(matches_node(&query, &node, None));
    }

    #[test]
    fn a_shorter_or_longer_path_is_not_the_node() {
        let node = path("/data[at0001]/items[at0002]");
        assert!(!matches_node(&path("/data[at0001]"), &node, None));
        assert!(!matches_node(
            &path("/data[at0001]/items[at0002]/value"),
            &node,
            None
        ));
    }
}
