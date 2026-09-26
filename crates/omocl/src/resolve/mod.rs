// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Compiling a mapping set into one immutable program per template.
//!
//! OMOCL maps by archetype, and a template may hold one archetype at several
//! places, so [`compile`] binds every file at every archetype root of the
//! template that the file maps. An `Include` binds the included archetype's
//! files below the including root at its `base_path`; the same archetype may
//! be included at two base paths, and an archetype that includes itself
//! through any chain is refused (<https://github.com/SevKohler/OMOCL>, the
//! railroad's INCLUDE clause; the cycle rule is our own design). Every path
//! is checked against the Web Template here, once, so the engine walks a
//! composition without parsing anything.

pub mod program;

use core::fmt;
use std::collections::BTreeSet;
use std::sync::Arc;

use openehr_mapping_core::header::archetype::ArchetypeId;
use openehr_mapping_core::index::WebTemplateIndex;
use openehr_mapping_core::index::matching::node_id_matches;
use openehr_mapping_core::path::MappingPath;
use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::RmPath;

use crate::engine::custom::Converters;
use crate::model::ast::Alternative;
use crate::model::ast::Entity;
use crate::model::ast::Factor;
use crate::model::ast::MappingFile;
use crate::model::ast::Record;
use crate::model::load::MappingSet;
use crate::resolve::program::BoundAlternative;
use crate::resolve::program::BoundColumn;
use crate::resolve::program::BoundCustom;
use crate::resolve::program::BoundEntry;
use crate::resolve::program::BoundFactor;
use crate::resolve::program::BoundInclude;
use crate::resolve::program::BoundPath;
use crate::resolve::program::BoundRecord;
use crate::resolve::program::Hop;
use crate::resolve::program::Program;
use crate::resolve::program::Scope;
use crate::resolve::program::Unbound;

/// The self step the library writes as `"."`.
const SELF_STEP: &str = ".";

/// Why a mapping set does not compile against a template.
///
/// Every variant names the file and the entry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// An archetype includes itself through the chain named.
    #[error(
        "the Include chain {} returns to an archetype already on it",
        render_chain(chain)
    )]
    IncludeCycle {
        /// The archetypes from the outermost to the repeated one.
        chain: Vec<ArchetypeId>,
    },
    /// A path opens with a `$` variable, which OMOCL does not define.
    #[error("{mapping}#{entry}: `{path}` opens with a variable, which OMOCL does not define")]
    PathVariable {
        /// The file.
        mapping: String,
        /// The entry.
        entry: usize,
        /// The path as written.
        path: String,
    },
    /// A path writes a positional predicate or a `//` pattern.
    #[error(
        "{mapping}#{entry}: `{path}` selects an instance or a pattern inside a path, and a record \
         iterates instances through base_path"
    )]
    UnsupportedStep {
        /// The file.
        mapping: String,
        /// The entry.
        entry: usize,
        /// The path as written.
        path: String,
    },
    /// No alternative of a column the mapping requires names a node of the
    /// template, so no instance could ever produce the record.
    #[error(
        "{mapping}#{entry}: no alternative of the required `{column}` names a node of the \
         template below {root}"
    )]
    UnboundRequiredColumn {
        /// The file.
        mapping: String,
        /// The entry.
        entry: usize,
        /// The OMOCL key.
        column: &'static str,
        /// The template path of the scope.
        root: String,
    },
    /// A `CustomMapping` names a converter the registry holds no object for.
    #[error("{mapping}#{entry}: no converter object is registered for `{name}`")]
    UnknownConverter {
        /// The file.
        mapping: String,
        /// The entry.
        entry: usize,
        /// The converter name.
        name: String,
    },
}

/// Renders an include chain.
fn render_chain(chain: &[ArchetypeId]) -> String {
    chain
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(" -> ")
}

