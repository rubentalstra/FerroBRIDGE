// SPDX-FileCopyrightText: Vernum Projecten B.V.
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

pub mod matching;
pub mod paths;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use openehr_rm::v1_2::paths::RmPath;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::flat::webtemplate::model::WebTemplateNode;

use crate::index::matching::matches_ancestor;
use crate::index::matching::matches_node;
use crate::index::matching::parse_aql_path;
use crate::index::paths::AqlPath;
use crate::index::paths::FlatId;
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

/// One node of a template, as the mapping engines need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedNode {
    /// The `aqlPath` of the node.
    aql_path: AqlPath,
    /// The same path, parsed.
    rm_path: RmPath,
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

    /// Returns the `aqlPath` of the node as a parsed openEHR RM path.
    ///
    /// The index parses it once while it is built, so a caller that needs the
    /// path structurally never re-parses it and never has a parse failure to
    /// handle. The root node carries the empty `aqlPath`, which reads as the
    /// absolute path of the composition (Simplified Formats, §Web Template
    /// Metadata).
    #[must_use]
    pub const fn rm_path(&self) -> &RmPath {
        &self.rm_path
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
            let path = parse_aql_path(&node.aql_path)?;
            entries.push(Entry {
                node: resolved_node(
                    node,
                    generation,
                    FlatId::new(flat_id),
                    children,
                    path.clone(),
                )?,
                id: node.id.clone(),
                path,
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
        let matched = self.all_at_rm_path(path);
        match *matched.as_slice() {
            [only] => Ok(only),
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

    /// Returns every node an already-resolved openEHR path names.
    ///
    /// [`WebTemplateIndex::at_rm_path`] narrows this to one node and refuses
    /// both the empty and the repeated case; a caller that has to name the
    /// candidates of an ambiguity in its own diagnostic reads them here.
    #[must_use]
    pub fn all_at_rm_path(&self, path: &RmPath) -> Vec<&ResolvedNode> {
        self.entries
            .iter()
            .filter(|entry| matches_node(path, &entry.path, entry.name.as_deref()))
            .map(|entry| &entry.node)
            .collect()
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
    rm_path: RmPath,
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
        rm_path,
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
