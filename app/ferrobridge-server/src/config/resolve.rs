// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The resolution of each lane of the tree into the settings the run path
//! holds.

use secrecy::ExposeSecret;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use super::CdmSettings;
use super::Error;
use super::FacadeSettings;
use super::Hl7v2Settings;
use super::MappingSettings;
use super::credentials::Scheme;
use super::credentials::cdm_connection_key;
use super::credentials::resolve_credentials;
use super::credentials::secret;
use super::section::Cdm;
use super::section::Cdr;
use super::section::Etl;
use super::section::Facade;
use super::section::Hl7v2;
use super::section::Hl7v2Sender;
use super::section::Mappings;
use super::section::Terminology;

/// Returns the mapping-set paths `mappings` names, when it names both.
///
/// A set with no `templates` resolves to `None` here and is carried by
/// [`Settings::mapping_directory`] alone: the facade and the two operations
/// then take their templates from the CDR at boot. A `templates` with no
/// `directory` names nothing to compile.
///
/// # Errors
///
/// Returns [`Error::Missing`] when `templates` is set and `directory` is not.
///
/// [`Settings::mapping_directory`]: super::Settings::mapping_directory
pub(super) fn resolve_mappings(mappings: &Mappings) -> Result<Option<MappingSettings>, Error> {
    match (mappings.directory.as_ref(), mappings.templates.as_ref()) {
        (Some(directory), Some(templates)) => Ok(Some(MappingSettings {
            directory: directory.clone(),
            templates: templates.clone(),
        })),
        (None, Some(_)) => Err(Error::Missing {
            key: String::from("mappings.directory"),
        }),
        _ => Ok(None),
    }
}

/// Returns the facade settings `facade` describes.
pub(super) fn resolve_facade(facade: &Facade) -> Result<FacadeSettings, Error> {
    let ehr_policy = ehr_policy_of("facade.ehr_policy", &facade.ehr_policy)?;
    for (key, value) in [
        ("facade.base_url", &facade.base_url),
        ("facade.composition_language", &facade.composition_language),
        (
            "facade.composition_territory",
            &facade.composition_territory,
        ),
    ] {
        if value.is_empty() {
            return Err(Error::Missing {
                key: String::from(key),
            });
        }
    }
    Ok(FacadeSettings {
        settings: crate::facade::Settings {
            base_url: facade.base_url.clone(),
            ehr_policy,
            subject_namespace: facade.subject_namespace.clone(),
            system_id: facade.system_id.clone(),
            language: facade.composition_language.clone(),
            territory: facade.composition_territory.clone(),
        },
        identity_store: facade.identity_store.clone(),
    })
}

/// Returns the sender `sender` names under `key`.
fn sender_of(key: &str, sender: &Hl7v2Sender) -> Result<crate::hl7v2::Sender, Error> {
    let named = |text: &str| !text.is_empty();
    match (
        named(&sender.namespace_id),
        named(&sender.universal_id),
        named(&sender.universal_id_type),
    ) {
        (true, false, false) => Ok(crate::hl7v2::Sender::Namespace(sender.namespace_id.clone())),
        (false, true, true) => Ok(crate::hl7v2::Sender::Universal {
            id: sender.universal_id.clone(),
            kind: sender.universal_id_type.clone(),
        }),
        _ => Err(Error::Sender {
            key: key.to_owned(),
        }),
    }
}

/// Returns the EHR policy `value` names under `key`.
fn ehr_policy_of(key: &str, value: &str) -> Result<crate::facade::ehr::Policy, Error> {
    match value {
        "existing" => Ok(crate::facade::ehr::Policy::Existing),
        "create_on_first_write" => Ok(crate::facade::ehr::Policy::CreateOnFirstWrite),
        _ => Err(Error::EhrPolicy {
            key: key.to_owned(),
            value: value.to_owned(),
        }),
    }
}

/// Returns the timeout `milliseconds` states, `None` for `0`.
fn timeout_of(milliseconds: u64) -> Option<Duration> {
    (milliseconds > 0).then(|| Duration::from_millis(milliseconds))
}

