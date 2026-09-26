// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a run asks of its caller: the functions a `mappingCode` names, the
//! resources a reference points at, and the identity of a resource the run
//! creates.
//!
//! The engine makes no call of its own. Going into openEHR, a `reference`
//! mapping maps the referenced resource, which "should" be fetched when it is
//! not in the Bundle, and "if not possible, the engine proceeds with the
//! mapping" (`docs/specs/fhirconnect/modules/ROOT/pages/engine/references.adoc`),
//! so the fetch is a seam and a reference it cannot resolve is a declared
//! skip. Coming out of openEHR, `reference` and `hierarchy.split` create
//! resources, and no specification says which id a created resource takes, so
//! that is a seam too; the default derives it from the request. No
//! specification governs the shape of either seam: our own design.

use core::error::Error;
use core::fmt;

use fhir_types::codec::Value;
use sha2::Digest;

use crate::engine::traverse::functions::MappingFunctions;
use crate::engine::traverse::functions::NoMappingFunctions;
use crate::resolve::program::binding::ResourceType;

/// Where the resource a FHIR reference points at comes from.
pub trait ReferenceSource: fmt::Debug {
    /// Returns the resource `reference` points at, `None` when the source
    /// holds none.
    ///
    /// # Errors
    ///
    /// Returns [`ReferenceError`] when the source could not answer, which is
    /// a failure and never an absence.
    fn fetch(
        &self,
        reference: &str,
        expected: &ResourceType,
    ) -> Result<Option<Value>, ReferenceError>;
}

/// The source that holds no resource, for a run that resolves no reference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoReferences;

impl ReferenceSource for NoReferences {
    fn fetch(
        &self,
        _reference: &str,
        _expected: &ResourceType,
    ) -> Result<Option<Value>, ReferenceError> {
        Ok(None)
    }
}

/// The entries of one Bundle, looked up before a further source.
///
/// A reference names an entry by its `fullUrl` or, relative to the server, as
/// `<type>/<id>` (<https://hl7.org/fhir/R4/references.html#literal> and
/// <https://hl7.org/fhir/R4/bundle.html#references>).
#[derive(Debug)]
pub struct BundleReferences<'source> {
    entries: Vec<(Option<String>, Value)>,
    further: &'source dyn ReferenceSource,
}

impl<'source> BundleReferences<'source> {
    /// Creates a source over the entries of one Bundle, each with its
    /// `fullUrl` when it carries one, asking `further` for what the Bundle
    /// does not hold.
    #[must_use]
    pub fn new(
        entries: Vec<(Option<String>, Value)>,
        further: &'source dyn ReferenceSource,
    ) -> Self {
        Self { entries, further }
    }
}

impl ReferenceSource for BundleReferences<'_> {
    fn fetch(
        &self,
        reference: &str,
        expected: &ResourceType,
    ) -> Result<Option<Value>, ReferenceError> {
        let found = self.entries.iter().find(|&(full_url, resource)| {
            full_url.as_deref() == Some(reference)
                || relative(resource).as_deref() == Some(reference)
        });
        match found {
            Some((_, resource)) => Ok(Some(resource.clone())),
            None => self.further.fetch(reference, expected),
        }
    }
}

/// Returns the `<type>/<id>` a resource is referenced by, when it has an id.
fn relative(resource: &Value) -> Option<String> {
    let kind = resource.get("resourceType").and_then(Value::as_str)?;
    let id = resource.get("id").and_then(Value::as_str)?;
    Some(format!("{kind}/{id}"))
}

/// What assigns the id of a resource a run creates.
pub trait IdentitySink: fmt::Debug {
    /// Returns the FHIR id the resource `request` describes takes.
    ///
    /// # Errors
    ///
    /// Returns [`ReferenceError`] when no id can be assigned.
    fn identify(&self, request: &IdentityRequest) -> Result<String, ReferenceError>;
}

/// Everything a run knows about one resource it creates.
///
/// The same composition and the same program produce the same request, so a
/// sink that derives the id from it answers the same id on every run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityRequest {
    resource_type: String,
    parent_type: String,
    parent_id: Option<String>,
    composition: Option<String>,
    mapping: String,
    occurrence: Vec<u32>,
    unique: Vec<String>,
}

