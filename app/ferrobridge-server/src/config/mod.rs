// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Configuration: an optional TOML file, then environment overrides, then one
//! typed [`Settings`] the run path holds.
//!
//! Every struct refuses an unknown key, every default lives inline in its own
//! `Default` impl, and every credential is reachable through a `<key>_file`
//! sibling read at boot. A lane is off until its section is present. No
//! specification governs the configuration: our own design.

mod credentials;
mod overrides;
mod resolve;
pub mod section;

use secrecy::SecretString;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::telemetry::{FILTER_ENV, FORMAT_ENV, Format};

use overrides::apply_console_overrides;
use overrides::apply_override;
use resolve::resolve_cdm;
use resolve::resolve_cdr;
use resolve::resolve_etl;
use resolve::resolve_facade;
use resolve::resolve_hl7v2;
use resolve::resolve_mappings;
use resolve::resolve_terminology;
use section::Cdm;
use section::Cdr;
use section::Etl;
use section::Facade;
use section::Hl7v2;
use section::Mappings;
use section::Operations;
use section::Server;
use section::Telemetry;
use section::Terminology;

/// The prefix of every environment override.
///
/// The name after it is the dotted key with `__` between segments, upper or
/// lower case: `FERROBRIDGE__SERVER__LISTEN` sets `[server] listen`.
pub const ENV_PREFIX: &str = "FERROBRIDGE__";

/// The environment variable naming the configuration file.
pub const CONFIG_PATH_ENV: &str = "FERROBRIDGE_CONFIG";

/// The whole configuration tree, as a file and the environment state it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The HTTP surface.
    pub server: Server,
    /// The console.
    pub telemetry: Telemetry,
    /// The openEHR CDR, when one is configured.
    pub cdr: Option<Cdr>,
    /// The FHIR terminology server, when one is configured.
    pub terminology: Option<Terminology>,
    /// The OMOP CDM database, when one is configured.
    pub cdm: Option<Cdm>,
    /// The OMOP ETL job, when one is configured.
    pub etl: Option<Etl>,
    /// Where the mapping files are read from.
    pub mappings: Mappings,
    /// The FHIR facade, off until its section turns it on.
    pub facade: Facade,
    /// The FHIRconnect operations.
    pub operations: Operations,
    /// The HL7 v2 face, off until its section turns it on.
    pub hl7v2: Hl7v2,
}

