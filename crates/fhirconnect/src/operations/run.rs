// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Running the engine behind each operation.
//!
//! Both operations are pure transformations that "change no server state", so
//! everything a run needs arrives with the request: the compiled set, the
//! element table, the mapping-function registry and the per-run [`Settings`].
//! Neither reaches a CDR.
//!
//! FerroBRIDGE pins strictness where the draft leaves room: a refusal answers
//! an `OperationOutcome` and nothing else, so a caller can never mistake a
//! partial result for a complete one. What a successful run declares as lost
//! travels as `information` and `warning` issues beside the result.

use fhir_types::codec::Json;
use fhir_types::codec::Object;
use fhir_types::codec::Path;
use fhir_types::codec::Value;
use fhir_types::r4::bundle::Bundle;
use fhir_types::r4::bundle::BundleEntry;
use fhir_types::r4::primitives::Code;
use fhir_types::r4::resource::Resource;
use openehr_mapping_core::composition::CanonicalComposition;
use openehr_mapping_core::index::WebTemplateIndex;

use crate::engine::context::CallContext;
use crate::engine::origin::Origin;
use crate::engine::origin::SourceItem;
use crate::engine::seam::BundleReferences;
use crate::engine::seam::NoReferences;
use crate::engine::seam::Seams;
use crate::engine::traverse;
use crate::engine::traverse::Defaults;
use crate::engine::traverse::MappingFunctions;
use crate::operations::contract::CompositionPayload;
use crate::operations::contract::Format;
use crate::operations::contract::ToFhirRequest;
use crate::operations::contract::ToFhirResponse;
use crate::operations::contract::ToOpenehrRequest;
use crate::operations::contract::ToOpenehrResponse;
use crate::operations::error::OperationError;
use crate::operations::issues;
use crate::operations::programs::ProgramSet;
use crate::operations::provenance;
use crate::operations::provenance::CompositionSource;
use crate::resolve::program::Program;
use crate::resolve::program::TemplateId;
use crate::resolve::select::select_by_profile_pinned;
use crate::resolve::select::select_by_template;
use crate::tree::element::Table;

/// The Bundle type `$tofhir` answers with.
///
/// The draft leaves the type open ("not constrained by this specification; an
/// engine MAY select it according to how the resulting resources are used"),
/// and a `$tofhir` answer is a set of resources with no search and no
/// transaction semantics, which is what `collection` is for
/// (<https://hl7.org/fhir/R4/valueset-bundle-type.html>).
pub const BUNDLE_TYPE: &str = "collection";

/// What one run needs beyond the request and the compiled set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    device: String,
    now: String,
    defaults: Defaults,
}

impl Settings {
    /// Creates the settings of one run.
    ///
    /// `device` is the engine's own `Device` reference, the `Provenance`
    /// `agent.who` a call that supplies no `context.who` gets and the
    /// `FEEDER_AUDIT` system a `$toopenehr` composition records. `now` is the
    /// caller's current timestamp: it is the `Provenance.recorded` instant and
    /// the composition context start time the engine defaults to. This crate
    /// reads no clock.
    #[must_use]
    pub fn new(device: impl Into<String>, now: impl Into<String>) -> Self {
        let now = now.into();
        Self {
            device: device.into(),
            defaults: Defaults::at(now.clone()),
            now,
        }
    }

    /// Returns these settings with `defaults` as the composition defaults.
    #[must_use]
    pub fn with_defaults(mut self, defaults: Defaults) -> Self {
        self.defaults = defaults;
        self
    }

    /// Returns the engine's own `Device` reference.
    #[must_use]
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Returns the run timestamp.
    #[must_use]
    pub fn now(&self) -> &str {
        &self.now
    }

    /// Returns the composition defaults the inbound direction applies.
    #[must_use]
    pub const fn defaults(&self) -> &Defaults {
        &self.defaults
    }
}

