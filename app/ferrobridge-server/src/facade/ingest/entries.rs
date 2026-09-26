// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The mapping of every entry of a Bundle, before anything reaches the CDR.

use std::collections::BTreeMap;

use fhir_types::codec::Value;
use fhirconnect::resolve::select::SelectError;
use http::StatusCode;
use openehr_rm::v1_2::composition::composition::Composition;

use crate::facade::identity::IdError;
use crate::facade::identity::PersonId;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::outcome::chain;
use crate::facade::request::Inbound;

use super::Ingest;
use super::Mapped;
use super::Mapping;
use super::Partition;
use super::Pending;
use super::Provenance;
use super::Refused;
use super::SkipReason;
use super::Skipped;
use super::UnmappedEntries;

/// Returns the subject a Bundle-local reference of `inbound` names, read from
/// the first `identifier` with a `value` of the entry it references.
///
/// A reference whose value is another entry's `fullUrl` resolves to that
/// entry (R4 `Bundle.entry.fullUrl`; <https://hl7.org/fhir/R4/bundle.html#references>),
/// so a `urn:uuid` a message Bundle allocates per message never becomes a
/// person key of its own. `None` when `inbound` names its subject by
/// identifier, when the reference names no entry of the Bundle, or when that
/// entry carries no identifier with a value; the reference is then read as
/// [`Inbound::subject`] reads it.
fn local_subject(
    inbound: &Inbound,
    locals: &BTreeMap<&str, &Value>,
    namespace: &str,
) -> Option<Result<PersonId, IdError>> {
    let document = inbound.document();
    let reference = document
        .get("subject")
        .or_else(|| document.get("patient"))?;
    if reference.get("identifier").is_some() {
        return None;
    }
    let target = locals.get(reference.get("reference")?.as_str()?)?;
    // NOTE: no specification governs this: our own design; the first identifier
    // in document order keys the person, as PID-3's first repetition does.
    let identifier = target
        .get("identifier")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .find(|identifier| identifier.get("value").and_then(Value::as_str).is_some())?;
    let value = identifier.get("value").and_then(Value::as_str)?;
    let system = identifier
        .get("system")
        .and_then(Value::as_str)
        .unwrap_or(namespace);
    Some(PersonId::new(system, value))
}

/// Returns the refusal of a Bundle no entry of which mapped, naming every
/// entry that was skipped and why.
pub(super) fn nothing_mapped(partition: &Partition<'_>) -> Refused {
    let mut issues = vec![
        Issue::error(IssueType::Required)
            .diagnosing("the Bundle carries no entry this server maps"),
    ];
    for slot in &partition.pending {
        if let Pending::Skipped(skipped) = slot {
            let why = match skipped.reason {
                SkipReason::NoProgramForType => {
                    format!("no loaded mapping answers for {}", skipped.resource_type)
                }
                SkipReason::NoProgramForProfiles { ref profiles } => format!(
                    "no loaded mapping for {} claims one of the profiles [{}]",
                    skipped.resource_type,
                    profiles.join(", ")
                ),
            };
            issues.push(
                Issue::error(IssueType::NotSupported)
                    .diagnosing(why)
                    .at(skipped.full_url.clone()),
            );
        }
    }
    Refused::of(StatusCode::UNPROCESSABLE_ENTITY, issues)
}