/// A configuration the server refuses to start on.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The configuration file could not be read.
    #[error("the configuration file {} could not be read", path.display())]
    Read {
        /// The path that was tried.
        path: PathBuf,
        /// What the file system reported.
        #[source]
        source: std::io::Error,
    },
    /// The configuration does not parse, names an unknown key, or holds a
    /// value of the wrong type.
    #[error("the configuration is not valid")]
    Parse {
        /// What the TOML reader reported, with the offending key.
        #[source]
        source: toml::de::Error,
    },
    /// The merged configuration could not be written back for re-reading.
    #[error("the configuration could not be assembled")]
    Assemble {
        /// What the TOML writer reported.
        #[source]
        source: toml::ser::Error,
    },
    /// An environment override names no key under the prefix.
    #[error("{name} names no configuration key; use {ENV_PREFIX}<SECTION>__<KEY>")]
    EnvName {
        /// The variable that was read.
        name: String,
    },
    /// An environment override addresses a key under a value that is not a
    /// section.
    #[error("{name} addresses a key under a value that is not a section")]
    EnvShape {
        /// The variable that was read.
        name: String,
    },
    /// A value and its `_file` sibling are both set.
    #[error("{key} is set together with {key}_file; set one of them")]
    Conflict {
        /// The inline key.
        key: String,
    },
    /// A `_file` sibling could not be read.
    #[error("{key} names {}, which could not be read", path.display())]
    Secret {
        /// The `_file` key.
        key: String,
        /// The path it named.
        path: PathBuf,
        /// What the file system reported.
        #[source]
        source: std::io::Error,
    },
    /// A section is present and a key it needs is not set.
    #[error("{key} is not set, and its section is present")]
    Missing {
        /// The key that carries no value.
        key: String,
    },
    /// A socket address does not parse.
    #[error("{key} is not a socket address")]
    Listen {
        /// The key that holds it.
        key: String,
        /// What the address parser reported.
        #[source]
        source: std::net::AddrParseError,
    },
    /// The EHR policy names neither of the two values.
    #[error("{key} is `{value}`; the policies are existing and create_on_first_write")]
    EhrPolicy {
        /// The key that holds it.
        key: String,
        /// The value it holds.
        value: String,
    },
    /// The identity store could not be opened at boot.
    #[error("the identity store at {} could not be opened", path.display())]
    IdentityStore {
        /// The path that was tried.
        path: PathBuf,
        /// What the store reported.
        #[source]
        source: Box<crate::facade::identity::store::StoreError>,
    },
    /// A URL does not parse.
    #[error("{key} is not a URL")]
    Url {
        /// The key that holds it.
        key: String,
        /// What the URL parser reported.
        #[source]
        source: url::ParseError,
    },
    /// A FHIR release name is not one this client speaks.
    #[error("{key} is `{value}`; the releases this client speaks are r4 and r4b")]
    WireVersion {
        /// The key that holds it.
        key: String,
        /// The value it holds.
        value: String,
    },
    /// A credentials section names both a bearer token and a user.
    #[error("{section} names both a bearer token and a user; set one scheme")]
    Scheme {
        /// The credentials section.
        section: String,
    },
    /// An upstream client refused the configuration it was handed.
    #[error("the {upstream} client refused its configuration")]
    Client {
        /// Which upstream it was.
        upstream: &'static str,
        /// What the client reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A schema name is not an unquoted PostgreSQL identifier.
    #[error("{key} is not a schema name")]
    Schema {
        /// The key that holds it.
        key: String,
        /// Why the name is refused.
        #[source]
        source: omop_cdm::ddl::SchemaNameError,
    },
    /// The CDM database URL, or the CA it is checked against, is refused.
    #[error("{key} is refused")]
    CdmConnection {
        /// The key that holds the refused value.
        key: String,
        /// Why it is refused, the `sslmode` named when that is the fault.
        #[source]
        source: omop_cdm::connection::ConnectionError,
    },
    /// The person policy names neither of the two values.
    #[error("{key} is `{value}`; the policies are create_on_first_sight and existing")]
    PersonPolicy {
        /// The key that holds it.
        key: String,
        /// The value it holds.
        value: String,
    },
    /// A page size that would never advance.
    #[error("{key} is 0; a page holds at least one row")]
    PageSize {
        /// The key that holds it.
        key: String,
        /// What the page size reported.
        #[source]
        source: crate::cdr::query::PageSizeError,
    },
    /// A configured AQL the runner cannot read.
    #[error("{key} is refused")]
    Aql {
        /// The key that holds it.
        key: String,
        /// Why the query is refused.
        #[source]
        source: Box<crate::etl::aql::AqlError>,
    },
    /// A lane is switched on and a lane it needs is not.
    #[error("{key} is set, and it needs {needs}")]
    Needs {
        /// The key that switches the lane on.
        key: String,
        /// The lane it needs, as the key that switches it on.
        needs: String,
    },
    /// A character set code is not one of HL7 table 0211 that is decoded.
    #[error(
        "{key} is `{value}`; the character sets are ASCII, UNICODE UTF-8, 8859/1 to 8859/9 and 8859/15"
    )]
    Charset {
        /// The key that holds it.
        key: String,
        /// The value it holds.
        value: String,
    },
    /// The unmapped-entry policy names neither of the two values.
    #[error("{key} is `{value}`; the policies are skip_and_count and refuse")]
    UnmappedEntries {
        /// The key that holds it.
        key: String,
        /// The value it holds.
        value: String,
    },
    /// A resource type the R4 element table does not name.
    #[error("{key} is `{value}`, which names no R4 resource type")]
    ResourceType {
        /// The key that holds it.
        key: String,
        /// The value it holds.
        value: String,
    },
    /// A resource type named twice.
    #[error("{key} names {value} twice")]
    Duplicate {
        /// The list that holds it.
        key: String,
        /// The value named twice.
        value: String,
    },
    /// A sending facility named neither by a namespace id alone nor by a
    /// universal id with its type.
    #[error(
        "{key} names a sender by namespace_id alone, or by universal_id with universal_id_type"
    )]
    Sender {
        /// The key that holds it.
        key: String,
    },
    /// A size that would refuse every message.
    #[error("{key} is 0; a frame carries at least one byte")]
    FrameLimit {
        /// The key that holds it.
        key: String,
    },
    /// The mapping set the operations lane runs did not load.
    #[error("the mapping set could not be loaded")]
    Mappings {
        /// Why it did not load, the template and upstream status included.
        #[source]
        source: Box<crate::mappings::Error>,
    },
}

