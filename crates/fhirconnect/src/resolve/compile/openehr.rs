// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR side of a mapping, located in the operational template.

use core::str::FromStr;
use std::collections::BTreeMap;
use std::path::Path;

use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::metadata::MappingName;
use openehr_mapping_core::index::ResolvedNode;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::index::matching::node_id_matches;
use openehr_mapping_core::path::MappingPath;
use openehr_mapping_core::position::Located;
use openehr_mapping_core::template::PathError;
use openehr_rm::v1_2::model as rm_model;
use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::RmPath;

use crate::model::ast::ModelMappingFile;
use crate::model::ast::With;
use crate::model::ast::keyword::Variable;
use crate::resolve::error::ResolveCode;
use crate::resolve::extensions::diagnostic;
use crate::resolve::program::target::OpenehrTarget;

use crate::resolve::compile::Compiler;
use crate::resolve::compile::Scope;

impl<'a> Compiler<'a> {
    /// Compiles the openEHR side of a `with`.
    pub(super) fn openehr_side(
        &mut self,
        file: &'a ModelMappingFile,
        with: &With,
        scope: &Scope,
        path: &ModelPath,
    ) -> Option<OpenehrTarget> {
        let written = with.openehr.as_ref()?;
        self.openehr_target(
            file.file(),
            file.header().name().value(),
            written,
            scope,
            &path.field("openehr"),
        )
    }

