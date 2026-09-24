// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Applying the extension mappings of one context to one model mapping.
//!
//! An extension inherits every mapping of the model mapping it extends and
//! interacts with them through `add`, `append` and `overwrite`
//! (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`).
//! The specification fixes what each method does and leaves open in which
//! order several extensions apply and what happens when their edits collide,
//! so the ordering and the collision refusals below are FerroBRIDGE's own.
//!
//! The merge borrows the loaded files rather than copying them: a [`Node`] is
//! one mapping method plus the file it was read from, and the tree of nodes is
//! what the compiler walks.

use std::path::Path;

use openehr_mapping_core::diagnostic::Diagnostic;
use openehr_mapping_core::diagnostic::ModelPath;
use openehr_mapping_core::header::MappingName;
use openehr_mapping_core::position::Position;

use crate::model::ast::ExtensionMethod;
use crate::model::ast::Mapping;
use crate::model::ast::ModelMappingFile;
use crate::resolve::error::ResolveCode;

/// One mapping method of a merged model mapping, with the file it came from.
///
/// The method's own `followedBy` children are materialized into
/// [`Node::followed_by`] so an `append` can add to them at any depth, and its
/// `reference` children into [`Node::reference`].
#[derive(Debug, Clone)]
pub(crate) struct Node<'a> {
    /// The mapping method as the file writes it.
    pub(crate) mapping: &'a Mapping,
    /// The file the method was read from.
    pub(crate) origin: &'a ModelMappingFile,
    /// Where the method sits in that file's document model.
    pub(crate) path: ModelPath,
    /// The methods that run after it.
    pub(crate) followed_by: Vec<Node<'a>>,
    /// The methods that populate a referenced resource.
    pub(crate) reference: Vec<Node<'a>>,
}

impl<'a> Node<'a> {
    /// Materializes one mapping method and everything nested under it.
    ///
    /// `path` is where the method sits in `origin`, so a refusal about a node
    /// an extension contributed names a place in the extension's own file
    /// rather than a position in the merged tree.
    fn of(mapping: &'a Mapping, origin: &'a ModelMappingFile, path: ModelPath) -> Self {
        let followed_by = mapping
            .followed_by
            .as_ref()
            .map(|followed| Self::all(&followed.mappings, origin, &path.field("followedBy")))
            .unwrap_or_default();
        let reference = mapping
            .reference
            .as_ref()
            .map(|reference| Self::all(&reference.mappings, origin, &path.field("reference")))
            .unwrap_or_default();
        Self {
            mapping,
            origin,
            path,
            followed_by,
            reference,
        }
    }

    /// Materializes a list of mapping methods under `parent`.
    fn all(mappings: &'a [Mapping], origin: &'a ModelMappingFile, parent: &ModelPath) -> Vec<Self> {
        let at = parent.field("mappings");
        mappings
            .iter()
            .enumerate()
            .map(|(index, mapping)| Self::of(mapping, origin, at.index(index)))
            .collect()
    }

    /// Returns the name the method is addressed by.
    pub(crate) fn name(&self) -> &str {
        self.mapping.name.value()
    }
}

/// One model mapping with the extensions of one context applied to it.
#[derive(Debug, Clone)]
pub(crate) struct Merged<'a> {
    /// The mapping methods, in the order the engine runs them.
    pub(crate) mappings: Vec<Node<'a>>,
    /// The extensions that applied, in the order they applied.
    pub(crate) applied: Vec<MappingName>,
}

/// Applies `extensions` to `model`, in the order the context declares them.
///
/// Within one extension the methods apply in file order. Every refusal is
/// collected rather than raised, so one run reports every collision.
pub(crate) fn apply<'a>(
    model: &'a ModelMappingFile,
    extensions: &[&'a ModelMappingFile],
    diagnostics: &mut Vec<Diagnostic>,
) -> Merged<'a> {
    refuse_duplicate_names(model, model.mappings(), &ModelPath::root(), diagnostics);
    for extension in extensions {
        refuse_duplicate_names(
            extension,
            extension.mappings(),
            &ModelPath::root(),
            diagnostics,
        );
    }
    let mut mappings = Node::all(model.mappings(), model, &ModelPath::root());
    let mut applied = Vec::new();
    let mut overwritten: Vec<(String, &MappingName)> = Vec::new();

    for extension in extensions {
        let name = extension.header().name().value();
        applied.push(name.clone());
        for (index, method) in extension.mappings().iter().enumerate() {
            let path = ModelPath::root().field("mappings").index(index);
            refuse_nested_methods(extension, method, &path, diagnostics);
            let Some(ref located) = method.extension else {
                diagnostics.push(refusal(
                    extension,
                    name,
                    ResolveCode::ExtensionMethodMissing,
                    method.position,
                    &path.field("extension"),
                    format!(
                        "`{}` writes no extension method, so nothing says how it meets `{}`",
                        method.name.value(),
                        model.header().name().value()
                    ),
                ));
                continue;
            };
            match *located.value() {
                ExtensionMethod::Add => add(extension, method, &path, &mut mappings, diagnostics),
                ExtensionMethod::Append => {
                    append(extension, method, &path, &mut mappings, diagnostics);
                }
                ExtensionMethod::Overwrite => overwrite(
                    extension,
                    method,
                    &path,
                    &mut mappings,
                    &mut overwritten,
                    diagnostics,
                ),
            }
        }
    }

    Merged { mappings, applied }
}