impl Config {
    /// Reads the configuration file `path` names, then the environment.
    ///
    /// `path` is what `--config` named; without it the file is the one
    /// `FERROBRIDGE_CONFIG` names, and without that the defaults stand.
    ///
    /// # Errors
    /// Returns [`Error::Read`] when a named file cannot be read and every
    /// other [`Error`] the merge and the parse produce.
    pub fn load(path: Option<&Path>) -> Result<Self, Error> {
        let named = path
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os(CONFIG_PATH_ENV).map(PathBuf::from));
        let text = match named {
            None => None,
            Some(path) => {
                let text = std::fs::read_to_string(&path).map_err(|source| Error::Read {
                    path: path.clone(),
                    source,
                })?;
                Some(text)
            }
        };
        let environment: BTreeMap<String, String> = std::env::vars()
            .filter(|(name, _)| {
                name.starts_with(ENV_PREFIX) || name == FORMAT_ENV || name == FILTER_ENV
            })
            .collect();
        Self::from_sources(text.as_deref(), &environment)
    }

    /// Reads `text` as TOML and applies `environment` over it.
    ///
    /// Reading the sources and reading the process environment are separate so
    /// a test can state both: setting an environment variable is `unsafe` in
    /// edition 2024 and this workspace forbids `unsafe`.
    ///
    /// # Errors
    /// Returns [`Error::Parse`] for TOML that does not read as this tree,
    /// [`Error::EnvName`] and [`Error::EnvShape`] for an override that names
    /// no key, and [`Error::Assemble`] when the merged tree cannot be written.
    pub fn from_sources(
        text: Option<&str>,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, Error> {
        let mut table = match text {
            None => toml::Table::new(),
            Some(text) => toml::from_str(text).map_err(|source| Error::Parse { source })?,
        };
        for (name, raw) in environment {
            apply_override(&mut table, name, raw)?;
        }
        apply_console_overrides(&mut table, environment)?;
        // The merged tree is written back and re-read so every refusal carries
        // the key and its position, which a `Table` alone cannot report.
        let merged = toml::to_string(&table).map_err(|source| Error::Assemble { source })?;
        toml::from_str(&merged).map_err(|source| Error::Parse { source })
    }

    /// Resolves this tree into the settings the run path holds.
    ///
    /// Every `_file` sibling is read here, so a secret reaches the process
    /// once, at boot, and never sits in the configuration tree.
    ///
    /// # Errors
    /// Returns [`Error::Conflict`] when a value and its `_file` sibling are
    /// both set, [`Error::Secret`] when a `_file` cannot be read, and the
    /// value errors ([`Error::Listen`], [`Error::Url`],
    /// [`Error::WireVersion`], [`Error::Missing`], [`Error::Scheme`],
    /// [`Error::Client`]) each naming the key that carries the fault. Returns
    /// [`Error::Mappings`] over [`crate::mappings::Error::NoTemplateSource`]
    /// when the operations are enabled and `[mappings] directory` is set with
    /// neither `[mappings] templates` nor `[cdr]`, so the refusal comes
    /// before any network call.
    pub fn resolve(&self) -> Result<Settings, Error> {
        if let Some(directory) = self.mappings.directory.as_ref()
            && self.operations.enabled
            && self.mappings.templates.is_none()
            && self.cdr.is_none()
        {
            return Err(Error::Mappings {
                source: Box::new(crate::mappings::Error::NoTemplateSource {
                    directory: directory.clone(),
                }),
            });
        }
        let listen = self
            .server
            .listen
            .parse::<SocketAddr>()
            .map_err(|source| Error::Listen {
                key: String::from("server.listen"),
                source,
            })?;
        Ok(Settings {
            server: ServerSettings {
                listen,
                request_timeout: Duration::from_millis(self.server.request_timeout_ms),
                shutdown_timeout: Duration::from_millis(self.server.shutdown_timeout_ms),
                body_limit: self.server.body_limit_bytes,
            },
            telemetry: TelemetrySettings {
                format: self.telemetry.format,
                filter: self.telemetry.filter.clone(),
                logged_query_parameters: self.telemetry.logged_query_parameters.clone(),
            },
            cdr: self.cdr.as_ref().map(resolve_cdr).transpose()?,
            terminology: self
                .terminology
                .as_ref()
                .map(resolve_terminology)
                .transpose()?,
            cdm: self.cdm.as_ref().map(resolve_cdm).transpose()?,
            etl: self.etl.as_ref().map(resolve_etl).transpose()?,
            mapping_directory: self.mappings.directory.clone(),
            omocl_directory: self.mappings.omocl.clone(),
            mappings: resolve_mappings(&self.mappings)?,
            facade: if self.facade.enabled {
                Some(resolve_facade(&self.facade)?)
            } else {
                None
            },
            hl7v2: if self.hl7v2.enabled {
                Some(resolve_hl7v2(&self.hl7v2, self.facade.enabled)?)
            } else {
                None
            },
            operations: OperationsSettings {
                enabled: self.operations.enabled,
                device_reference: self.operations.device_reference.clone(),
                composer: self.operations.composer.clone(),
                composition_language: self.operations.composition_language.clone(),
                composition_territory: self.operations.composition_territory.clone(),
            },
        })
    }
}

