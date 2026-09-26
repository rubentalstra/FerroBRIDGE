// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The claims a delivery takes on its source keys before it reads the
//! identity map.

use http::StatusCode;

use crate::facade::identity::claims::Claim;
use crate::facade::identity::claims::Contended;
use crate::facade::identity::record::SourceVersion;
use crate::facade::outcome::Issue;
use crate::facade::outcome::IssueType;
use crate::facade::request::Inbound;

use super::Ingest;
use super::Mapped;
use super::Provenance;
use super::Refused;
use super::binding::resource_source;

/// Returns the keys one Bundle delivery claims.
///
/// Every keyed entry claims its source key ([`SourceVersion::storage_key`]).
/// An entry mapped from a resource also claims its `resourceType` and `id`
/// alone ([`SourceVersion::resource_key`]), as a single create does, since a
/// new version of that `id` revises the composition any other version
/// produced; the entries of one message share their message's key, so they
/// claim theirs alone.
pub(super) fn claim_keys(
    sources: &[Option<SourceVersion>],
    provenance: &Provenance,
) -> Vec<String> {
    let each = *provenance == Provenance::EachResource;
    sources
        .iter()
        .flatten()
        .flat_map(|source| {
            let resource = each.then(|| source.resource_key());
            std::iter::once(source.storage_key()).chain(resource)
        })
        .collect()
}

/// Returns the refusal of a Bundle whose entries `contended` names another
/// in-flight delivery as holding.
pub(super) fn contended_entries(
    mapped: &[Mapped<'_>],
    sources: &[Option<SourceVersion>],
    contended: &Contended,
) -> Refused {
    in_flight(
        mapped
            .iter()
            .zip(sources)
            .filter(|(_, source)| {
                source.as_ref().is_some_and(|source| {
                    contended.holds(&source.storage_key())
                        || contended.holds(&source.resource_key())
                })
            })
            .map(|(entry, _)| entry.full_url.clone()),
    )
}

/// Returns the refusal of a delivery whose source another in-flight delivery
/// holds, naming each place in `at`.
///
/// Nothing is committed; the caller retries once the first delivery settles
/// (no specification governs the redelivery rule: our own design).
fn in_flight(at: impl IntoIterator<Item = String>) -> Refused {
    let mut issues = vec![Issue::error(IssueType::Duplicate).diagnosing(
        "another delivery of the same source is in flight, so nothing is committed; retry once it settles",
    )];
    issues.extend(at.into_iter().map(|place| {
        Issue::error(IssueType::Duplicate)
            .diagnosing("another in-flight delivery holds this entry")
            .at(place)
    }));
    Refused::of(StatusCode::CONFLICT, issues)
}

/// Returns the refusal of a delivery that commits a source its claim does
/// not hold.
pub(super) fn unclaimed() -> Refused {
    Refused::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        Issue::error(IssueType::Exception)
            .diagnosing("the delivery holds no claim on the source it commits"),
    )
}

impl<'a> Ingest<'a> {
    /// Claims the source keys of `inbound` for one delivery.
    ///
    /// The caller takes the claim before it reads the identity map and holds
    /// it until the delivery settles. The claim holds the source's
    /// `resourceType`, `id` and `meta.versionId` ([`SourceVersion::storage_key`])
    /// and its `resourceType` and `id` alone ([`SourceVersion::resource_key`]),
    /// so a second single delivery of the same `id` that arrives meanwhile,
    /// whatever its `meta.versionId`, commits nothing. A resource with no `id`
    /// has no key, and its claim holds nothing. No specification governs the
    /// redelivery rule: our own design.
    ///
    /// # Errors
    ///
    /// Returns a `409` [`Refused`] with a `duplicate` issue when another
    /// in-flight delivery holds a key; the caller retries once that delivery
    /// settles.
    pub fn claim_resource(&self, inbound: &Inbound) -> Result<Claim<'a>, Refused> {
        let source = resource_source(inbound);
        self.claims
            .claim(
                source
                    .iter()
                    .flat_map(|source| [source.storage_key(), source.resource_key()]),
            )
            .map_err(|_contended| {
                let at = source.map_or_else(
                    || String::from(inbound.resource_type()),
                    |source| format!("{}/{}", source.resource_type(), source.id()),
                );
                in_flight(std::iter::once(at))
            })
    }
}