/// Adds one method at the bottom of the model mapping.
///
/// "This extension method adds a mapping method to the model mapping. It is
/// always added at the bottom of the model mapping file"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Add). The same section says an `add` "can also overwrite prior methods to
/// the same path", which the bottom position gives on its own: the added
/// method is the last one that writes that path. A repeated `name` is another
/// matter, because `appendTo` and `overwrite` address a method by it, so that
/// is a refusal.
fn add<'a>(
    extension: &'a ModelMappingFile,
    method: &'a Mapping,
    path: &ModelPath,
    mappings: &mut Vec<Node<'a>>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let name = method.name.value();
    if let Some(existing) = mappings.iter().find(|node| node.name() == name) {
        let owner = existing.origin.header().name().value();
        diagnostics.push(refusal(
            extension,
            extension.header().name().value(),
            ResolveCode::AddNameCollision,
            method.name.position(),
            &path.field("name"),
            format!(
                "`{name}` is already a mapping method of `{owner}` ({}), so the `add` in `{}` \
                 ({}) has no unambiguous target",
                existing.origin.file().display(),
                extension.header().name().value(),
                extension.file().display()
            ),
        ));
        return;
    }
    mappings.push(Node::of(method, extension, path.clone()));
}

/// Appends the method's `followedBy` to the method `appendTo` names.
///
/// "To maintain readability the append is transformed into a followedBy. If
/// more logic needs to be altered, the overwrite method must be used instead"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Append), so an `append` that carries mapping logic of its own is refused
/// rather than silently replacing what the model mapping already holds.
fn append<'a>(
    extension: &'a ModelMappingFile,
    method: &'a Mapping,
    path: &ModelPath,
    mappings: &mut [Node<'a>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let owner = extension.header().name().value();
    for (key, position) in carried_keys(method) {
        diagnostics.push(refusal(
            extension,
            owner,
            ResolveCode::AppendCarriesMapping,
            position,
            &path.field(key),
            format!(
                "the `append` `{}` carries a `{key}`, and an append only adds a `followedBy`",
                method.name.value()
            ),
        ));
    }
    let Some(ref target) = method.append_to else {
        diagnostics.push(refusal(
            extension,
            owner,
            ResolveCode::AppendWithoutTarget,
            method.position,
            &path.field("appendTo"),
            format!(
                "the `append` `{}` names no `appendTo` target",
                method.name.value()
            ),
        ));
        return;
    };
    let Some(node) = locate_mut(mappings, target.value()) else {
        diagnostics.push(refusal(
            extension,
            owner,
            ResolveCode::UnknownExtensionTarget,
            target.position(),
            &path.field("appendTo"),
            format!(
                "`{}` names no mapping method of the merged model mapping",
                target.value()
            ),
        ));
        return;
    };
    let appended = method
        .followed_by
        .as_ref()
        .map(|followed| Node::all(&followed.mappings, extension, &path.field("followedBy")))
        .unwrap_or_default();
    node.followed_by.extend(appended);
}

/// Returns every key of an `append` beyond the four an append is made of.
///
/// An `append` carries `name`, `extension`, `appendTo` and `followedBy`, and
/// "if more logic needs to be altered, the overwrite method must be used
/// instead"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Append), so every other key is refused rather than dropped.
fn carried_keys(method: &Mapping) -> Vec<(&'static str, Position)> {
    let carried = [
        method.with.as_ref().map(|with| ("with", with.position)),
        method
            .fhir_condition
            .as_ref()
            .map(|condition| ("fhirCondition", condition.position)),
        method
            .openehr_condition
            .as_ref()
            .map(|condition| ("openehrCondition", condition.position)),
        method
            .manual
            .first()
            .map(|entry| ("manual", entry.position)),
        method
            .reference
            .as_ref()
            .map(|reference| ("reference", reference.position)),
        method
            .slot_archetype
            .as_ref()
            .map(|slot| ("slotArchetype", slot.position())),
        method
            .mapping_code
            .as_ref()
            .map(|code| ("mappingCode", code.position())),
        method.link.as_ref().map(|link| ("link", link.position)),
        method
            .participations_function
            .as_ref()
            .map(|function| ("participationsFunction", function.position())),
        method
            .conceptmap
            .as_ref()
            .map(|conceptmap| ("conceptmap", conceptmap.position())),
        method
            .unidirectional
            .as_ref()
            .map(|direction| ("unidirectional", direction.position())),
    ];
    carried.into_iter().flatten().collect()
}

