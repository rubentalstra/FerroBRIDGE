// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Where the content of a run into openEHR came from.
//!
//! The openEHR reference model records the origin of transformed content in
//! `FEEDER_AUDIT`, which "defines the semantics of an audit trail which is
//! constructed to describe the origin of data that have been transformed into
//! openEHR form and committed to the system"
//! (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_feeder_audit_class>).
//! A run carries one [`Origin`] on its [`crate::engine::traverse::Defaults`],
//! and the engine writes it into the composition beside the fields it
//! defaulted, so a reader of the stored composition sees both.

use fhir_types::codec::Value;

/// What a source resource that carries no `id` is recorded as.
///
/// An absent identifier is recorded as unknown and never invented. No
/// specification governs the spelling: our own design.
pub const UNKNOWN_SOURCE: &str = "unknown";

/// The system a run's content passed through, and the resource it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    system_id: String,
    source: Option<SourceItem>,
}

impl Origin {
    /// Creates the origin of a run through the system `system_id` names.
    ///
    /// `system_id` becomes `FEEDER_AUDIT_DETAILS.system_id`, the "identifier
    /// of the system which handled the information item"
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_feeder_audit_details_class>).
    #[must_use]
    pub fn new(system_id: impl Into<String>) -> Self {
        Self {
            system_id: system_id.into(),
            source: None,
        }
    }

    /// Returns this origin with `source` as the resource the content came
    /// from.
    #[must_use]
    pub fn with_source(mut self, source: SourceItem) -> Self {
        self.source = Some(source);
        self
    }

    /// Returns the system the content passed through.
    #[must_use]
    pub fn system_id(&self) -> &str {
        &self.system_id
    }

    /// Returns the resource the content came from, when the caller named one.
    #[must_use]
    pub const fn source(&self) -> Option<&SourceItem> {
        self.source.as_ref()
    }
}

/// The FHIR resource one run into openEHR read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceItem {
    resource_type: String,
    id: Option<String>,
    version_id: Option<String>,
}

impl SourceItem {
    /// Creates a source of `resource_type` with no id and no version.
    #[must_use]
    pub fn new(resource_type: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            id: None,
            version_id: None,
        }
    }

    /// Reads the source item a FHIR resource describes.
    ///
    /// The type is `resourceType`, the id is `id` and the version is
    /// `meta.versionId` (<https://hl7.org/fhir/R4/resource.html>). A document
    /// with no `resourceType` is no resource and answers `None`.
    #[must_use]
    pub fn of(document: &Value) -> Option<Self> {
        let resource_type = document.get("resourceType").and_then(Value::as_str)?;
        let mut source = Self::new(resource_type);
        if let Some(id) = document.get("id").and_then(Value::as_str) {
            source = source.with_id(id);
        }
        if let Some(version) = document
            .get("meta")
            .and_then(|meta| meta.get("versionId"))
            .and_then(Value::as_str)
        {
            source = source.with_version_id(version);
        }
        Some(source)
    }

    /// Returns this source with the resource id `id`.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Returns this source with the resource version `version_id`.
    #[must_use]
    pub fn with_version_id(mut self, version_id: impl Into<String>) -> Self {
        self.version_id = Some(version_id.into());
        self
    }

    /// Returns the resource type.
    #[must_use]
    pub fn resource_type(&self) -> &str {
        &self.resource_type
    }

    /// Returns the resource id, when the resource carries one.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Returns the resource version, when the resource carries one.
    #[must_use]
    pub fn version_id(&self) -> Option<&str> {
        self.version_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::SourceItem;
    use fhir_types::codec::Value;

    #[test]
    fn a_source_item_reads_the_type_the_id_and_the_version() {
        let document = Value::from_serde_json(serde_json::json!({
            "resourceType": "Condition",
            "id": "sender-1",
            "meta": {"versionId": "3"}
        }));
        let source = SourceItem::of(&document).expect("a resource names its type");
        assert_eq!(source.resource_type(), "Condition");
        assert_eq!(source.id(), Some("sender-1"));
        assert_eq!(source.version_id(), Some("3"));
    }

    #[test]
    fn a_document_with_no_resource_type_is_no_source() {
        let document = Value::from_serde_json(serde_json::json!({"id": "x"}));
        assert_eq!(SourceItem::of(&document), None);
    }
}