/// Runs `$tofhir`: one openEHR composition to one FHIR Bundle.
///
/// The Bundle carries the mapped resources, exactly one `Provenance` covering
/// every one of them, and an `OperationOutcome` entry when the run declared a
/// loss.
///
/// # Errors
///
/// Returns [`OperationError`] for a payload that does not resolve to one
/// template, a template or program the set does not hold, an element the
/// mapping cannot carry, and a mapped resource with no identity for the
/// `Provenance` to target.
pub fn to_fhir<T: Table + ?Sized>(
    programs: &ProgramSet,
    table: &T,
    functions: &dyn MappingFunctions,
    settings: &Settings,
    request: &ToFhirRequest,
) -> Result<ToFhirResponse, OperationError> {
    let template = template_of(request)?;
    let index = programs
        .template(&template)
        .ok_or_else(|| OperationError::UnknownTemplate {
            template: String::from(template.as_str()),
        })?;
    let composition = composition_of(request.composition(), index, settings.now())?;
    let program = select_by_template(programs.programs(), &template)
        .map_err(|source| OperationError::Select { source })?;
    let context = request.context().cloned().unwrap_or_default();
    let seams = Seams::default().with_functions(functions);
    let outcome = traverse::to_fhir(program, table, index, &composition, &seams, &context)
        .map_err(|source| OperationError::Mapping {
            source: Box::new(source),
        })?;
    let resource_type = String::from(program.resource().as_str());
    let mut object = object_of(outcome.value())?;
    subject(table, &mut object, &resource_type, request.context())?;
    let id = identity(&object, &composition, &resource_type)?;
    object.insert(String::from("id"), Value::String(id.clone()));
    let mapped = typed(&object, &resource_type)?;
    let mut targets = vec![format!("{resource_type}/{id}")];
    let mut resources = vec![mapped];
    // NOTE: a created resource carries the id the identity sink gave it and is
    // referenced as `<type>/<id>`, so it travels as one more entry of the answer
    // (<https://hl7.org/fhir/R4/bundle.html#references>).
    for created in outcome.created() {
        let mut object = object_of(created)?;
        let kind = object
            .get("resourceType")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or(OperationError::NotAnObject {
                what: "created resource",
            })?;
        subject(table, &mut object, &kind, request.context())?;
        if let Some(created_id) = object.get("id").and_then(Value::as_str) {
            targets.push(format!("{kind}/{created_id}"));
        }
        resources.push(typed(&object, &kind)?);
    }
    let source = match composition_uid(&composition) {
        Some(uid) => CompositionSource::new(template.as_str()).with_uid(uid),
        None => CompositionSource::new(template.as_str()),
    };
    let provenance = provenance::of_run(
        &targets,
        settings.now(),
        settings.device(),
        &source,
        request.context(),
    );
    let mut entry: Vec<BundleEntry> = resources.into_iter().map(entry_of).collect();
    entry.push(entry_of(Resource::Provenance(Box::new(provenance))));
    if let Some(reported) = issues::of_warnings(outcome.warnings()) {
        entry.push(entry_of(Resource::OperationOutcome(Box::new(reported))));
    }
    Ok(ToFhirResponse::new(Bundle {
        r#type: Code::from(BUNDLE_TYPE),
        entry,
        ..Bundle::default()
    }))
}

