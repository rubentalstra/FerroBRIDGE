// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The immutable output of compiling a mapping set against one template.
//!
//! A [`Program`] is built once by [`crate::resolve::compile`] and never
//! changes afterwards: every path is already a list of BASE path segments
//! anchored where the mapping is bound, every `Include` is already the scopes
//! it binds, and every `CustomMapping` is already its converter, so the engine
//! parses nothing and reads no mapping file. The [`fmt::Display`] rendering
//! is the shape the snapshot tests pin.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::Arc;

use openehr_mapping_core::header::ArchetypeId;
use openehr_mapping_core::header::MappingName;
use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::RmPath;

use crate::engine::custom::CustomConverter;
use crate::model::ast::AtCode;
use crate::model::ast::ConceptId;
use crate::model::ast::Target;
use crate::model::projection::KeyProjection;

/// Every mapping of one template, bound at the archetype roots it maps.
#[derive(Debug)]
pub struct Program {
    pub(crate) template_id: String,
    pub(crate) roots: Vec<Arc<Scope>>,
    pub(crate) unbound: Vec<Unbound>,
}

impl Program {
    /// Returns the template the program was compiled against.
    #[must_use]
    pub fn template_id(&self) -> &str {
        &self.template_id
    }

    /// Returns the scopes bound at the template's outermost mapped roots.
    #[must_use]
    pub fn roots(&self) -> &[Arc<Scope>] {
        &self.roots
    }

    /// Returns what the template carries no node for, and so never applies.
    #[must_use]
    pub fn unbound(&self) -> &[Unbound] {
        &self.unbound
    }
}

/// A walk from an anchor: `up` parent steps, then `down` segments.
///
/// A mapping path opens with any number of `../` steps and then names
/// segments below what it reached, so a bound path is exactly those two
/// parts, relative to the instance it is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub(crate) up: usize,
    pub(crate) down: Vec<PathSegment>,
}

impl Hop {
    /// Returns how many parent steps the walk takes first.
    #[must_use]
    pub const fn up(&self) -> usize {
        self.up
    }

    /// Returns the segments the walk descends.
    #[must_use]
    pub fn down(&self) -> &[PathSegment] {
        &self.down
    }
}

/// The mappings of one file, bound at one archetype root of the template.
#[derive(Debug)]
pub struct Scope {
    pub(crate) mapping: MappingName,
    pub(crate) archetype: ArchetypeId,
    pub(crate) root: RmPath,
    pub(crate) entries: Vec<BoundEntry>,
}

impl Scope {
    /// Returns the `metadata.name` of the file.
    #[must_use]
    pub const fn mapping(&self) -> &MappingName {
        &self.mapping
    }

    /// Returns the archetype the file maps.
    #[must_use]
    pub const fn archetype(&self) -> &ArchetypeId {
        &self.archetype
    }

    /// Returns the template path of the archetype root the scope is bound at.
    #[must_use]
    pub const fn root(&self) -> &RmPath {
        &self.root
    }

    /// Returns the bound entries, in file order.
    #[must_use]
    pub fn entries(&self) -> &[BoundEntry] {
        &self.entries
    }
}

/// One bound entry of `mappings`.
#[derive(Debug)]
#[non_exhaustive]
pub enum BoundEntry {
    /// A record writing into a CDM table.
    Record(BoundRecord),
    /// An `Include`, with the scopes it binds.
    Include(BoundInclude),
    /// A `CustomMapping`, with its converter.
    Custom(BoundCustom),
}

/// A bound record.
#[derive(Debug)]
pub struct BoundRecord {
    pub(crate) entry: usize,
    pub(crate) target: Target,
    pub(crate) base: Option<Hop>,
    pub(crate) columns: Vec<BoundColumn>,
}

impl BoundRecord {
    /// Returns the index of the entry within `mappings`.
    #[must_use]
    pub const fn entry(&self) -> usize {
        self.entry
    }

    /// Returns the CDM target.
    #[must_use]
    pub const fn target(&self) -> Target {
        self.target
    }

    /// Returns the columns, in key order.
    #[must_use]
    pub fn columns(&self) -> &[BoundColumn] {
        &self.columns
    }
}

/// A bound column: its projection and its alternatives in order.
#[derive(Debug)]
pub struct BoundColumn {
    pub(crate) projection: &'static KeyProjection,
    pub(crate) optional: bool,
    pub(crate) alternatives: Vec<BoundAlternative>,
}

impl BoundColumn {
    /// Returns the OMOCL key.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        self.projection.key
    }

    /// Returns the alternatives, in file order.
    #[must_use]
    pub fn alternatives(&self) -> &[BoundAlternative] {
        &self.alternatives
    }
}

/// A path bound at its anchor, with the spelling the file wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundPath {
    pub(crate) written: String,
    pub(crate) hop: Hop,
}

impl BoundPath {
    /// Returns the path as the mapping file writes it.
    #[must_use]
    pub fn written(&self) -> &str {
        &self.written
    }

    /// Returns the walk from the anchor.
    #[must_use]
    pub const fn hop(&self) -> &Hop {
        &self.hop
    }
}

/// One bound alternative.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoundAlternative {
    /// A value read from the composition.
    Path(BoundPath),
    /// A literal concept id.
    Code(ConceptId),
    /// An at-code table over one coded node.
    ConceptMap {
        /// The coded node.
        path: BoundPath,
        /// The at-code to concept id table.
        mapping: BTreeMap<AtCode, ConceptId>,
    },
    /// The product of its factors.
    Multiplication(Vec<BoundFactor>),
    /// An alternative whose path names no node of this template, so it is
    /// never present.
    Unbound {
        /// The path as the file writes it.
        written: String,
    },
}