/// The facade lane, resolved.
#[derive(Debug, Clone)]
pub struct FacadeSettings {
    /// What the facade itself was configured with.
    pub settings: crate::facade::Settings,
    /// The file the identity map is kept in.
    pub identity_store: PathBuf,
}

/// The HL7 v2 face, resolved.
#[derive(Debug, Clone)]
pub struct Hl7v2Settings {
    /// The socket address the MLLP listener binds.
    pub listen: SocketAddr,
    /// The character set a message with an empty MSH-18 is read in.
    pub default_charset: ferrobridge_hl7v2::decode::Charset,
    /// The directory of the guide's `ConceptMap` files.
    pub concept_maps: PathBuf,
    /// The supplement directories, in load order.
    pub supplements: Vec<PathBuf>,
    /// What an entry no program maps does to the message.
    pub unmapped: crate::facade::ingest::UnmappedEntries,
    /// The EHR policy of the face, when it differs from the facade's.
    pub ehr_policy: Option<crate::facade::ehr::Policy>,
    /// The profile each resource type claims when the message map wrote
    /// none, by resource type.
    pub profiles: BTreeMap<String, String>,
    /// How long a connection and a frame may wait on the peer.
    pub timeouts: ferrobridge_hl7v2::mllp::Timeouts,
    /// The largest message one frame may carry, in bytes.
    pub frame_limit: usize,
    /// Whether an `AE` or an `AR` also logs the counted outcomes.
    pub log_outcomes: bool,
    /// The sending facilities the face accepts; empty accepts any.
    pub senders: Vec<crate::hl7v2::Sender>,
}

/// The settings the run path holds, with every secret already read.
#[derive(Debug)]
pub struct Settings {
    /// The HTTP surface.
    pub server: ServerSettings,
    /// The console.
    pub telemetry: TelemetrySettings,
    /// The CDR client configuration, when the lane is on.
    pub cdr: Option<crate::cdr::config::CdrConfig>,
    /// The terminology client configuration, when the lane is on.
    pub terminology: Option<ferrobridge_term::config::Config>,
    /// The OMOP CDM database, when the lane is on.
    pub cdm: Option<CdmSettings>,
    /// The OMOP ETL job, when it is configured.
    pub etl: Option<crate::etl::EtlSettings>,
    /// The directory the mapping files are read from, when one is configured.
    pub mapping_directory: Option<PathBuf>,
    /// The directory the OMOCL mapping files are read from, when one is
    /// configured.
    pub omocl_directory: Option<PathBuf>,
    /// The mapping set the operations compile from disk, when both
    /// directories are set.
    ///
    /// With `None` and a [`Settings::mapping_directory`], the operations take
    /// their templates from the CDR instead.
    pub mappings: Option<MappingSettings>,
    /// The facade lane, when it is on.
    pub facade: Option<FacadeSettings>,
    /// The HL7 v2 face, when it is on.
    pub hl7v2: Option<Hl7v2Settings>,
    /// The FHIRconnect operations lane.
    pub operations: OperationsSettings,
}