/// Runs `$toopenehr`: one FHIR Bundle to one openEHR composition.
///
/// # Errors
///
/// Returns [`OperationError`] when the Bundle references more than one
/// subject, when no single program answers it, when it carries no single
/// resource of the mapped type, and for any element the mapping cannot carry.
pub fn to_openehr<T: Table + ?Sized>(
    programs: &ProgramSet,
    table: &T,
    functions: &dyn MappingFunctions,
    settings: &Settings,
    request: &ToOpenehrRequest,
) -> Result<ToOpenehrResponse, OperationError> {
    let entries = entry_objects(request.bundle())?;
    refuse_several_subjects(&entries)?;
    let program = select(programs, &entries, request.template_id())?;
    let template = program.template().id().clone();
    let index = programs
        .template(&template)
        .ok_or_else(|| OperationError::UnknownTemplate {
            template: String::from(template.as_str()),
        })?;
    let document = mapped_resource(&entries, program)?;
    let mut origin = Origin::new(settings.device());
    if let Some(source) = SourceItem::of(&document) {
        origin = origin.with_source(source);
    }
    let defaults = settings.defaults().clone().with_origin(origin);
    let references = BundleReferences::new(bundle_entries(request.bundle())?, &NoReferences);
    let seams = Seams::default()
        .with_functions(functions)
        .with_references(&references);
    let outcome = traverse::to_openehr(
        program,
        table,
        index,
        &document,
        &seams,
        &defaults,
        &CallContext::new(),
    )
    .map_err(|source| OperationError::Mapping {
        source: Box::new(source),
    })?;
    let serialized = serialize(index, outcome.value(), request.format())?;
    let answer = ToOpenehrResponse::new(serialized);
    Ok(match issues::of_warnings(outcome.warnings()) {
        Some(reported) => answer.with_outcome(reported),
        None => answer,
    })
}

/// Returns the template one `$tofhir` call resolves to.
///
/// The parameter wins when both name a template and they agree; a
/// disagreement is a refusal rather than a silent choice.
fn template_of(request: &ToFhirRequest) -> Result<TemplateId, OperationError> {
    let inline = request.composition().template_id();
    match (inline, request.template_id()) {
        (Some(inline), Some(pinned)) if inline.as_str() != pinned.as_str() => {
            Err(OperationError::TemplateDisagreement {
                payload: String::from(inline.as_str()),
                pinned: String::from(pinned.as_str()),
            })
        }
        (_, Some(pinned)) => Ok(pinned.clone()),
        (Some(inline), None) => Ok(inline),
        (None, None) => match *request.composition() {
            CompositionPayload::Flat(_) => Err(OperationError::FlatTemplate),
            CompositionPayload::Canonical(_) => Err(OperationError::CanonicalTemplate),
        },
    }
}

/// Returns the composition one `$tofhir` call carries, canonical either way.
fn composition_of(
    payload: &CompositionPayload,
    index: &WebTemplateIndex,
    now: &str,
) -> Result<CanonicalComposition, OperationError> {
    match *payload {
        CompositionPayload::Canonical(ref object) => index
            .accept(serde_json::Value::Object(object.clone()))
            .map_err(|source| OperationError::Serialization {
                source: Box::new(source),
            }),
        CompositionPayload::Flat(ref object) => {
            index
                .build_from_flat(object, now)
                .map_err(|source| OperationError::Serialization {
                    source: Box::new(source),
                })
        }
    }
}

/// Returns the `OBJECT_VERSION_ID` a composition carries, when it carries one.
fn composition_uid(composition: &CanonicalComposition) -> Option<&str> {
    composition.value().get("uid")?.get("value")?.as_str()
}

/// Returns the mapped document as an object, or refuses it.
fn object_of(document: &Value) -> Result<Object, OperationError> {
    match *document {
        Value::Object(ref object) => Ok(object.clone()),
        _ => Err(OperationError::NotAnObject {
            what: "mapped resource",
        }),
    }
}

/// Returns the FHIR id the mapped resource takes.
///
/// A mapping that wrote an `id` keeps it. Otherwise the id is the
/// `versioned_object_uid` of the composition, the leading segment of its
/// `OBJECT_VERSION_ID`, which is the one part stable across versions (openEHR
/// RM Common §`OBJECT_VERSION_ID`) and which fits the FHIR id grammar
/// `[A-Za-z0-9\-\.]{1,64}` (<https://hl7.org/fhir/R4/resource.html>).
// TODO(#85): the identity map assigns the id and keeps it stable across runs.
fn identity(
    object: &Object,
    composition: &CanonicalComposition,
    resource_type: &str,
) -> Result<String, OperationError> {
    if let Some(Value::String(written)) = object.get("id")
        && is_fhir_id(written)
    {
        return Ok(written.clone());
    }
    composition_uid(composition)
        .and_then(|uid| uid.split("::").next())
        .filter(|candidate| is_fhir_id(candidate))
        .map(String::from)
        .ok_or_else(|| OperationError::Unidentified {
            resource: String::from(resource_type),
        })
}