impl<'a> Ingest<'a> {
    /// Maps every entry, collecting the refusals rather than stopping at the
    /// first.
    pub(super) fn map_entries(
        &self,
        entries: &[Value],
        unmapped: UnmappedEntries,
        provenance: &Provenance,
    ) -> Partition<'a> {
        let mut partition = Partition::default();
        let locals: BTreeMap<&str, &Value> = entries
            .iter()
            .filter_map(|entry| Some((entry.get("fullUrl")?.as_str()?, entry.get("resource")?)))
            .collect();
        for (index, entry) in entries.iter().enumerate() {
            let full_url = entry
                .get("fullUrl")
                .and_then(Value::as_str)
                .map_or_else(|| format!("Bundle.entry[{index}]"), str::to_owned);
            let Some(resource) = entry.get("resource") else {
                partition.failures.push(
                    Issue::error(IssueType::Required)
                        .diagnosing("the entry carries no resource")
                        .at(full_url),
                );
                continue;
            };
            match self.map_one(resource, &full_url, index, &locals, provenance) {
                Ok(Mapping::Mapped(one, person)) => {
                    match partition.subject {
                        None => partition.subject = Some(person),
                        Some(ref held) if held == &person => {}
                        Some(_) => {
                            partition.failures.push(
                                Issue::error(IssueType::Processing)
                                    .diagnosing(
                                        "the Bundle references more than one subject, and one Bundle maps to one composition",
                                    )
                                    .at(one.full_url.clone()),
                            );
                        }
                    }
                    partition
                        .pending
                        .push(Pending::Mapped(partition.mapped.len()));
                    partition.mapped.push(*one);
                }
                Ok(Mapping::Unmapped(skipped, issue)) => match unmapped {
                    UnmappedEntries::Refuse => partition.failures.push(issue),
                    UnmappedEntries::SkipAndCount => {
                        partition.pending.push(Pending::Skipped(skipped));
                    }
                },
                Err(issue) => partition.failures.push(issue),
            }
        }
        partition
    }

    /// Maps one Bundle entry, naming it by its `fullUrl` on refusal.
    ///
    /// `locals` holds every entry of the Bundle by its `fullUrl`, so a
    /// subject reference to another entry is read through that entry
    /// ([`local_subject`]).
    fn map_one(
        &self,
        resource: &Value,
        full_url: &str,
        position: usize,
        locals: &BTreeMap<&str, &Value>,
        provenance: &Provenance,
    ) -> Result<Mapping<'a>, Issue> {
        let inbound = Inbound::of(resource.clone(), "").map_err(|error| {
            Issue::error(IssueType::Structure)
                .diagnosing(chain(&error))
                .at(String::from(full_url))
        })?;
        if !self.programs.supports(inbound.resource_type()) {
            return Ok(Mapping::Unmapped(
                Skipped {
                    full_url: String::from(full_url),
                    resource_type: String::from(inbound.resource_type()),
                    reason: SkipReason::NoProgramForType,
                },
                Issue::error(IssueType::NotSupported)
                    .diagnosing(format!(
                        "no loaded mapping answers for {}",
                        inbound.resource_type()
                    ))
                    .at(String::from(full_url)),
            ));
        }
        let program = match self.programs.select(inbound.profiles(), None) {
            Ok(program) => program,
            Err(error) => {
                let issue = Issue::error(IssueType::NotSupported)
                    .diagnosing(error.to_string())
                    .at(String::from(full_url));
                return match error {
                    SelectError::NoMatch { .. } => Ok(Mapping::Unmapped(
                        Skipped {
                            full_url: String::from(full_url),
                            resource_type: String::from(inbound.resource_type()),
                            reason: SkipReason::NoProgramForProfiles {
                                profiles: inbound.profiles().to_vec(),
                            },
                        },
                        issue,
                    )),
                    _ => Err(issue),
                };
            }
        };
        let namespace = self.settings.subject_namespace.as_str();
        let subject = match local_subject(&inbound, locals, namespace) {
            Some(local) => local.map_err(|error| chain(&error)),
            None => inbound.subject(namespace).map_err(|error| chain(&error)),
        }
        .map_err(|text| {
            Issue::error(IssueType::Required)
                .diagnosing(text)
                .at(String::from(full_url))
        })?;
        let (composition, instant) = self.run(program, &inbound, provenance).map_err(|error| {
            Issue::error(IssueType::Processing)
                .diagnosing(chain(&error))
                .at(String::from(full_url))
        })?;
        let text = serde_json::to_string(composition.value()).map_err(|error| {
            Issue::error(IssueType::Exception)
                .diagnosing(format!(
                    "the built composition could not be written: {error}"
                ))
                .at(String::from(full_url))
        })?;
        let rm = openehr_its::json::from_canonical_json::<Composition>(&text).map_err(|error| {
            Issue::error(IssueType::Processing)
                .diagnosing(format!(
                    "the built composition is no valid openEHR COMPOSITION: {error}"
                ))
                .at(String::from(full_url))
        })?;
        Ok(Mapping::Mapped(
            Box::new(Mapped {
                full_url: String::from(full_url),
                position,
                template_id: String::from(composition.template_id()),
                inbound,
                program,
                built: composition,
                composition: Box::new(rm),
                instant,
            }),
            subject,
        ))
    }
}