    /// Binds one openEHR path to its anchor and resolves it against the Web
    /// Template.
    pub(super) fn openehr_target(
        &mut self,
        file: &Path,
        owner: &MappingName,
        written: &Located<String>,
        scope: &Scope,
        path: &ModelPath,
    ) -> Option<OpenehrTarget> {
        let parsed = match MappingPath::from_str(written.value()) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    ResolveCode::MalformedOpenehrPath,
                    written.position(),
                    path,
                    error.to_string(),
                ));
                return None;
            }
        };
        let anchor = self.openehr_anchor(file, owner, written, scope, path, &parsed)?;
        let resolved = match parsed.resolve(&anchor) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.diagnostics
                    .push(error.to_diagnostic(file.to_path_buf()));
                return None;
            }
        };
        if let Some(segment) = resolved
            .segments
            .iter()
            .find(|segment| segment.predicate.position.is_some())
        {
            self.diagnostics.push(diagnostic(
                file,
                owner,
                ResolveCode::MalformedOpenehrPath,
                written.position(),
                path,
                format!(
                    "`{resolved}` selects the instance `{}` inside a path, and occurrences are \
                     structured",
                    segment.attribute
                ),
            ));
            return None;
        }
        match self.locate(&resolved) {
            Ok((node, tail, leaf)) => {
                let occurrences = match occurrences(self.template, node) {
                    Ok(axes) => axes,
                    Err(error) => {
                        self.diagnostics.push(diagnostic(
                            file,
                            owner,
                            ResolveCode::UnknownTemplateNode,
                            written.position(),
                            path,
                            format!(
                                "the occurrence axes of `{}` do not resolve: {error}",
                                node.flat_id().as_str()
                            ),
                        ));
                        return None;
                    }
                };
                let target = OpenehrTarget::new(resolved, node.clone(), tail, occurrences);
                Some(match leaf {
                    Some(class) => target.with_leaf_class(class),
                    None => target,
                })
            }
            Err(error) => {
                let (code, message) = match error {
                    LocateError::Unknown(message) => (ResolveCode::UnknownTemplateNode, message),
                    LocateError::Ambiguous(message) => {
                        (ResolveCode::AmbiguousTemplateNode, message)
                    }
                };
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    code,
                    written.position(),
                    path,
                    message,
                ));
                None
            }
        }
    }

    /// Returns the path the variable an openEHR path opens with names.
    ///
    /// The five openEHR-side variables are `$archetype`, `$openehrRoot`,
    /// `$composition` and `$reference`, plus the anchor a path with no
    /// variable takes
    /// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Variables.adoc`).
    fn openehr_anchor(
        &mut self,
        file: &Path,
        owner: &MappingName,
        written: &Located<String>,
        scope: &Scope,
        path: &ModelPath,
        parsed: &MappingPath,
    ) -> Option<RmPath> {
        let Some(name) = parsed
            .variable()
            .map(openehr_mapping_core::path::PathVariable::name)
        else {
            return Some(scope.openehr.clone());
        };
        match Variable::from_str(name) {
            Ok(Variable::Archetype) => Some(scope.archetype.clone()),
            Ok(Variable::OpenehrRoot) => Some(scope.openehr.clone()),
            Ok(Variable::Composition) => Some(RmPath {
                absolute: true,
                segments: Vec::new(),
            }),
            // NOTE: `$reference` "indicates that there is no direct mapping
            // to openEHR" (`basics/Variables.adoc`), so the mapping
            // legitimately has no openEHR side rather than a broken one.
            Ok(Variable::Reference) => None,
            Ok(Variable::Resource | Variable::FhirRoot | Variable::Context) | Err(_) => {
                self.diagnostics.push(diagnostic(
                    file,
                    owner,
                    ResolveCode::UnboundPathVariable,
                    written.position(),
                    path,
                    format!(
                        "`{}` opens an openEHR path with `${name}`, which names no openEHR anchor",
                        written.value()
                    ),
                ));
                None
            }
        }
    }

    /// Returns the deepest template node an openEHR path names, and the
    /// reference-model attributes below it.
    ///
    /// A Web Template carries the archetype roots and the leaves and compacts
    /// what lies between, so three forms resolve: a path that names an indexed
    /// node, a path that names an `ELEMENT` the builder compacted into its
    /// value node, and a path that reaches past the deepest indexed node into
    /// the reference model or through a structure the template compacted away.
    /// A segment the template carries nowhere and that constrains a node
    /// identity is refused. No specification governs this: our own design.
    pub(super) fn locate(
        &self,
        path: &RmPath,
    ) -> Result<(&'a ResolvedNode, RmPath, Option<String>), LocateError> {
        let total = path.segments.len();
        for taken in (0..=total).rev() {
            let head = path.segments.get(..taken).unwrap_or_default();
            let tail = path.segments.get(taken..).unwrap_or_default();
            let prefix = RmPath {
                absolute: path.absolute,
                segments: head.to_vec(),
            };
            let rest = RmPath {
                absolute: false,
                segments: tail.to_vec(),
            };
            let node = match self.template.at_rm_path(&prefix) {
                Ok(node) => node,
                Err(PathError::AmbiguousPath { .. }) => {
                    return Err(self.ambiguity(&prefix, &self.template.all_at_rm_path(&prefix)));
                }
                Err(_) => {
                    let found: Vec<&ResolvedNode> = self
                        .compacted
                        .iter()
                        .filter(|entry| names_same_place(&prefix, &entry.0))
                        .map(|&(_, node)| node)
                        .collect();
                    match *found.as_slice() {
                        [only] => only,
                        [] => continue,
                        _ => return Err(self.ambiguity(&prefix, &found)),
                    }
                }
            };
            let constrained = rest.segments.iter().find(|segment| constrains(segment));
            if let Some(segment) = constrained
                && !self
                    .interior
                    .iter()
                    .any(|interior| names_same_place(path, interior))
            {
                return Err(LocateError::Unknown(format!(
                    "the template `{}` has no node at `{path}`; the deepest node it reaches is \
                     `{}` and `{}` below it names a node identity the template does not carry",
                    self.template.template_id(),
                    node.aql_path().as_str(),
                    segment.attribute
                )));
            }
            let leaf = rm_tail(node.rm_type(), &rest).map_err(|message| {
                LocateError::Unknown(format!(
                    "the template `{}` reaches `{}` and `{path}` walks `{rest}` below it: \
                     {message}",
                    self.template.template_id(),
                    node.aql_path().as_str()
                ))
            })?;
            return Ok((node, rest, leaf));
        }
        Err(LocateError::Unknown(format!(
            "the template `{}` has no node at `{path}`",
            self.template.template_id()
        )))
    }

    /// Builds the refusal for a path that names more than one template node.
    fn ambiguity(&self, path: &RmPath, candidates: &[&ResolvedNode]) -> LocateError {
        let named: Vec<&str> = candidates
            .iter()
            .map(|node| node.flat_id().as_str())
            .collect();
        LocateError::Ambiguous(format!(
            "`{path}` names {} nodes of the template `{}`: {}",
            named.len(),
            self.template.template_id(),
            named.join(", ")
        ))
    }
}

