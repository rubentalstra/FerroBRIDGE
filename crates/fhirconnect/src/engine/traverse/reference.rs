// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `reference` mappings: the resource resolved going in and created going
//! out, and the id every created resource takes.

use fhir_types::codec::Object;
use fhir_types::codec::Value;

use crate::engine::fhir::FhirKind;
use crate::engine::outcome::SkipReason;
use crate::engine::outcome::Warning;
use crate::engine::seam::IdentityRequest;
use crate::engine::seam::ReferenceError;
use crate::model::ast::keyword::Direction;
use crate::resolve::program::binding::ResourceType;
use crate::resolve::program::mapping::Mapping;
use crate::tree::Occurrence;

use crate::tree::element::Table;
use crate::tree::read::read;
use crate::tree::write::write;

use crate::engine::traverse::Binding;
use crate::engine::traverse::Run;
use crate::engine::traverse::empty_resource;
use crate::engine::traverse::error::EngineError;

impl<T: Table + ?Sized> Run<'_, T> {
    /// Returns the identity request of a resource this run creates.
    pub(super) fn identity_request(&self, resource_type: &str, mapping: &str) -> IdentityRequest {
        let mut request =
            IdentityRequest::new(resource_type, self.program.resource().as_str(), mapping);
        if let Some(id) = self.fhir.get("id").and_then(Value::as_str) {
            request = request.with_parent_id(id);
        }
        if let Some(uid) = self
            .composition
            .and_then(|composition| composition.value().get("uid"))
            .and_then(|uid| uid.get("value"))
            .and_then(serde_json::Value::as_str)
        {
            request = request.with_composition(uid);
        }
        request
    }

    /// Asks the identity sink for a created resource's id and sets it.
    pub(super) fn identify(
        &self,
        mapping: &str,
        created: &mut Value,
        request: &IdentityRequest,
    ) -> Result<String, EngineError> {
        let id = self
            .seams
            .identities()
            .identify(request)
            .map_err(|source| EngineError::Reference {
                mapping: String::from(mapping),
                source: Box::new(source),
            })?;
        if let Value::Object(ref mut object) = *created {
            object.insert(String::from("id"), Value::String(id.clone()));
        }
        Ok(id)
    }

    /// Runs a `reference` mapping.
    ///
    /// "It allows us to initialize a new resource in FHIR (or the other way
    /// around) and reference it inside the resource we are currently mapping"
    /// (`types-of-mappings/concept-type/Reference.adoc`). Going into openEHR
    /// the referenced resource is found in the current document's
    /// `contained`, then through the reference source, and its mappings run
    /// over it; going out of openEHR a new resource is created, its mappings
    /// write it, and the reference to it is written in the current one.
    pub(super) fn reference(
        &mut self,
        mapping: &Mapping,
        parent: &Binding,
        inputs: &[Occurrence],
        resource: &ResourceType,
        mappings: &[Mapping],
    ) -> Result<Vec<Binding>, EngineError> {
        match self.direction {
            Direction::FhirToOpenehr => {
                let mut bindings = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let binding = Binding {
                        fhir: input.clone(),
                        openehr: parent.openehr.clone(),
                    };
                    self.enter_reference(mapping, input, resource, mappings, &binding)?;
                    bindings.push(binding);
                }
                Ok(bindings)
            }
            Direction::OpenehrToFhir => {
                let bindings = self.bindings(mapping, parent, inputs)?;
                for binding in &bindings {
                    self.create_reference(mapping, resource, mappings, binding)?;
                }
                Ok(bindings)
            }
        }
    }

    /// Resolves one reference occurrence and runs the mappings over it.
    fn enter_reference(
        &mut self,
        mapping: &Mapping,
        input: &Occurrence,
        resource: &ResourceType,
        mappings: &[Mapping],
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let Some(literal) = self.reference_at(mapping, input)? else {
            self.warnings.push(Warning::Skipped {
                mapping: String::from(mapping.name()),
                reason: SkipReason::EmptyReference,
            });
            return Ok(());
        };
        if self.references.contains(&literal) {
            return Err(EngineError::ReferenceCycle {
                mapping: String::from(mapping.name()),
                reference: literal,
            });
        }
        let Some(fetched) = self.resolve_reference(mapping, &literal, resource)? else {
            self.warnings.push(Warning::Skipped {
                mapping: String::from(mapping.name()),
                reason: SkipReason::UnresolvedReference { reference: literal },
            });
            return Ok(());
        };
        let outer = core::mem::replace(&mut self.fhir, fetched);
        let held_pin = self.fhir_pin.take();
        self.references.push(literal);
        let inner = Binding {
            fhir: Occurrence::default(),
            openehr: binding.openehr.clone(),
        };
        let ran = self.mappings(mappings, &inner);
        self.references.pop();
        self.fhir_pin = held_pin;
        self.fhir = outer;
        ran
    }

    /// Returns the resource a literal reference points at, checked against
    /// the type the mapping names.
    ///
    /// A `#id` reference names a resource in the current document's
    /// `contained` (<https://hl7.org/fhir/R4/references.html#contained>); any
    /// other goes to the reference source.
    fn resolve_reference(
        &self,
        mapping: &Mapping,
        literal: &str,
        resource: &ResourceType,
    ) -> Result<Option<Value>, EngineError> {
        let refuse = |source: ReferenceError| EngineError::Reference {
            mapping: String::from(mapping.name()),
            source: Box::new(source),
        };
        let found = if let Some(local) = literal.strip_prefix('#') {
            self.fhir
                .get("contained")
                .and_then(Value::as_array)
                .and_then(|contained| {
                    contained
                        .iter()
                        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(local))
                })
                .cloned()
        } else {
            self.seams
                .references()
                .fetch(literal, resource)
                .map_err(refuse)?
        };
        let Some(found) = found else {
            return Ok(None);
        };
        let kind = found
            .get("resourceType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind != resource.as_str() {
            return Err(refuse(ReferenceError::WrongType {
                reference: String::from(literal),
                expected: String::from(resource.as_str()),
                found: String::from(kind),
            }));
        }
        Ok(Some(found))
    }

    /// Returns the literal reference the mapping's FHIR side holds at one
    /// occurrence, `None` when it holds none.
    ///
    /// The side names either a `Reference` or its `reference` string, as the
    /// specification's example writes it (`Reference.adoc`).
    pub(super) fn reference_at(
        &self,
        mapping: &Mapping,
        input: &Occurrence,
    ) -> Result<Option<String>, EngineError> {
        let Some(value) = self.fhir_value_at(mapping, input)? else {
            return Ok(None);
        };
        Ok(match value {
            Value::String(text) => Some(text),
            Value::Object(ref object) => object
                .get("reference")
                .and_then(Value::as_str)
                .map(String::from),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
        })
    }

    /// Returns the raw FHIR value the mapping's FHIR side holds at one
    /// occurrence.
    pub(super) fn fhir_value_at(
        &self,
        mapping: &Mapping,
        input: &Occurrence,
    ) -> Result<Option<Value>, EngineError> {
        let Some(target) = mapping.fhir() else {
            return Ok(None);
        };
        let matches = read(self.table, &self.fhir, target.expression()).map_err(|source| {
            EngineError::Read {
                mapping: String::from(mapping.name()),
                expression: String::from(target.expression().as_str()),
                source: Box::new(source),
            }
        })?;
        Ok(matches
            .iter()
            .find(|matched| matched.occurrence() == input)
            .and_then(crate::tree::read::Match::value)
            .cloned())
    }

    /// Creates one referenced resource, and references it from the current
    /// one.
    fn create_reference(
        &mut self,
        mapping: &Mapping,
        resource: &ResourceType,
        mappings: &[Mapping],
        binding: &Binding,
    ) -> Result<(), EngineError> {
        let outer = core::mem::replace(&mut self.fhir, empty_resource(resource.as_str()));
        let counters = core::mem::take(&mut self.counters);
        let inner = Binding {
            fhir: Occurrence::default(),
            openehr: binding.openehr.clone(),
        };
        let ran = self.mappings(mappings, &inner);
        self.counters = counters;
        let mut created = core::mem::replace(&mut self.fhir, outer);
        ran?;
        // NOTE: no specification governs this: our own design, a resource its
        // mappings wrote nothing into carries no content, so no resource and no
        // reference to it is created.
        if created.as_object().is_none_or(|object| object.len() <= 1) {
            return Ok(());
        }
        let request = self
            .identity_request(resource.as_str(), mapping.name())
            .with_occurrence(
                binding
                    .openehr
                    .iter()
                    .map(|position| position.get())
                    .collect(),
            );
        let id = self.identify(mapping.name(), &mut created, &request)?;
        self.created.push(created);
        self.write_reference(
            mapping,
            binding,
            &format!("{}/{id}", resource.as_str()),
            None,
        )
    }

    /// Writes one literal reference at the mapping's FHIR side.
    ///
    /// A side that names a `Reference` takes the element, one that names its
    /// `reference` string takes the string.
    pub(super) fn write_reference(
        &mut self,
        mapping: &Mapping,
        binding: &Binding,
        literal: &str,
        display: Option<&str>,
    ) -> Result<(), EngineError> {
        let Some(target) = mapping.fhir() else {
            return Ok(());
        };
        let element = String::from(target.resolved().leaf());
        let value = if FhirKind::at(target.resolved().location()) == Some(FhirKind::Reference) {
            let mut object = Object::new();
            object.insert(
                String::from("reference"),
                Value::String(String::from(literal)),
            );
            if let Some(display) = display {
                object.insert(
                    String::from("display"),
                    Value::String(String::from(display)),
                );
            }
            Value::Object(object)
        } else {
            Value::String(String::from(literal))
        };
        write(
            self.table,
            &mut self.fhir,
            target.expression(),
            &binding.fhir,
            value,
        )
        .map_err(|source| EngineError::Write {
            mapping: String::from(mapping.name()),
            element,
            source: Box::new(source),
        })
    }
}
