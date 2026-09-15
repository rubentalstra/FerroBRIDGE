// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The `Provenance` every `$tofhir` run carries.
//!
//! "The engine generates a `Provenance` resource as a side-effect of every
//! run, describing the transformation itself (what was produced, from what,
//! when, and by whom), and includes it in the Bundle" (`engine/rest-api.adoc`
//! §$tofhir Output). The `context` fields `who` and `onBehalfOf` feed it, and
//! "when they are omitted the engine supplies its own defaults".
//!
//! The FHIR shape is the R4 one (<https://hl7.org/fhir/R4/provenance.html>):
//! `target` is what the run produced, `recorded` is when, `agent.who` is by
//! whom, and `entity` with `role = derivation` is what it came from. The
//! composition is not a FHIR resource, so the entity names it by identifier
//! rather than by a literal reference; no specification governs that choice:
//! our own design.

use fhir_types::r4::identifier::Identifier;
use fhir_types::r4::primitives::Code;
use fhir_types::r4::primitives::Instant;
use fhir_types::r4::provenance::Provenance;
use fhir_types::r4::provenance::ProvenanceAgent;
use fhir_types::r4::provenance::ProvenanceEntity;
use fhir_types::r4::reference::Reference;

use crate::engine::context::CallContext;

/// The `ProvenanceEntityRole` code a transformation carries.
///
/// "derivation: A transformation of an entity into another, an update of an
/// entity resulting in a new one, or the construction of a new entity based on
/// a pre-existing entity" (<https://hl7.org/fhir/R4/valueset-provenance-entity-role.html>).
pub const DERIVATION: &str = "derivation";

/// Builds the `Provenance` of one run.
///
/// `targets` are the literal references of every resource the run mapped,
/// `recorded` is the run time as an `instant`, `device` is the engine's own
/// `Device` reference, and `composition` names the openEHR composition the
/// run read, with `uid` when it carried one.
#[must_use]
pub fn of_run(
    targets: &[String],
    recorded: &str,
    device: &str,
    composition: &CompositionSource,
    context: Option<&CallContext>,
) -> Provenance {
    let who = context
        .and_then(CallContext::who)
        .cloned()
        .unwrap_or_else(|| literal(device));
    Provenance {
        target: targets.iter().map(|target| literal(target)).collect(),
        recorded: Instant::from(recorded),
        agent: vec![ProvenanceAgent {
            who,
            on_behalf_of: context.and_then(CallContext::on_behalf_of).cloned(),
            ..ProvenanceAgent::default()
        }],
        entity: vec![ProvenanceEntity {
            role: Code::from(DERIVATION),
            what: composition.reference(),
            ..ProvenanceEntity::default()
        }],
        ..Provenance::default()
    }
}

/// The openEHR composition a run read, as the `entity` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionSource {
    template_id: String,
    uid: Option<String>,
}

impl CompositionSource {
    /// Names the composition built against `template_id`.
    #[must_use]
    pub fn new(template_id: impl Into<String>) -> Self {
        Self {
            template_id: template_id.into(),
            uid: None,
        }
    }

    /// Returns this source with the composition's `uid`.
    ///
    /// The value is the `OBJECT_VERSION_ID` the composition carries, so the
    /// entity names the composition VERSION the run read.
    #[must_use]
    pub fn with_uid(mut self, uid: impl Into<String>) -> Self {
        self.uid = Some(uid.into());
        self
    }

    /// Returns the composition `uid`, when it carried one.
    #[must_use]
    pub fn uid(&self) -> Option<&str> {
        self.uid.as_deref()
    }

    /// Returns the reference the `entity` carries.
    #[must_use]
    fn reference(&self) -> Reference {
        Reference {
            identifier: self.uid.as_ref().map(|uid| {
                Box::new(Identifier {
                    value: Some(fhir_types::r4::primitives::String::from(uid.as_str())),
                    ..Identifier::default()
                })
            }),
            display: Some(fhir_types::r4::primitives::String::from(
                self.template_id.as_str(),
            )),
            ..Reference::default()
        }
    }
}

/// Returns a `Reference` carrying the literal reference `text`.
fn literal(text: &str) -> Reference {
    Reference {
        reference: Some(fhir_types::r4::primitives::String::from(text)),
        ..Reference::default()
    }
}

#[cfg(test)]
mod tests {
    use super::CompositionSource;
    use super::of_run;
    use crate::engine::context::CallContext;
    use fhir_types::r4::reference::Reference;

    /// Returns a `Reference` whose literal reference is `text`.
    fn reference(text: &str) -> Reference {
        Reference {
            reference: Some(fhir_types::r4::primitives::String::from(text)),
            ..Reference::default()
        }
    }

    #[test]
    fn the_agent_defaults_to_the_engines_own_device() {
        let source = CompositionSource::new("ferrobridge.diagnose.v1");
        let provenance = of_run(
            &[String::from("Condition/one")],
            "2026-09-15T10:00:00Z",
            "Device/ferrobridge",
            &source,
            None,
        );
        let who = provenance
            .agent
            .first()
            .and_then(|agent| agent.who.reference.as_ref())
            .and_then(|value| value.value.clone());
        assert_eq!(who, Some(String::from("Device/ferrobridge")));
    }

    #[test]
    fn the_call_context_overrides_the_agent_and_names_the_institution() {
        let source = CompositionSource::new("ferrobridge.diagnose.v1");
        let context = CallContext::new()
            .with_who(reference("Practitioner/456"))
            .with_on_behalf_of(reference("Organization/charite"));
        let provenance = of_run(
            &[String::from("Condition/one")],
            "2026-09-15T10:00:00Z",
            "Device/ferrobridge",
            &source,
            Some(&context),
        );
        let agent = provenance.agent.first().expect("one agent");
        assert_eq!(
            agent.who.reference.as_ref().and_then(|v| v.value.clone()),
            Some(String::from("Practitioner/456"))
        );
        assert_eq!(
            agent
                .on_behalf_of
                .as_ref()
                .and_then(|value| value.reference.as_ref())
                .and_then(|value| value.value.clone()),
            Some(String::from("Organization/charite"))
        );
    }

    #[test]
    fn the_entity_is_a_derivation_naming_the_composition_uid() {
        let source = CompositionSource::new("ferrobridge.diagnose.v1")
            .with_uid("8849182c-82ad-4088-a07f-48ead4180515::ferrobridge::1");
        let provenance = of_run(
            &[String::from("Condition/one")],
            "2026-09-15T10:00:00Z",
            "Device/ferrobridge",
            &source,
            None,
        );
        let entity = provenance.entity.first().expect("one entity");
        assert_eq!(entity.role.value.as_deref(), Some("derivation"));
        assert_eq!(
            entity
                .what
                .identifier
                .as_ref()
                .and_then(|identifier| identifier.value.as_ref())
                .and_then(|value| value.value.clone()),
            Some(String::from(
                "8849182c-82ad-4088-a07f-48ead4180515::ferrobridge::1"
            ))
        );
    }
}