/// Why an openEHR path names no single node of the operational template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LocateError {
    /// No node of the template carries the path.
    Unknown(String),
    /// More than one node carries it, so nothing says which one is meant.
    Ambiguous(String),
}

/// Whether `candidate` walks below `ancestor` in either path syntax.
pub(super) fn walks_below(candidate: &str, ancestor: &str) -> bool {
    candidate
        .strip_prefix(ancestor)
        .is_some_and(|tail| tail.starts_with(['.', '/', '[']))
}

/// Checks the reference-model attributes an openEHR path walks below the
/// deepest template node it reaches.
///
/// A Web Template carries the nodes an archetype constrains and stops there, so
/// everything below the deepest one is plain reference model. The attribute
/// model `openehr-rm` generates from the RM BMM is the oracle for it
/// (<https://docs.rs/openehr-rm/0.0.69/openehr_rm/v1_2/model/fn.attribute.html>):
/// an attribute is looked up on the node's type and on the concrete subtypes of
/// it, because a declared type may be abstract (`ELEMENT.value` is
/// `DATA_VALUE`) and the attribute then belongs to one of its descendants.
///
/// A type the model does not carry stops the walk rather than refusing it: the
/// mapping is then checked as far as the model reaches and no further.
///
/// Returns the class of the last attribute the tail names, `None` for an
/// empty tail and for a walk the model stopped before its end. The class is
/// the declared type, or its one concrete descendant when the declared type is
/// abstract and has exactly one.
fn rm_tail(rm_type: &str, tail: &RmPath) -> Result<Option<String>, String> {
    let mut candidates = concrete_forms(rm_type);
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut leaf: Option<&'static str> = None;
    for (position, segment) in tail.segments.iter().enumerate() {
        let attribute = segment.attribute.as_str();
        let mut declared: Option<&'static str> = None;
        let mut next: Vec<&'static str> = Vec::new();
        for candidate in &candidates {
            let Some(found) = rm_model::attribute(candidate, attribute) else {
                continue;
            };
            declared.get_or_insert(found.declared_type);
            for form in concrete_forms(found.declared_type) {
                if !next.contains(&form) {
                    next.push(form);
                }
            }
        }
        let Some(declared) = declared else {
            return Err(format!(
                "`{attribute}` is no attribute of `{}` in the openEHR reference model",
                candidates.join("`, `")
            ));
        };
        leaf = Some(declared);
        if next.is_empty() {
            let last = position.saturating_add(1) == tail.segments.len();
            return Ok(last.then(|| String::from(declared)));
        }
        candidates = next;
    }
    Ok(leaf.map(concrete_leaf))
}

/// Returns the one concrete form of an abstract class, or the class itself.
fn concrete_leaf(declared: &'static str) -> String {
    let Some(class) = rm_model::class(declared) else {
        return String::from(declared);
    };
    if !class.is_abstract {
        return String::from(declared);
    }
    let concrete: Vec<&str> = class
        .descendants
        .iter()
        .copied()
        .filter(|name| rm_model::class(name).is_some_and(|found| !found.is_abstract))
        .collect();
    match *concrete.as_slice() {
        [only] => String::from(only),
        _ => String::from(declared),
    }
}

/// Returns the reference-model class plus every concrete class below it.
///
/// An empty result means the reference model carries no class of that name,
/// which is what stops [`rm_tail`] rather than refusing.
fn concrete_forms(rm_type: &str) -> Vec<&'static str> {
    let Some(class) = rm_model::class(rm_type) else {
        return Vec::new();
    };
    let mut forms: Vec<&'static str> = vec![class.name];
    for descendant in class.descendants {
        if !forms.contains(descendant) {
            forms.push(descendant);
        }
    }
    forms
}

/// Whether a path segment constrains which node it selects.
fn constrains(segment: &PathSegment) -> bool {
    segment.predicate.archetype_node_id.is_some()
        || segment.predicate.name_value.is_some()
        || segment.predicate.position.is_some()
}