/// Returns the HL7 v2 face `hl7v2` describes, refused when the facade it
/// writes through is off.
pub(super) fn resolve_hl7v2(hl7v2: &Hl7v2, facade: bool) -> Result<Hl7v2Settings, Error> {
    if !facade {
        return Err(Error::Needs {
            key: String::from("hl7v2.enabled"),
            needs: String::from("facade.enabled"),
        });
    }
    let listen = hl7v2
        .listen
        .parse::<SocketAddr>()
        .map_err(|source| Error::Listen {
            key: String::from("hl7v2.listen"),
            source,
        })?;
    let default_charset = ferrobridge_hl7v2::decode::Charset::from_code(&hl7v2.default_charset)
        .ok_or_else(|| Error::Charset {
            key: String::from("hl7v2.default_charset"),
            value: hl7v2.default_charset.clone(),
        })?;
    let concept_maps = hl7v2.concept_maps.clone().ok_or_else(|| Error::Missing {
        key: String::from("hl7v2.concept_maps"),
    })?;
    let unmapped = match hl7v2.unmapped_entries.as_str() {
        "skip_and_count" => crate::facade::ingest::UnmappedEntries::SkipAndCount,
        "refuse" => crate::facade::ingest::UnmappedEntries::Refuse,
        _ => {
            return Err(Error::UnmappedEntries {
                key: String::from("hl7v2.unmapped_entries"),
                value: hl7v2.unmapped_entries.clone(),
            });
        }
    };
    let ehr_policy = hl7v2
        .ehr_policy
        .as_deref()
        .map(|value| ehr_policy_of("hl7v2.ehr_policy", value))
        .transpose()?;
    let mut profiles = BTreeMap::new();
    for claimed in &hl7v2.profiles {
        if !fhir_types::r4::schema::SCHEMAS.is_resource(&claimed.resource_type) {
            return Err(Error::ResourceType {
                key: String::from("hl7v2.profiles.resource_type"),
                value: claimed.resource_type.clone(),
            });
        }
        url_of("hl7v2.profiles.profile", &claimed.profile)?;
        if profiles
            .insert(claimed.resource_type.clone(), claimed.profile.clone())
            .is_some()
        {
            return Err(Error::Duplicate {
                key: String::from("hl7v2.profiles"),
                value: claimed.resource_type.clone(),
            });
        }
    }
    if hl7v2.frame_limit_bytes == 0 {
        return Err(Error::FrameLimit {
            key: String::from("hl7v2.frame_limit_bytes"),
        });
    }
    Ok(Hl7v2Settings {
        listen,
        default_charset,
        concept_maps,
        supplements: hl7v2.supplements.clone(),
        unmapped,
        ehr_policy,
        profiles,
        timeouts: ferrobridge_hl7v2::mllp::Timeouts {
            idle: timeout_of(hl7v2.idle_timeout_ms),
            frame: timeout_of(hl7v2.frame_timeout_ms),
        },
        frame_limit: hl7v2.frame_limit_bytes,
        log_outcomes: hl7v2.log_outcomes,
        senders: hl7v2
            .senders
            .iter()
            .map(|sender| sender_of("hl7v2.senders", sender))
            .collect::<Result<Vec<_>, Error>>()?,
    })
}

/// Returns the CDR client configuration `cdr` describes.
pub(super) fn resolve_cdr(cdr: &Cdr) -> Result<crate::cdr::config::CdrConfig, Error> {
    let base_url = url_of("cdr.base_url", &cdr.base_url)?;
    let mut config = crate::cdr::config::CdrConfig::new(base_url)
        .with_timeout(Duration::from_millis(cdr.timeout_ms))
        .with_retry(openehr_its::rest::client::RetryPolicy {
            // NOTE: no specification governs this: our own design; a u32 fits a
            // usize on every target the server builds for, so the clamp never bites.
            max_attempts: usize::try_from(cdr.retry.max_attempts).unwrap_or(usize::MAX),
            initial_backoff: Duration::from_millis(cdr.retry.initial_backoff_ms),
            max_backoff: Duration::from_millis(cdr.retry.max_backoff_ms),
        });
    if let Some(scheme) = resolve_credentials("cdr.credentials", &cdr.credentials)? {
        config = config.with_credentials(match scheme {
            Scheme::Bearer(token) => openehr_its::rest::client::Credentials::bearer(token),
            Scheme::Basic { user, password } => {
                openehr_its::rest::client::Credentials::basic(user, password)
            }
        });
    }
    Ok(config)
}

/// Returns the terminology client configuration `terminology` describes.
pub(super) fn resolve_terminology(
    terminology: &Terminology,
) -> Result<ferrobridge_term::config::Config, Error> {
    let base_url = url_of("terminology.base_url", &terminology.base_url)?;
    let wire_version = match terminology.wire_version.to_ascii_lowercase().as_str() {
        "r4" => ferrobridge_term::config::WireVersion::R4,
        "r4b" => ferrobridge_term::config::WireVersion::R4B,
        _ => {
            return Err(Error::WireVersion {
                key: String::from("terminology.wire_version"),
                value: terminology.wire_version.clone(),
            });
        }
    };
    let mut config = ferrobridge_term::config::Config::new(base_url, wire_version)
        .with_timeout(Duration::from_millis(terminology.timeout_ms))
        .with_retry(ferrobridge_term::config::RetryPolicy {
            max_attempts: terminology.retry.max_attempts,
            initial_backoff: Duration::from_millis(terminology.retry.initial_backoff_ms),
            max_backoff: Duration::from_millis(terminology.retry.max_backoff_ms),
        });
    if let Some(scheme) = resolve_credentials("terminology.credentials", &terminology.credentials)?
    {
        config = config.with_credentials(match scheme {
            Scheme::Bearer(token) => ferrobridge_term::config::Credentials::Bearer(token),
            Scheme::Basic { user, password } => {
                ferrobridge_term::config::Credentials::Basic { user, password }
            }
        });
    }
    Ok(config)
}