/// Returns whether `text` fits the R4 `id` grammar.
fn is_fhir_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '.'
        })
}

/// Writes `context.patient` over the subject the mapping produced.
///
/// "When the engine can resolve the subject itself and `context.patient` is
/// also supplied, the supplied value takes precedence" and "an engine MUST NOT
/// require `context.patient`" (`engine/rest-api.adoc` §Resolving the patient).
/// Which element carries the subject comes from the element table, never from
/// a list written here.
///
/// # Errors
///
/// Returns [`OperationError::Encode`] when the reference the caller supplied
/// does not render as JSON.
fn subject<T: Table + ?Sized>(
    table: &T,
    object: &mut Object,
    resource_type: &str,
    context: Option<&CallContext>,
) -> Result<(), OperationError> {
    let Some(patient) = context.and_then(CallContext::patient) else {
        return Ok(());
    };
    // NOTE: a resource whose type defines no `subject` or `patient` element
    // has nowhere to carry the override, which is a legitimate absence rather
    // than a defect (<https://hl7.org/fhir/R4/element.html>).
    let Some(schema) = table.type_named(resource_type) else {
        return Ok(());
    };
    let Some(field) = schema
        .fields
        .iter()
        .find(|field| field.name == "subject" || field.name == "patient")
    else {
        return Ok(());
    };
    let rendered = Json::to_json(patient).map_err(|source| OperationError::Encode { source })?;
    let value = if field.many {
        Value::Array(vec![Value::Object(rendered)])
    } else {
        Value::Object(rendered)
    };
    object.insert(String::from(field.name), value);
    Ok(())
}

/// Returns one Bundle entry carrying `resource`.
fn entry_of(resource: Resource) -> BundleEntry {
    BundleEntry {
        resource: Some(resource),
        ..BundleEntry::default()
    }
}

/// Returns a mapped resource object as the typed resource of `resource_type`.
fn typed(object: &Object, resource_type: &str) -> Result<Resource, OperationError> {
    Resource::from_json(object, &mut Path::root(resource_type)).map_err(|source| {
        OperationError::Mapped {
            version: "R4",
            source: Box::new(source),
        }
    })
}

/// Returns every entry resource of `bundle` with its `fullUrl`, as a
/// reference source reads them.
///
/// A reference that the Bundle does not resolve goes to no further source:
/// the operations "change no server state" and reach no server, so an
/// unresolved reference is a declared skip of the run.
fn bundle_entries(bundle: &Bundle) -> Result<Vec<(Option<String>, Value)>, OperationError> {
    let mut entries = Vec::new();
    for entry in &bundle.entry {
        let Some(ref resource) = entry.resource else {
            continue;
        };
        let object = Json::to_json(resource).map_err(|source| OperationError::Encode { source })?;
        let full_url = entry.full_url.as_ref().and_then(|url| url.value.clone());
        entries.push((full_url, Value::Object(object)));
    }
    Ok(entries)
}

/// Returns every entry resource of `bundle` as a JSON object.
///
/// The typed model is read back through the codec so the profile, the subject
/// and the resource type are read the same way for every resource type.
fn entry_objects(bundle: &Bundle) -> Result<Vec<Object>, OperationError> {
    let mut objects = Vec::new();
    for entry in &bundle.entry {
        let Some(ref resource) = entry.resource else {
            continue;
        };
        let object = Json::to_json(resource).map_err(|source| OperationError::Encode { source })?;
        objects.push(object);
    }
    Ok(objects)
}

