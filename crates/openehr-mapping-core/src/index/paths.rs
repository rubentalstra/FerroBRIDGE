// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two node identifiers of a Web Template and the relative path between
//! two nodes.

use openehr_rm::v1_2::paths::PathSegment;
use openehr_rm::v1_2::paths::RmPath;

use crate::index::ResolvedNode;
use crate::index::matching::parse_aql_path;
use crate::template::PathError;

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