/// Returns the repeating nodes from the root down to `node`, outermost first.
///
/// # Errors
///
/// Returns the refusal the index raised for a prefix that is not simply
/// absent, so a lost occurrence axis is a diagnostic rather than a node that
/// silently stops repeating.
pub(super) fn occurrences(
    template: &WebTemplateIndex,
    node: &ResolvedNode,
) -> Result<Vec<openehr_mapping_core::index::paths::FlatId>, PathError> {
    let mut axes = Vec::new();
    let mut prefix = String::new();
    for segment in node.flat_id().as_str().split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        let flat_id = openehr_mapping_core::index::paths::FlatId::new(prefix.clone());
        let found = match template.node_by_flat_id(&flat_id) {
            Ok(found) => found,
            // NOTE: a flat id is built one level at a time and the index
            // carries a node only where the builder kept one, so an unknown
            // prefix is a level with no node of its own.
            Err(PathError::UnknownNode { .. }) => continue,
            Err(error) => return Err(error),
        };
        if found.repeats() {
            axes.push(flat_id);
        }
    }
    Ok(axes)
}

/// Returns every path that lies on the way to a node of the template.
///
/// The builder compacts the structure between an archetype root and a leaf, so
/// a path that names one of those structures is a path the template knows
/// without carrying a node for it.
pub(super) fn interior_paths(template: &WebTemplateIndex) -> Vec<RmPath> {
    let mut found: BTreeMap<String, RmPath> = BTreeMap::new();
    for node in template.nodes() {
        let path = node.rm_path();
        for taken in 1..path.segments.len() {
            let prefix = RmPath {
                absolute: path.absolute,
                segments: path.segments.get(..taken).unwrap_or_default().to_vec(),
            };
            found.entry(prefix.to_string()).or_insert(prefix);
        }
    }
    found.into_values().collect()
}

/// Maps the path a mapping writes for a compacted node onto the nodes it
/// reaches.
///
/// A shortened path that reaches more than one node keeps all of them, because
/// binding the mapping to one of them would bind it to a node nothing named.
pub(super) fn compaction_map(template: &WebTemplateIndex) -> Vec<(RmPath, &ResolvedNode)> {
    let mut found: Vec<(RmPath, &ResolvedNode)> = Vec::new();
    for node in template.nodes() {
        let path = node.rm_path();
        let kept = path
            .segments
            .iter()
            .rposition(constrains)
            .map_or(0, |last| last.saturating_add(1));
        if kept == path.segments.len() {
            continue;
        }
        let shortened = RmPath {
            absolute: path.absolute,
            segments: path.segments.get(..kept).unwrap_or_default().to_vec(),
        };
        found.push((shortened, node));
    }
    found
}

/// Whether a path a mapping writes names the place `carried` holds in the
/// template.
///
/// A mapping names an archetype node by its node id, and the template may
/// constrain the same node further with a name, which the `aqlPath` carries.
/// A name the mapping does not write selects nothing, the rule the index
/// applies to an indexed node; a node id the mapping writes must be the one
/// the template carries, and one it does not write matches only a segment
/// that carries none. No specification governs this: our own design.
fn names_same_place(written: &RmPath, carried: &RmPath) -> bool {
    written.absolute == carried.absolute
        && written.segments.len() == carried.segments.len()
        && written
            .segments
            .iter()
            .zip(&carried.segments)
            .all(|(wanted, held)| {
                let node_id = match (
                    wanted.predicate.archetype_node_id.as_deref(),
                    held.predicate.archetype_node_id.as_deref(),
                ) {
                    (Some(wanted), Some(held)) => node_id_matches(wanted, held),
                    (None, None) => true,
                    (Some(_), None) | (None, Some(_)) => false,
                };
                let name = match (
                    wanted.predicate.name_value.as_deref(),
                    held.predicate.name_value.as_deref(),
                ) {
                    (Some(wanted), Some(held)) => wanted == held,
                    (Some(_) | None, None) | (None, Some(_)) => true,
                };
                !wanted.descendant
                    && wanted.attribute == held.attribute
                    && wanted.predicate.position == held.predicate.position
                    && node_id
                    && name
            })
}