impl IdentityRequest {
    /// Creates the request for a `resource_type` a mapping creates under a
    /// resource of `parent_type`.
    #[must_use]
    pub fn new(
        resource_type: impl Into<String>,
        parent_type: impl Into<String>,
        mapping: impl Into<String>,
    ) -> Self {
        Self {
            resource_type: resource_type.into(),
            parent_type: parent_type.into(),
            parent_id: None,
            composition: None,
            mapping: mapping.into(),
            occurrence: Vec::new(),
            unique: Vec::new(),
        }
    }

    /// Returns this request with the parent's id, when the run knows it.
    #[must_use]
    pub fn with_parent_id(mut self, id: impl Into<String>) -> Self {
        self.parent_id = Some(id.into());
        self
    }

    /// Returns this request with the `LOCATABLE.uid` of the composition the
    /// run reads.
    #[must_use]
    pub fn with_composition(mut self, uid: impl Into<String>) -> Self {
        self.composition = Some(uid.into());
        self
    }

    /// Returns this request with the openEHR positions of the occurrence the
    /// resource is created for, outermost first.
    #[must_use]
    pub fn with_occurrence(mut self, occurrence: Vec<u32>) -> Self {
        self.occurrence = occurrence;
        self
    }

    /// Returns this request with the `unique` values of a split.
    #[must_use]
    pub fn with_unique(mut self, unique: Vec<String>) -> Self {
        self.unique = unique;
        self
    }

    /// Returns the type of the resource created.
    #[must_use]
    pub fn resource_type(&self) -> &str {
        &self.resource_type
    }

    /// Returns the type of the resource it is created under.
    #[must_use]
    pub fn parent_type(&self) -> &str {
        &self.parent_type
    }

    /// Returns the id of the resource it is created under, when known.
    #[must_use]
    pub fn parent_id(&self) -> Option<&str> {
        self.parent_id.as_deref()
    }

    /// Returns the `LOCATABLE.uid` of the composition read, when it has one.
    #[must_use]
    pub fn composition(&self) -> Option<&str> {
        self.composition.as_deref()
    }

    /// Returns the dotted name of the mapping or the context that creates it.
    #[must_use]
    pub fn mapping(&self) -> &str {
        &self.mapping
    }

    /// Returns the openEHR positions of the occurrence, outermost first.
    #[must_use]
    pub fn occurrence(&self) -> &[u32] {
        &self.occurrence
    }

    /// Returns the `unique` values of a split, in declaration order.
    #[must_use]
    pub fn unique(&self) -> &[String] {
        &self.unique
    }
}

/// The sink that derives an id from the request by a digest.
///
/// Every field is framed with its byte length before it is hashed, so two
/// different requests never frame to the same bytes, and the SHA-256 digest
/// renders as 64 lowercase hexadecimal characters, inside the R4 id grammar
/// `[A-Za-z0-9\-\.]{1,64}` (<https://hl7.org/fhir/R4/datatypes.html#id>).
/// No specification governs the derivation: our own design.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DerivedIdentity;

