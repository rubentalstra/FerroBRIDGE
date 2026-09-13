// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The diagnostic codes context resolution raises.
//!
//! Every refusal in this module tree renders into the one
//! [`openehr_mapping_core::diagnostic::Diagnostic`] the shared foundation
//! defines, carrying the file, the YAML position, the mapping name and the
//! path into the document model, and travels as
//! [`openehr_mapping_core::diagnostic::DiagnosticCode::Language`].

use core::fmt;

use openehr_mapping_core::diagnostic::DiagnosticCode;
use openehr_mapping_core::diagnostic::LanguageCode;

/// A refusal the resolver raises while compiling one context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResolveCode {
    /// The name handed to the compiler is no context mapping of the set.
    UnknownContext,
    /// A name the context lists is no model or extension mapping of the set.
    UnknownContextReference,
    /// A name the context lists under `extensions` is not an extension file.
    NotAnExtension,
    /// An extension file writes no `spec.extends`.
    ExtensionWithoutTarget,
    /// A top-level mapping of an extension file writes no `extension` method.
    ExtensionMethodMissing,
    /// A mapping nested under another carries an `extension` method.
    NestedExtensionMethod,
    /// An `add` carries the name of a mapping the model already has.
    AddNameCollision,
    /// An `append` carries a `with` or a condition.
    AppendCarriesMapping,
    /// An `append` names no `appendTo` target.
    AppendWithoutTarget,
    /// An `appendTo` or an `overwrite` names no mapping method of the model.
    UnknownExtensionTarget,
    /// Two extensions of one context overwrite one mapping name.
    RepeatedOverwrite,
    /// The model mapping writes no `spec.fhirConfig.structureDefinition`, so
    /// no resource type is named.
    ResourceTypeUnnamed,
    /// The named resource type is none the FHIR element table carries.
    UnknownResourceType,
    /// A `with.fhir` expression is outside the path grammar.
    MalformedFhirPath,
    /// A `with.fhir` expression does not bind to its anchor.
    UnanchoredFhirPath,
    /// A `with.fhir` expression names no element of the FHIR element table.
    UnknownFhirElement,
    /// A mapping that writes FHIR carries a read-only `with.fhir` expression.
    ReadOnlyFhirWrite,
    /// A `with.openehr` path is outside the openEHR path grammar.
    MalformedOpenehrPath,
    /// A `with.openehr` path names no node of the operational template.
    UnknownTemplateNode,
    /// A path opens with a variable that names nothing here.
    UnboundPathVariable,
    /// The archetype a model mapping declares is not the archetype of the
    /// node its `$archetype` resolves to.
    ArchetypeMismatch,
    /// The template carries no node for the archetype a model mapping
    /// declares, or carries more than one.
    UnresolvedArchetypeRoot,
    /// A `slotArchetype` names a model mapping already in the slot chain.
    SlotCycle,
    /// A mapping carries two mapping methods that exclude each other.
    ConflictingMappingMethods,
    /// The template the context names is not the template compiled against.
    TemplateIdMismatch,
    /// The template version the context pins is not the one the template
    /// carries.
    TemplateSemVerMismatch,
}

