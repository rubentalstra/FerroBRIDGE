// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The preprocessor of a model mapping and its `hierarchy.split`.

use core::fmt;
use core::str::FromStr;

use openehr_mapping_core::header::metadata::MappingName;

use crate::resolve::program::condition::Condition;
use crate::resolve::program::target::FhirTarget;
use crate::resolve::program::target::OpenehrTarget;
use crate::resolve::program::target::Target;

/// What one side of a `hierarchy.split` creates for each occurrence.
///
/// "The `create` method defines the element to create. In the case of a
/// `resource` or `archetype`, this type is inferred by the FHIRconnect mapping
/// file it is included in"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/concept-type/HierarchyMappings.adoc`,
/// §split), and its worked example writes `event` for the openEHR side. The
/// specification enumerates no others and no schema constrains the key, so the
/// closed set is FerroBRIDGE's own: no specification governs this: our own
/// design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Create {
    /// A FHIR resource of the type the mapping file names.
    Resource,
    /// An openEHR `EVENT` of the archetype the mapping file names.
    Event,
    /// An openEHR archetype root of the type the mapping file names.
    Archetype,
}

impl Create {
    /// Returns the spelling a mapping file writes.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::Event => "event",
            Self::Archetype => "archetype",
        }
    }

    /// Every spelling the key admits, in declaration order.
    #[must_use]
    pub const fn admitted() -> &'static [&'static str] {
        &["resource", "event", "archetype"]
    }
}

impl fmt::Display for Create {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Create {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "resource" => Ok(Self::Resource),
            "event" => Ok(Self::Event),
            "archetype" => Ok(Self::Archetype),
            _ => Err(()),
        }
    }
}

/// One side of a compiled `hierarchy.split`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    create: Option<Create>,
    path: Option<Target>,
    unique: Vec<Target>,
}

impl Split {
    /// Assembles one side of a split.
    #[must_use]
    pub const fn new(create: Option<Create>, path: Option<Target>, unique: Vec<Target>) -> Self {
        Self {
            create,
            path,
            unique,
        }
    }

    /// Returns the element the split creates.
    #[must_use]
    pub const fn create(&self) -> Option<Create> {
        self.create
    }

    /// Returns the resolved path the element is created at.
    #[must_use]
    pub const fn path(&self) -> Option<&Target> {
        self.path.as_ref()
    }

    /// Returns the resolved paths whose distinct values trigger a split.
    #[must_use]
    pub fn unique(&self) -> &[Target] {
        &self.unique
    }
}

/// The compiled `preprocessor` of one model or extension mapping file.
///
/// A preprocessor condition "defines that the mapping file is only executed if
/// the given condition is met"
/// (`docs/specs/fhirconnect/modules/ROOT/pages/basics/Conditions.adoc`,
/// §Conditions in the preprocessor), so the gate belongs to the file that
/// wrote it. One program carries one of these per contributing file, which is
/// how a slotted model mapping and an extension keep their own gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preprocessor {
    model: MappingName,
    fhir_condition: Option<Condition>,
    openehr_condition: Option<Condition>,
    hierarchy: Option<Hierarchy>,
}

impl Preprocessor {
    /// Assembles the compiled preprocessor of one file.
    #[must_use]
    pub const fn new(
        model: MappingName,
        fhir_condition: Option<Condition>,
        openehr_condition: Option<Condition>,
        hierarchy: Option<Hierarchy>,
    ) -> Self {
        Self {
            model,
            fhir_condition,
            openehr_condition,
            hierarchy,
        }
    }

    /// Returns the `metadata.name` of the file the preprocessor came from.
    #[must_use]
    pub const fn model(&self) -> &MappingName {
        &self.model
    }

    /// Returns the gate the FHIR to openEHR direction evaluates.
    #[must_use]
    pub const fn fhir_condition(&self) -> Option<&Condition> {
        self.fhir_condition.as_ref()
    }

    /// Returns the gate the openEHR to FHIR direction evaluates.
    #[must_use]
    pub const fn openehr_condition(&self) -> Option<&Condition> {
        self.openehr_condition.as_ref()
    }

    /// Returns the hierarchy realignment the file writes.
    #[must_use]
    pub const fn hierarchy(&self) -> Option<&Hierarchy> {
        self.hierarchy.as_ref()
    }

    /// Whether the file's preprocessor carries nothing the engine runs.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.fhir_condition.is_none()
            && self.openehr_condition.is_none()
            && self.hierarchy.is_none()
    }
}

/// The compiled `preprocessor.hierarchy` of one mapping file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hierarchy {
    fhir: Option<FhirTarget>,
    openehr: Option<OpenehrTarget>,
    split_fhir: Option<Split>,
    split_openehr: Option<Split>,
}

impl Hierarchy {
    /// Assembles a compiled hierarchy mapping.
    #[must_use]
    pub const fn new(
        fhir: Option<FhirTarget>,
        openehr: Option<OpenehrTarget>,
        split_fhir: Option<Split>,
        split_openehr: Option<Split>,
    ) -> Self {
        Self {
            fhir,
            openehr,
            split_fhir,
            split_openehr,
        }
    }

    /// Returns the FHIR side of the iterated path.
    #[must_use]
    pub const fn fhir(&self) -> Option<&FhirTarget> {
        self.fhir.as_ref()
    }

    /// Returns the openEHR side of the iterated path.
    #[must_use]
    pub const fn openehr(&self) -> Option<&OpenehrTarget> {
        self.openehr.as_ref()
    }

    /// Returns the split that creates FHIR resources.
    #[must_use]
    pub const fn split_fhir(&self) -> Option<&Split> {
        self.split_fhir.as_ref()
    }

    /// Returns the split that creates openEHR elements.
    #[must_use]
    pub const fn split_openehr(&self) -> Option<&Split> {
        self.split_openehr.as_ref()
    }
}