impl IdentitySink for DerivedIdentity {
    fn identify(&self, request: &IdentityRequest) -> Result<String, ReferenceError> {
        let mut hasher = sha2::Sha256::new();
        let mut frame = |field: &[u8]| {
            hasher.update(u64::try_from(field.len()).unwrap_or(u64::MAX).to_be_bytes());
            hasher.update(field);
        };
        frame(request.resource_type.as_bytes());
        frame(request.parent_type.as_bytes());
        frame(request.parent_id.as_deref().unwrap_or_default().as_bytes());
        frame(
            request
                .composition
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        frame(request.mapping.as_bytes());
        for position in &request.occurrence {
            frame(&position.to_be_bytes());
        }
        frame(b"unique");
        for value in &request.unique {
            frame(value.as_bytes());
        }
        let mut id = String::with_capacity(64);
        for byte in hasher.finalize().as_slice() {
            for nibble in [byte >> 4, byte & 0x0f] {
                id.extend(char::from_digit(u32::from(nibble), 16));
            }
        }
        Ok(id)
    }
}

/// Why a reference or an identity could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum ReferenceError {
    /// The source failed while fetching the reference.
    #[error("the resource {reference} could not be fetched")]
    Fetch {
        /// The reference the source was asked for.
        reference: String,
        /// The failure the source reported.
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// The reference points at a resource of another type.
    #[error("{reference} resolves to a {found}, and the mapping reads a {expected}")]
    WrongType {
        /// The reference that was resolved.
        reference: String,
        /// The resource type the mapping names.
        expected: String,
        /// The resource type the source answered with.
        found: String,
    },
    /// The sink could not assign an id.
    #[error("no id could be assigned to the {resource_type} the run creates")]
    Identity {
        /// The type of the resource the run creates.
        resource_type: String,
        /// The failure the sink reported.
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

/// Everything a run calls out to, in one place.
///
/// [`Seams::default`] registers no function, resolves no reference and
/// derives every created id with [`DerivedIdentity`].
#[derive(Debug, Clone, Copy)]
pub struct Seams<'seam> {
    functions: &'seam dyn MappingFunctions,
    references: &'seam dyn ReferenceSource,
    identities: &'seam dyn IdentitySink,
}

impl Default for Seams<'_> {
    fn default() -> Self {
        Self {
            functions: &NoMappingFunctions,
            references: &NoReferences,
            identities: &DerivedIdentity,
        }
    }
}

impl<'seam> Seams<'seam> {
    /// Returns these seams with `functions` as the `mappingCode` registry.
    #[must_use]
    pub fn with_functions(mut self, functions: &'seam dyn MappingFunctions) -> Self {
        self.functions = functions;
        self
    }

    /// Returns these seams with `references` as the reference source.
    #[must_use]
    pub fn with_references(mut self, references: &'seam dyn ReferenceSource) -> Self {
        self.references = references;
        self
    }

    /// Returns these seams with `identities` as the identity sink.
    #[must_use]
    pub fn with_identities(mut self, identities: &'seam dyn IdentitySink) -> Self {
        self.identities = identities;
        self
    }

    /// Returns the `mappingCode` registry.
    #[must_use]
    pub fn functions(&self) -> &'seam dyn MappingFunctions {
        self.functions
    }

    /// Returns the reference source.
    #[must_use]
    pub fn references(&self) -> &'seam dyn ReferenceSource {
        self.references
    }

    /// Returns the identity sink.
    #[must_use]
    pub fn identities(&self) -> &'seam dyn IdentitySink {
        self.identities
    }
}

#[cfg(test)]
mod tests {
    use super::BundleReferences;
    use super::DerivedIdentity;
    use super::IdentityRequest;
    use super::IdentitySink;
    use super::NoReferences;
    use super::ReferenceSource;
    use crate::resolve::program::binding::ResourceType;
    use fhir_types::codec::Value;

    #[test]
    fn a_derived_id_is_stable_and_inside_the_id_grammar() {
        let request = IdentityRequest::new("Condition", "Condition", "context")
            .with_occurrence(vec![2])
            .with_unique(vec![String::from("left")]);
        let first = DerivedIdentity
            .identify(&request)
            .expect("a digest is total");
        let second = DerivedIdentity
            .identify(&request)
            .expect("a digest is total");
        assert_eq!(first, second, "the same request derives the same id");
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn two_occurrences_derive_two_ids() {
        let at = |position: u32| {
            DerivedIdentity
                .identify(
                    &IdentityRequest::new("Condition", "Condition", "context")
                        .with_occurrence(vec![position]),
                )
                .expect("a digest is total")
        };
        assert_ne!(at(1), at(2));
    }

    #[test]
    fn a_bundle_entry_answers_by_full_url_and_by_type_and_id() {
        let specimen = Value::from_serde_json(serde_json::json!({
            "resourceType": "Specimen",
            "id": "synthetic-specimen-1"
        }));
        let source = BundleReferences::new(
            vec![(Some(String::from("urn:uuid:5e3c")), specimen.clone())],
            &NoReferences,
        );
        let expected = ResourceType::new("Specimen");
        assert_eq!(
            source
                .fetch("urn:uuid:5e3c", &expected)
                .expect("no failure"),
            Some(specimen.clone())
        );
        assert_eq!(
            source
                .fetch("Specimen/synthetic-specimen-1", &expected)
                .expect("no failure"),
            Some(specimen)
        );
        assert_eq!(
            source
                .fetch("Specimen/another", &expected)
                .expect("no failure"),
            None
        );
    }
}