/// Compiles `set` against the template `index` was built from.
///
/// Every archetype root of the template that a loaded file maps is bound,
/// except a root an `Include` of an enclosing bound root already binds, so a
/// node is mapped once.
///
/// # Errors
///
/// Returns every [`ResolveError`] the set raises against the template.
pub fn compile(
    set: &MappingSet,
    index: &WebTemplateIndex,
    converters: &dyn Converters,
) -> Result<Arc<Program>, Vec<ResolveError>> {
    let mut compiler = Compiler {
        set,
        index,
        converters,
        errors: Vec::new(),
        unbound: Vec::new(),
        covered: BTreeSet::new(),
    };
    let mut roots = Vec::new();
    for node in index.nodes() {
        let Some(node_id) = node.node_id() else {
            continue;
        };
        // NOTE: no specification governs this: our own design; a node id that
        // is not an archetype id is an at-code, never a mapped root.
        let Ok(archetype) = node_id.parse::<ArchetypeId>() else {
            continue;
        };
        let path = node.rm_path().clone();
        if compiler.covered.contains(&path.to_string()) {
            continue;
        }
        let mut chain = Vec::new();
        roots.append(&mut compiler.scopes(&archetype, &path, &mut chain));
    }
    if compiler.errors.is_empty() {
        Ok(Arc::new(Program {
            template_id: index.template_id().to_owned(),
            roots,
            unbound: compiler.unbound,
        }))
    } else {
        Err(compiler.errors)
    }
}

/// The state of one compilation.
struct Compiler<'a> {
    set: &'a MappingSet,
    index: &'a WebTemplateIndex,
    converters: &'a dyn Converters,
    errors: Vec<ResolveError>,
    unbound: Vec<Unbound>,
    covered: BTreeSet<String>,
}

impl fmt::Debug for Compiler<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Compiler")
            .field("errors", &self.errors)
            .finish_non_exhaustive()
    }
}