/// One bound factor of a `multiplication`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoundFactor {
    /// A number read from the composition.
    Path(BoundPath),
    /// A literal integer factor.
    Code(i64),
}

/// A bound `Include`.
#[derive(Debug)]
pub struct BoundInclude {
    pub(crate) entry: usize,
    pub(crate) archetype: ArchetypeId,
    pub(crate) hop: Hop,
    pub(crate) scopes: Vec<Arc<Scope>>,
}

impl BoundInclude {
    /// Returns the index of the entry within `mappings`.
    #[must_use]
    pub const fn entry(&self) -> usize {
        self.entry
    }

    /// Returns the scopes of the files mapping the included archetype.
    #[must_use]
    pub fn scopes(&self) -> &[Arc<Scope>] {
        &self.scopes
    }
}

/// A bound `CustomMapping`.
#[derive(Debug)]
pub struct BoundCustom {
    pub(crate) entry: usize,
    pub(crate) converter: Arc<dyn CustomConverter>,
}

impl BoundCustom {
    /// Returns the index of the entry within `mappings`.
    #[must_use]
    pub const fn entry(&self) -> usize {
        self.entry
    }

    /// Returns the converter.
    #[must_use]
    pub fn converter(&self) -> &dyn CustomConverter {
        self.converter.as_ref()
    }
}

/// A part of a mapping the template carries no node for.
///
/// OMOCL maps an archetype and a template may leave any node of it out, so an
/// alternative, a record base or an `Include` whose path the template does not
/// carry is never present. No specification governs this: our own design,
/// recorded so the program states what it will never apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unbound {
    /// The `metadata.name` of the file.
    pub mapping: String,
    /// The index of the entry within `mappings`.
    pub entry: usize,
    /// What is unbound: an OMOCL key, `base_path` or `Include`.
    pub part: String,
    /// The path as the file writes it.
    pub path: String,
    /// The template path of the scope the entry belongs to.
    pub root: String,
}

impl fmt::Display for Unbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}#{} {} `{}` below {}",
            self.mapping, self.entry, self.part, self.path, self.root
        )
    }
}

impl fmt::Display for Hop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "up {} down ", self.up)?;
        let path = RmPath {
            absolute: false,
            segments: self.down.clone(),
        };
        if self.down.is_empty() {
            f.write_str(".")
        } else {
            write!(f, "{path}")
        }
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "program {}", self.template_id)?;
        for scope in &self.roots {
            write_scope(f, scope, 1)?;
        }
        for unbound in &self.unbound {
            writeln!(f, "  unbound {unbound}")?;
        }
        Ok(())
    }
}

/// Writes one scope and everything under it.
fn write_scope(f: &mut fmt::Formatter<'_>, scope: &Scope, depth: usize) -> fmt::Result {
    let pad = "  ".repeat(depth);
    writeln!(
        f,
        "{pad}scope {} ({}) at {}",
        scope.mapping.as_str(),
        scope.archetype,
        scope.root
    )?;
    for entry in &scope.entries {
        match *entry {
            BoundEntry::Record(ref record) => {
                let base = record
                    .base
                    .as_ref()
                    .map_or_else(String::new, |hop| format!(" base {hop}"));
                writeln!(f, "{pad}  record#{} {}{base}", record.entry, record.target)?;
                for column in &record.columns {
                    let optional = if column.optional { " optional" } else { "" };
                    let alternatives: Vec<String> =
                        column.alternatives.iter().map(render_alternative).collect();
                    writeln!(
                        f,
                        "{pad}    {}{optional}: {}",
                        column.key(),
                        alternatives.join(" | ")
                    )?;
                }
            }
            BoundEntry::Include(ref include) => {
                writeln!(
                    f,
                    "{pad}  include#{} {} {}",
                    include.entry, include.archetype, include.hop
                )?;
                for child in &include.scopes {
                    write_scope(f, child, depth.saturating_add(2))?;
                }
            }
            BoundEntry::Custom(ref custom) => {
                writeln!(
                    f,
                    "{pad}  custom#{} {}",
                    custom.entry,
                    custom.converter.name()
                )?;
            }
        }
    }
    Ok(())
}

/// Renders one alternative for the program listing.
fn render_alternative(alternative: &BoundAlternative) -> String {
    match *alternative {
        BoundAlternative::Path(ref path) => format!("path {}", path.hop),
        BoundAlternative::Code(code) => format!("code {code}"),
        BoundAlternative::ConceptMap {
            ref path,
            ref mapping,
        } => {
            let pairs: Vec<String> = mapping
                .iter()
                .map(|(code, concept)| format!("{code}={concept}"))
                .collect();
            format!("conceptMap {} {{{}}}", path.hop, pairs.join(", "))
        }
        BoundAlternative::Multiplication(ref factors) => {
            let rendered: Vec<String> = factors
                .iter()
                .map(|factor| match *factor {
                    BoundFactor::Path(ref path) => format!("path {}", path.hop),
                    BoundFactor::Code(code) => format!("code {code}"),
                })
                .collect();
            format!("multiplication [{}]", rendered.join(", "))
        }
        BoundAlternative::Unbound { ref written } => format!("unbound `{written}`"),
    }
}