/// Returns the CDM database `cdm` describes.
pub(super) fn resolve_cdm(cdm: &Cdm) -> Result<CdmSettings, Error> {
    let url =
        secret("cdm.url", cdm.url.as_deref(), cdm.url_file.as_deref())?.ok_or(Error::Missing {
            key: String::from("cdm.url"),
        })?;
    let schema_of = |key: &str, name: &str| {
        omop_cdm::ddl::SchemaName::new(name).map_err(|source| Error::Schema {
            key: key.to_owned(),
            source,
        })
    };
    let person_policy = match cdm.person_policy.as_str() {
        "create_on_first_sight" => omop_cdm::writer::input::PersonPolicy::CreateOnFirstSight,
        "existing" => omop_cdm::writer::input::PersonPolicy::Existing,
        _ => {
            return Err(Error::PersonPolicy {
                key: String::from("cdm.person_policy"),
                value: cdm.person_policy.clone(),
            });
        }
    };
    let ca = match (cdm.tls_ca.as_deref(), cdm.tls_ca_file.as_deref()) {
        (Some(_), Some(_)) => {
            return Err(Error::Conflict {
                key: String::from("cdm.tls_ca"),
            });
        }
        (Some(pem), None) => Some(pem.as_bytes().to_vec()),
        (None, Some(path)) => Some(std::fs::read(path).map_err(|source| Error::Secret {
            key: String::from("cdm.tls_ca_file"),
            path: path.to_path_buf(),
            source,
        })?),
        (None, None) => None,
    };
    let connection = omop_cdm::connection::CdmConnection::new(url.expose_secret(), ca.as_deref())
        .map_err(|source| Error::CdmConnection {
        key: String::from(cdm_connection_key(&source, cdm)),
        source,
    })?;
    Ok(CdmSettings {
        url,
        connection,
        schema: schema_of("cdm.schema", &cdm.schema)?,
        bridge_schema: schema_of("cdm.bridge_schema", &cdm.bridge_schema)?,
        person_policy,
    })
}

/// Returns the value of the required integer `key`.
pub(super) fn required(key: &str, value: Option<i32>) -> Result<i32, Error> {
    value.ok_or_else(|| Error::Missing {
        key: key.to_owned(),
    })
}

/// Returns the checked query `text` holds, refusing it under `key`.
fn query_of(
    key: &str,
    text: &str,
    check: fn(&str) -> Result<crate::etl::aql::CheckedQuery, crate::etl::aql::AqlError>,
) -> Result<crate::etl::aql::CheckedQuery, Error> {
    if text.is_empty() {
        return Err(Error::Missing {
            key: key.to_owned(),
        });
    }
    check(text).map_err(|source| Error::Aql {
        key: key.to_owned(),
        source: Box::new(source),
    })
}

/// Returns the ETL job `etl` describes, with both queries checked.
pub(super) fn resolve_etl(etl: &Etl) -> Result<crate::etl::EtlSettings, Error> {
    let compositions = query_of(
        "etl.aql",
        &etl.aql,
        crate::etl::aql::CheckedQuery::compositions,
    )?;
    let page_size =
        crate::cdr::query::PageSize::new(etl.page_size).map_err(|source| Error::PageSize {
            key: String::from("etl.page_size"),
            source,
        })?;
    let visits = etl
        .visits
        .as_ref()
        .map(|visits| {
            Ok::<_, Error>(crate::etl::VisitSettings {
                query: query_of(
                    "etl.visits.aql",
                    &visits.aql,
                    crate::etl::aql::CheckedQuery::visits,
                )?,
                concepts: omop_cdm::writer::input::VisitConcepts {
                    visit_concept_id: required(
                        "etl.visits.visit_concept_id",
                        visits.visit_concept_id,
                    )?,
                    visit_type_concept_id: required(
                        "etl.visits.visit_type_concept_id",
                        visits.visit_type_concept_id,
                    )?,
                },
            })
        })
        .transpose()?;
    Ok(crate::etl::EtlSettings {
        compositions,
        page_size,
        type_concept_id: required("etl.type_concept_id", etl.type_concept_id)?,
        observation_period_type_concept_id: required(
            "etl.observation_period_type_concept_id",
            etl.observation_period_type_concept_id,
        )?,
        visits,
    })
}

/// Returns the URL `value` holds, naming `key` when it does not parse.
fn url_of(key: &str, value: &str) -> Result<url::Url, Error> {
    if value.is_empty() {
        return Err(Error::Missing {
            key: key.to_owned(),
        });
    }
    value.parse().map_err(|source| Error::Url {
        key: key.to_owned(),
        source,
    })
}