impl Compiler<'_> {
    /// Binds every file mapping `archetype` at `root`.
    fn scopes(
        &mut self,
        archetype: &ArchetypeId,
        root: &RmPath,
        chain: &mut Vec<ArchetypeId>,
    ) -> Vec<Arc<Scope>> {
        if chain.contains(archetype) {
            self.cycle(chain, archetype);
            return Vec::new();
        }
        let files: Vec<&MappingFile> = self.set.for_archetype(archetype).collect();
        if files.is_empty() {
            return Vec::new();
        }
        chain.push(archetype.clone());
        let mut scopes = Vec::new();
        for file in files {
            scopes.push(Arc::new(self.scope(file, root, chain)));
        }
        chain.pop();
        scopes
    }

    /// Binds one file at `root`.
    fn scope(&mut self, file: &MappingFile, root: &RmPath, chain: &mut Vec<ArchetypeId>) -> Scope {
        let mapping = file.header.name().value().clone();
        let mut entries = Vec::new();
        for (entry, entity) in file.entities.iter().enumerate() {
            let site = Site {
                mapping: mapping.as_str().to_owned(),
                entry,
                root: root.to_string(),
            };
            match *entity {
                Entity::Record(ref record) => {
                    if let Some(bound) = self.record(&site, record, root) {
                        entries.push(BoundEntry::Record(bound));
                    }
                }
                Entity::Include(ref include) => {
                    let archetype = include.archetype_id.value().clone();
                    // NOTE: no specification governs this: our own design; a
                    // cycle is refused by archetype alone, wherever it binds.
                    if chain.contains(&archetype) {
                        self.cycle(chain, &archetype);
                        continue;
                    }
                    let written = include
                        .base_path
                        .as_ref()
                        .map_or_else(|| SELF_STEP.to_owned(), |path| path.value().to_string());
                    let bound = match include.base_path {
                        None => Some(Bind::Bound((self_hop(), root.clone()))),
                        Some(ref path) => self.hop(&site, path.value(), root),
                    };
                    let Some(bound) = bound else {
                        continue;
                    };
                    let Bind::Bound((hop, target)) = bound else {
                        self.unbound(&site, "Include", &written);
                        continue;
                    };
                    let names_root = target
                        .segments
                        .last()
                        .and_then(|segment| segment.predicate.archetype_node_id.as_deref())
                        .is_some_and(|id| node_id_matches(archetype.as_str(), id));
                    if !names_root || self.index.at_rm_path(&target).is_err() {
                        self.unbound(&site, "Include", &written);
                        continue;
                    }
                    self.covered.insert(target.to_string());
                    let scopes = self.scopes(&archetype, &target, chain);
                    entries.push(BoundEntry::Include(BoundInclude {
                        entry,
                        archetype,
                        hop,
                        scopes,
                    }));
                }
                Entity::CustomMapping(ref custom) => {
                    match self.converters.converter(custom.name.value()) {
                        Some(converter) => {
                            entries.push(BoundEntry::Custom(BoundCustom { entry, converter }));
                        }
                        None => self.errors.push(ResolveError::UnknownConverter {
                            mapping: site.mapping.clone(),
                            entry,
                            name: custom.name.value().clone(),
                        }),
                    }
                }
            }
        }
        Scope {
            mapping,
            archetype: file.archetype.value().clone(),
            root: root.clone(),
            entries,
        }
    }

    /// Binds one record at `root`.
    fn record(&mut self, site: &Site, record: &Record, root: &RmPath) -> Option<BoundRecord> {
        let (base, anchor) = match record.base_path {
            None => (None, root.clone()),
            Some(ref path) => match self.hop(site, path.value(), root)? {
                Bind::Bound((hop, target)) if self.binds(&target) => (Some(hop), target),
                Bind::Bound(_) | Bind::Absent => {
                    self.unbound(site, "base_path", &path.value().to_string());
                    return None;
                }
            },
        };
        let mut columns = Vec::new();
        for column in &record.columns {
            let mut alternatives = Vec::new();
            for alternative in &column.alternatives {
                alternatives.push(self.alternative(site, column.key(), alternative, &anchor)?);
            }
            let bindable = alternatives
                .iter()
                .any(|alternative| !matches!(alternative, BoundAlternative::Unbound { .. }));
            if !bindable && !column.is_optional() {
                self.errors.push(ResolveError::UnboundRequiredColumn {
                    mapping: site.mapping.clone(),
                    entry: site.entry,
                    column: column.key(),
                    root: site.root.clone(),
                });
                return None;
            }
            columns.push(BoundColumn {
                projection: column.projection,
                optional: column.is_optional(),
                alternatives,
            });
        }
        Some(BoundRecord {
            entry: site.entry,
            target: *record.target.value(),
            base,
            columns,
        })
    }

    /// Binds one alternative at `anchor`.
    fn alternative(
        &mut self,
        site: &Site,
        key: &'static str,
        alternative: &Alternative,
        anchor: &RmPath,
    ) -> Option<BoundAlternative> {
        match *alternative {
            Alternative::Code(ref code) => Some(BoundAlternative::Code(*code.value())),
            Alternative::Path(ref path) => {
                Some(match self.path(site, key, path.value(), anchor)? {
                    Bind::Bound(bound) => BoundAlternative::Path(bound),
                    Bind::Absent => BoundAlternative::Unbound {
                        written: path.value().to_string(),
                    },
                })
            }
            Alternative::ConceptMap(ref map) => {
                let mapping = map
                    .mapping
                    .iter()
                    .map(|(code, concept)| (code.clone(), *concept.value()))
                    .collect();
                Some(match self.path(site, key, map.path.value(), anchor)? {
                    Bind::Bound(path) => BoundAlternative::ConceptMap { path, mapping },
                    Bind::Absent => BoundAlternative::Unbound {
                        written: map.path.value().to_string(),
                    },
                })
            }
            Alternative::Multiplication(ref multiplication) => {
                let mut factors = Vec::new();
                for factor in &multiplication.factors {
                    match *factor {
                        Factor::Code(ref code) => factors.push(BoundFactor::Code(*code.value())),
                        Factor::Path(ref path) => {
                            match self.path(site, key, path.value(), anchor)? {
                                Bind::Bound(bound) => factors.push(BoundFactor::Path(bound)),
                                Bind::Absent => {
                                    return Some(BoundAlternative::Unbound {
                                        written: path.value().to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
                Some(BoundAlternative::Multiplication(factors))
            }
        }
    }

    /// Binds one value path; [`Bind::Absent`] when the template carries no node
    /// for it.
    fn path(
        &mut self,
        site: &Site,
        key: &'static str,
        path: &MappingPath,
        anchor: &RmPath,
    ) -> Option<Bind<BoundPath>> {
        match self.hop(site, path, anchor)? {
            Bind::Bound((hop, target)) if self.binds(&target) => Some(Bind::Bound(BoundPath {
                written: path.to_string(),
                hop,
            })),
            Bind::Bound(_) | Bind::Absent => {
                self.unbound(site, key, &path.to_string());
                Some(Bind::Absent)
            }
        }
    }

    /// Splits a mapping path into its walk and the template path it reaches.
    ///
    /// `None` records an error; [`Bind::Absent`] is a path that walks above the
    /// template root from this anchor, so it names nothing here.
    fn hop(
        &mut self,
        site: &Site,
        path: &MappingPath,
        anchor: &RmPath,
    ) -> Option<Bind<(Hop, RmPath)>> {
        if path.variable().is_some() {
            self.errors.push(ResolveError::PathVariable {
                mapping: site.mapping.clone(),
                entry: site.entry,
                path: path.to_string(),
            });
            return None;
        }
        // NOTE: no specification governs this: our own design; the library's
        // `"."` is the self step, which names the anchor itself.
        let down: Vec<PathSegment> = path
            .tail()
            .segments
            .iter()
            .filter(|segment| segment.attribute != SELF_STEP)
            .cloned()
            .collect();
        if down
            .iter()
            .any(|segment| segment.descendant || segment.predicate.position.is_some())
        {
            self.errors.push(ResolveError::UnsupportedStep {
                mapping: site.mapping.clone(),
                entry: site.entry,
                path: path.to_string(),
            });
            return None;
        }
        let depth = anchor.segments.len();
        let Some(kept) = depth.checked_sub(path.parent_steps()) else {
            return Some(Bind::Absent);
        };
        let mut segments: Vec<PathSegment> = anchor.segments.iter().take(kept).cloned().collect();
        segments.extend(down.iter().cloned());
        Some(Bind::Bound((
            Hop {
                up: path.parent_steps(),
                down,
            },
            RmPath {
                absolute: anchor.absolute,
                segments,
            },
        )))
    }

    /// Whether the template carries `path`: a node at it, an `ELEMENT` the
    /// Web Template compacted into its value node, or a structure on the way
    /// to a node.
    ///
    /// The Web Template carries an `ELEMENT` as its `value` node and leaves
    /// the structures between an archetype root and its leaves out
    /// (Simplified Formats, §Web Template Metadata), so all three forms name
    /// a place a composition can hold. No specification governs this reading:
    /// our own design.
    fn binds(&self, path: &RmPath) -> bool {
        let mut element = path.clone();
        element.segments.push(PathSegment {
            attribute: "value".to_owned(),
            predicate: openehr_rm::v1_2::paths::Predicate::default(),
            descendant: false,
        });
        self.index.nodes().any(|node| {
            let carried = &node.rm_path().segments;
            prefix_of(&path.segments, carried) || same_place(&element.segments, carried)
        })
    }

    /// Records an include chain that returns to `archetype`, once.
    fn cycle(&mut self, chain: &[ArchetypeId], archetype: &ArchetypeId) {
        let mut cycle = chain.to_vec();
        cycle.push(archetype.clone());
        let error = ResolveError::IncludeCycle { chain: cycle };
        if !self.errors.contains(&error) {
            self.errors.push(error);
        }
    }

    /// Records one part of a mapping the template does not carry.
    fn unbound(&mut self, site: &Site, part: &str, path: &str) {
        self.unbound.push(Unbound {
            mapping: site.mapping.clone(),
            entry: site.entry,
            part: part.to_owned(),
            path: path.to_owned(),
            root: site.root.clone(),
        });
    }
}

/// A path bound at an anchor, or one the template carries no node for.
#[derive(Debug)]
enum Bind<T> {
    /// The path names a place of the template.
    Bound(T),
    /// The path names nothing of the template from this anchor.
    Absent,
}

/// Where in the set an entry is compiled.
#[derive(Debug)]
struct Site {
    mapping: String,
    entry: usize,
    root: String,
}

/// The walk that stays where it is.
const fn self_hop() -> Hop {
    Hop {
        up: 0,
        down: Vec::new(),
    }
}

/// Whether `written` names the leading segments of `carried`.
fn prefix_of(written: &[PathSegment], carried: &[PathSegment]) -> bool {
    written.len() <= carried.len()
        && written
            .iter()
            .zip(carried)
            .all(|(wanted, held)| segment_matches(wanted, held))
}

/// Whether `written` names exactly `carried`.
fn same_place(written: &[PathSegment], carried: &[PathSegment]) -> bool {
    written.len() == carried.len() && prefix_of(written, carried)
}

/// Whether one written segment names one template segment.
///
/// A node id the file writes must be the one the template carries, in its
/// interface form; a name the template fixes and the file does not write
/// selects nothing.
fn segment_matches(wanted: &PathSegment, held: &PathSegment) -> bool {
    if wanted.attribute != held.attribute {
        return false;
    }
    match (
        wanted.predicate.archetype_node_id.as_deref(),
        held.predicate.archetype_node_id.as_deref(),
    ) {
        (Some(wanted), Some(held)) => node_id_matches(wanted, held),
        (None, _) => true,
        (Some(_), None) => false,
    }
}