/// The OMOP CDM database, resolved.
#[derive(Debug, Clone)]
pub struct CdmSettings {
    /// The PostgreSQL connection URL, which may carry a password.
    pub url: SecretString,
    /// The connection both CDM clients open, with its TLS settled.
    pub connection: omop_cdm::connection::CdmConnection,
    /// The schema the CDM tables live in.
    pub schema: omop_cdm::ddl::SchemaName,
    /// The schema the natural-key side table lives in.
    pub bridge_schema: omop_cdm::ddl::SchemaName,
    /// What the writer does for an EHR with no `PERSON`.
    pub person_policy: omop_cdm::writer::input::PersonPolicy,
}

/// The mapping set, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingSettings {
    /// The directory holding the FHIRconnect mapping files.
    pub directory: PathBuf,
    /// The directory holding the operational templates.
    pub templates: PathBuf,
}

/// The FHIRconnect operations lane, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationsSettings {
    /// Whether the two operations are served.
    pub enabled: bool,
    /// The `Device` reference the `Provenance` `agent.who` defaults to.
    pub device_reference: String,
    /// The composer an inbound run fills in when no mapping does.
    pub composer: String,
    /// The composition language, as an ISO 639-1 code.
    pub composition_language: Option<String>,
    /// The composition territory, as an ISO 3166-1 alpha-2 code.
    pub composition_territory: Option<String>,
}

/// The HTTP surface, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettings {
    /// The socket address to bind.
    pub listen: SocketAddr,
    /// How long one request may take before the server answers `408`.
    pub request_timeout: Duration,
    /// How long the drain may take after the stop signal.
    pub shutdown_timeout: Duration,
    /// The largest request body the server reads before answering `413`.
    pub body_limit: usize,
}

/// The console, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySettings {
    /// The rendering.
    pub format: Format,
    /// The `tracing` filter directive.
    pub filter: String,
    /// The query parameter names whose values may reach the request log.
    pub logged_query_parameters: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{Config, Error};
    use std::collections::BTreeMap;

    #[test]
    fn an_absent_file_and_an_empty_environment_leave_the_defaults() {
        let config = Config::from_sources(None, &BTreeMap::new()).expect("the defaults parse");
        assert_eq!(Config::default(), config);
        assert_eq!("127.0.0.1:8080", config.server.listen);
        assert_eq!(1024 * 1024, config.server.body_limit_bytes);
        assert_eq!(10_000, config.server.shutdown_timeout_ms);
        assert!(config.telemetry.logged_query_parameters.is_empty());
    }

    #[test]
    fn an_override_under_a_value_that_is_not_a_section_is_refused() {
        let environment = BTreeMap::from([(
            String::from("FERROBRIDGE__SERVER__LISTEN__PORT"),
            String::from("1"),
        )]);
        let error =
            Config::from_sources(Some("[server]\nlisten = \"127.0.0.1:1\"\n"), &environment)
                .expect_err("listen is a string, not a section");
        assert!(matches!(error, Error::EnvShape { .. }), "{error:?}");
    }

    #[test]
    fn an_unknown_key_refuses_to_boot_and_the_message_names_it() {
        let error = Config::from_sources(Some("[server]\nlisten_port = 8080\n"), &BTreeMap::new())
            .expect_err("an unknown key is refused");
        assert!(
            format!("{:?}", std::error::Error::source(&error)).contains("listen_port"),
            "the refusal names the key: {error:?}"
        );
    }
}