/// Refuses a Bundle that references more than one subject.
///
/// One Bundle maps to one composition and a composition belongs to one EHR, so
/// a mixed-subject Bundle cannot be one composition. FHIRconnect governs no
/// part of this: our own design.
fn refuse_several_subjects(entries: &[Object]) -> Result<(), OperationError> {
    let mut subjects: Vec<String> = Vec::new();
    for object in entries {
        if let Some(Value::String(resource_type)) = object.get("resourceType")
            && resource_type == "Patient"
            && let Some(Value::String(id)) = object.get("id")
        {
            record(&mut subjects, format!("Patient/{id}"));
        }
        for key in ["subject", "patient"] {
            if let Some(reference) = object.get(key).and_then(|value| value.get("reference"))
                && let Value::String(ref text) = *reference
            {
                record(&mut subjects, text.clone());
            }
        }
    }
    if subjects.len() > 1 {
        return Err(OperationError::SeveralSubjects { subjects });
    }
    Ok(())
}

/// Records `value` in `seen` when it is not there already.
fn record(seen: &mut Vec<String>, value: String) {
    if !seen.contains(&value) {
        seen.push(value);
    }
}

/// Returns the program one `$toopenehr` call runs.
fn select<'set>(
    programs: &'set ProgramSet,
    entries: &[Object],
    template: Option<&TemplateId>,
) -> Result<&'set Program, OperationError> {
    let claimed = profiles(entries);
    if claimed.is_empty() {
        let pinned = template.ok_or(OperationError::Select {
            source: crate::resolve::select::SelectError::NoMatch {
                wanted: String::from("a bundle that claims no profile and pins no template"),
            },
        })?;
        return select_by_template(programs.programs(), pinned)
            .map(AsRef::as_ref)
            .map_err(|source| OperationError::Select { source });
    }
    select_by_profile_pinned(programs.programs(), &claimed, template)
        .map(AsRef::as_ref)
        .map_err(|source| OperationError::Select { source })
}

/// Returns every profile the Bundle's resources claim, in entry order.
///
/// A FHIR instance names the profiles it claims in `meta.profile`
/// (<https://hl7.org/fhir/R4/resource.html#Meta>); a Bundle carries its
/// resources, so the claimed set is the union over its entries.
fn profiles(entries: &[Object]) -> Vec<String> {
    let mut claimed: Vec<String> = Vec::new();
    for object in entries {
        let Some(Value::Array(listed)) = object.get("meta").and_then(|meta| meta.get("profile"))
        else {
            continue;
        };
        for entry in listed {
            if let Value::String(url) = entry {
                record(&mut claimed, url.clone());
            }
        }
    }
    claimed
}

/// Returns the one resource of the mapped type the Bundle carries.
fn mapped_resource(entries: &[Object], program: &Program) -> Result<Value, OperationError> {
    let wanted = program.resource().as_str();
    let found: Vec<&Object> = entries
        .iter()
        .filter(|object| {
            matches!(object.get("resourceType"), Some(Value::String(name)) if name == wanted)
        })
        .collect();
    match *found.as_slice() {
        [only] => Ok(Value::Object(only.clone())),
        [] => Err(OperationError::NoSubjectResource {
            resource: String::from(wanted),
            context: String::from(program.context().as_str()),
        }),
        // TODO(#94): split a bundle into one composition per mapped resource.
        ref several => Err(OperationError::SeveralSubjectResources {
            resource: String::from(wanted),
            count: several.len(),
        }),
    }
}

/// Returns the composition in the serialization the request asked for.
fn serialize(
    index: &WebTemplateIndex,
    composition: &CanonicalComposition,
    format: Format,
) -> Result<String, OperationError> {
    match format {
        Format::Canonical => Ok(composition.value().to_string()),
        Format::Flat => {
            let flat =
                index
                    .flatten(composition)
                    .map_err(|source| OperationError::Serialization {
                        source: Box::new(source),
                    })?;
            Ok(serde_json::Value::Object(flat).to_string())
        }
    }
}