impl ResolveCode {
    /// Returns the stable spelling of the code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownContext => "fc-unknown-context",
            Self::UnknownContextReference => "fc-unknown-context-reference",
            Self::NotAnExtension => "fc-not-an-extension",
            Self::ExtensionWithoutTarget => "fc-extension-without-target",
            Self::ExtensionMethodMissing => "fc-extension-method-missing",
            Self::NestedExtensionMethod => "fc-nested-extension-method",
            Self::AddNameCollision => "fc-add-name-collision",
            Self::AppendCarriesMapping => "fc-append-carries-mapping",
            Self::AppendWithoutTarget => "fc-append-without-target",
            Self::UnknownExtensionTarget => "fc-unknown-extension-target",
            Self::RepeatedOverwrite => "fc-repeated-overwrite",
            Self::ResourceTypeUnnamed => "fc-resource-type-unnamed",
            Self::UnknownResourceType => "fc-unknown-resource-type",
            Self::MalformedFhirPath => "fc-malformed-fhir-path",
            Self::UnanchoredFhirPath => "fc-unanchored-fhir-path",
            Self::UnknownFhirElement => "fc-unknown-fhir-element",
            Self::ReadOnlyFhirWrite => "fc-read-only-fhir-write",
            Self::MalformedOpenehrPath => "fc-malformed-openehr-path",
            Self::UnknownTemplateNode => "fc-unknown-template-node",
            Self::UnboundPathVariable => "fc-unbound-path-variable",
            Self::ArchetypeMismatch => "fc-archetype-mismatch",
            Self::UnresolvedArchetypeRoot => "fc-unresolved-archetype-root",
            Self::SlotCycle => "fc-slot-cycle",
            Self::ConflictingMappingMethods => "fc-conflicting-mapping-methods",
            Self::TemplateIdMismatch => "fc-template-id-mismatch",
            Self::TemplateSemVerMismatch => "fc-template-sem-ver-mismatch",
        }
    }

    /// Every code this enum defines, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::UnknownContext,
            Self::UnknownContextReference,
            Self::NotAnExtension,
            Self::ExtensionWithoutTarget,
            Self::ExtensionMethodMissing,
            Self::NestedExtensionMethod,
            Self::AddNameCollision,
            Self::AppendCarriesMapping,
            Self::AppendWithoutTarget,
            Self::UnknownExtensionTarget,
            Self::RepeatedOverwrite,
            Self::ResourceTypeUnnamed,
            Self::UnknownResourceType,
            Self::MalformedFhirPath,
            Self::UnanchoredFhirPath,
            Self::UnknownFhirElement,
            Self::ReadOnlyFhirWrite,
            Self::MalformedOpenehrPath,
            Self::UnknownTemplateNode,
            Self::UnboundPathVariable,
            Self::ArchetypeMismatch,
            Self::UnresolvedArchetypeRoot,
            Self::SlotCycle,
            Self::ConflictingMappingMethods,
            Self::TemplateIdMismatch,
            Self::TemplateSemVerMismatch,
        ]
    }
}

impl fmt::Display for ResolveCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ResolveCode> for DiagnosticCode {
    /// # Panics
    ///
    /// Never in practice. Every spelling [`ResolveCode::as_str`] returns is
    /// lowercase ASCII letters and hyphens, which is the alphabet
    /// [`LanguageCode::new`] admits, and `every_code_is_a_language_code` in
    /// this module asserts that over [`ResolveCode::all`].
    #[expect(
        clippy::expect_used,
        reason = "the code alphabet is fixed by as_str and pinned by every_code_is_a_language_code"
    )]
    fn from(code: ResolveCode) -> Self {
        let language =
            LanguageCode::new(code.as_str()).expect("a resolve code should be a language code");
        Self::Language(language)
    }
}

#[cfg(test)]
mod tests {
    use openehr_mapping_core::diagnostic::DiagnosticCode;
    use openehr_mapping_core::diagnostic::LanguageCode;

    use super::ResolveCode;

    #[test]
    fn every_code_is_a_language_code() {
        for code in ResolveCode::all() {
            assert!(
                LanguageCode::new(code.as_str()).is_ok(),
                "the code `{code}` is outside the language-code alphabet"
            );
        }
    }

    #[test]
    fn every_code_has_a_distinct_spelling() {
        let mut spellings: Vec<&str> = ResolveCode::all()
            .iter()
            .map(|code| code.as_str())
            .collect();
        let total = spellings.len();
        spellings.sort_unstable();
        spellings.dedup();
        assert_eq!(spellings.len(), total, "two resolve codes share a spelling");
    }

    #[test]
    fn a_code_renders_as_its_diagnostic_code() {
        assert_eq!(
            DiagnosticCode::from(ResolveCode::SlotCycle).to_string(),
            "fc-slot-cycle"
        );
    }
}