/// Replaces the method of the same name.
///
/// "This method allows to overwrite an existing method contained in the model
/// mapping"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Overwrite). Two extensions of one context overwriting one name is refused,
/// because the result would depend on which one the engine happened to apply
/// last.
fn overwrite<'a>(
    extension: &'a ModelMappingFile,
    method: &'a Mapping,
    path: &ModelPath,
    mappings: &mut [Node<'a>],
    overwritten: &mut Vec<(String, &'a MappingName)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let owner = extension.header().name().value();
    let target = method.name.value().clone();
    if let Some((_, first)) = overwritten.iter().find(|(name, _)| *name == target) {
        diagnostics.push(refusal(
            extension,
            owner,
            ResolveCode::RepeatedOverwrite,
            method.name.position(),
            &path.field("name"),
            format!("`{first}` and `{owner}` both overwrite `{target}` in one context"),
        ));
        return;
    }
    let Some(node) = locate_mut(mappings, &target) else {
        diagnostics.push(refusal(
            extension,
            owner,
            ResolveCode::UnknownExtensionTarget,
            method.name.position(),
            &path.field("name"),
            format!("`{target}` names no mapping method of the merged model mapping"),
        ));
        return;
    };
    *node = Node::of(method, extension, path.clone());
    overwritten.push((target, owner));
}

/// Refuses an `extension` method written on a nested mapping.
///
/// The extension-methods page defines the three methods for a mapping method
/// of an extension file and gives no meaning to one written on a child method,
/// so applying it would be a guess. No specification governs this: our own
/// design.
fn refuse_nested_methods(
    extension: &ModelMappingFile,
    method: &Mapping,
    path: &ModelPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let owner = extension.header().name().value();
    let nested = crate::model::ast::walk(std::slice::from_ref(method));
    for child in nested.into_iter().skip(1) {
        if let Some(ref located) = child.extension {
            diagnostics.push(refusal(
                extension,
                owner,
                ResolveCode::NestedExtensionMethod,
                located.position(),
                &path.field("extension"),
                format!(
                    "`{}` is nested under `{}` and carries the extension method `{}`, which \
                     applies to a mapping method of the file only",
                    child.name.value(),
                    method.name.value(),
                    located.value()
                ),
            ));
        }
    }
}

/// Refuses two sibling mapping methods of one file that carry one name.
///
/// `overwrite` "overwrites the mapping method with the same name" and
/// `appendTo` names a method, or a child by its dotted path
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`),
/// so a name two siblings share addresses neither of them, and an edit aimed
/// at one could land on the other. The refusal names both positions. Every
/// list is checked, the `followedBy` and `reference` lists included.
fn refuse_duplicate_names(
    file: &ModelMappingFile,
    mappings: &[Mapping],
    parent: &ModelPath,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let at = parent.field("mappings");
    for (index, method) in mappings.iter().enumerate() {
        let path = at.index(index);
        if let Some(first) = mappings
            .iter()
            .take(index)
            .find(|earlier| earlier.name.value() == method.name.value())
        {
            let first_at = first.name.position();
            let again = method.name.position();
            diagnostics.push(refusal(
                file,
                file.header().name().value(),
                ResolveCode::DuplicateMethodName,
                again,
                &path.field("name"),
                format!(
                    "`{}` names two mapping methods of one list in {}, at line {} column {} and \
                     at line {} column {}, so an `overwrite` or an `appendTo` of it has no \
                     single target",
                    method.name.value(),
                    file.file().display(),
                    first_at.line(),
                    first_at.column(),
                    again.line(),
                    again.column()
                ),
            ));
        }
        if let Some(ref followed) = method.followed_by {
            refuse_duplicate_names(
                file,
                &followed.mappings,
                &path.field("followedBy"),
                diagnostics,
            );
        }
        if let Some(ref reference) = method.reference {
            refuse_duplicate_names(
                file,
                &reference.mappings,
                &path.field("reference"),
                diagnostics,
            );
        }
    }
}

/// Resolves a dotted `parent.child` mapping-method path in the merged tree.
///
/// "The value used is the name of the mapping method. This can be also the
/// child method. The path then would be `appendTo: parent.child`"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mapping-files/extension-methods.adoc`,
/// §Append).
fn locate_mut<'n, 'a>(mappings: &'n mut [Node<'a>], dotted: &str) -> Option<&'n mut Node<'a>> {
    let mut segments = dotted.split('.');
    let first = segments.next()?;
    let mut current = mappings.iter_mut().find(|node| node.name() == first)?;
    for segment in segments {
        current = current
            .followed_by
            .iter_mut()
            .chain(current.reference.iter_mut())
            .find(|node| node.name() == segment)?;
    }
    Some(current)
}

/// Builds one refusal about an extension file.
fn refusal(
    file: &ModelMappingFile,
    owner: &MappingName,
    code: ResolveCode,
    position: Position,
    path: &ModelPath,
    message: String,
) -> Diagnostic {
    diagnostic(file.file(), owner, code, position, path, message)
}

/// Builds one refusal about a file.
pub(crate) fn diagnostic(
    file: &Path,
    owner: &MappingName,
    code: ResolveCode,
    position: Position,
    path: &ModelPath,
    message: String,
) -> Diagnostic {
    Diagnostic::error(file.to_path_buf(), code.into(), message)
        .with_position(position)
        .with_mapping_name(owner.clone())
        .with_model_path(path.clone())
}
