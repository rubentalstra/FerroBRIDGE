// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Configuration: an optional TOML file, then environment overrides, then one
//! typed [`Settings`] the run path holds.
//!
//! Every struct refuses an unknown key, every default lives inline in its own
//! `Default` impl, and every credential is reachable through a `<key>_file`
//! sibling read at boot. A lane is off until its section is present. No
//! specification governs the configuration: our own design.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::telemetry::{DEFAULT_FILTER, FILTER_ENV, FORMAT_ENV, Format};

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

/// The mapping set this deployment runs.
///
/// One tree of FHIRconnect mapping files and, optionally, one directory of the
/// operational templates the two operations compile against. With no
/// `templates`, the operations take each template a context names from the
/// `[cdr]`, and a `directory` with neither is refused at boot. Everything is
/// read once at boot, so a mapping that does not compile is a refusal to start
/// rather than a failure on the request that first touches it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mappings {
    /// The directory the FHIRconnect files are read from, recursively.
    pub directory: Option<PathBuf>,
    /// The directory holding the operational templates, as OPT 1.4 XML.
    ///
    /// When set, it wins over the `[cdr]` for the two operations; the facade
    /// always takes its templates from the CDR.
    pub templates: Option<PathBuf>,
    /// The directory the OMOCL files `etl run` maps with are read from,
    /// recursively.
    pub omocl: Option<PathBuf>,
}

/// The FHIR R4 facade lane.
///
/// The lane is off until `enabled` is set, and a disabled facade mounts no
/// route, so a request answers `404` rather than `403`
/// (`docs/architecture.md` §4.6).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Facade {
    /// Whether the facade routes are mounted.
    pub enabled: bool,
    /// The absolute FHIR service base a client reaches this server at.
    pub base_url: String,
    /// The EHR policy: `existing` or `create_on_first_write`.
    pub ehr_policy: String,
    /// The file the identity map is kept in.
    pub identity_store: PathBuf,
    /// The namespace a literal subject reference is read in.
    pub subject_namespace: String,
    /// The `AUDIT_DETAILS.system_id` every commit records.
    pub system_id: String,
    /// The `COMPOSITION.language` every commit carries, ISO 639-1.
    ///
    /// The mapping specification puts this field on "the project performing
    /// the mapping" (`engine/defaults-for-fields.adoc`), and a clinical record
    /// has no correct default language, so the key is empty until a deployment
    /// sets it and an enabled facade refuses to start without it.
    pub composition_language: String,
    /// The `COMPOSITION.territory` every commit carries, ISO 3166-1.
    pub composition_territory: String,
}

impl Default for Facade {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::from("http://127.0.0.1:8080/fhir"),
            ehr_policy: String::from("existing"),
            identity_store: PathBuf::from("identity.redb"),
            subject_namespace: String::from("ferrobridge"),
            system_id: String::from("FerroBRIDGE"),
            composition_language: String::new(),
            composition_territory: String::new(),
        }
    }
}

/// The HL7 v2 face: an MLLP listener whose messages enter the facade's
/// ingest service.
///
/// The face is off until `enabled` is set, and it needs the facade: the
/// ingest service, the identity map and the programs it writes through are
/// the facade's. No specification governs this configuration: our own
/// design.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hl7v2 {
    /// Whether the MLLP listener runs.
    pub enabled: bool,
    /// The socket address the MLLP listener binds.
    pub listen: String,
    /// The HL7 table 0211 code a message with an empty MSH-18 is read in.
    pub default_charset: String,
    /// The directory the v2-to-FHIR `ConceptMap` files are read from: the
    /// `package` directory of `hl7.fhir.uv.v2mappings`.
    pub concept_maps: Option<PathBuf>,
    /// Directories of supplement `ConceptMap` files, loaded over the guide in
    /// order; a map with the url of a loaded one replaces it.
    pub supplements: Vec<PathBuf>,
    /// What an entry no program maps does to the message: `skip_and_count`
    /// or `refuse`.
    pub unmapped_entries: String,
    /// The EHR policy of the face, `existing` or `create_on_first_write`;
    /// unset, the facade's.
    pub ehr_policy: Option<String>,
    /// The profile each resource type claims when the message map wrote none,
    /// so the facade's program selection reaches the context that claims it.
    pub profiles: Vec<Hl7v2Profile>,
    /// How long a connection may sit between frames before it is closed; `0`
    /// never closes it.
    pub idle_timeout_ms: u64,
    /// How long one frame may take from its first byte to its trailer before
    /// it is answered `AR`; `0` waits for the trailer.
    pub frame_timeout_ms: u64,
    /// The largest message one frame may carry, in bytes.
    pub frame_limit_bytes: usize,
    /// Whether an `AE` or an `AR` also logs, at debug level, the counted
    /// outcomes of the run by kind.
    pub log_outcomes: bool,
    /// The sending facilities (MSH-4) the face accepts; empty accepts any.
    pub senders: Vec<Hl7v2Sender>,
}

impl Default for Hl7v2 {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: String::from("127.0.0.1:2575"),
            default_charset: String::from("ASCII"),
            concept_maps: None,
            supplements: Vec::new(),
            unmapped_entries: String::from("skip_and_count"),
            ehr_policy: None,
            profiles: Vec::new(),
            idle_timeout_ms: 300_000,
            frame_timeout_ms: 30_000,
            frame_limit_bytes: 1024 * 1024,
            log_outcomes: false,
            senders: Vec::new(),
        }
    }
}

/// One sending facility the HL7 v2 face accepts: a namespace id alone, or a
/// universal id with its type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hl7v2Sender {
    /// HD.1 of MSH-4.
    pub namespace_id: String,
    /// HD.2 of MSH-4.
    pub universal_id: String,
    /// HD.3 of MSH-4, such as `ISO`.
    pub universal_id_type: String,
}

/// The profile one resource type of a mapped message claims.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hl7v2Profile {
    /// The R4 resource type, such as `Observation`.
    pub resource_type: String,
    /// The canonical URL written into `meta.profile`.
    pub profile: String,
}

/// The FHIRconnect operations lane.
///
/// The two operations are pure transformations that reach no CDR on a
/// request, so they are served whenever `[mappings] directory` is set and its
/// templates load, from `[mappings] templates` or, at boot, from the `[cdr]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Operations {
    /// Whether the two operations are served.
    pub enabled: bool,
    /// The `Device` reference the `Provenance` `agent.who` carries when a call
    /// supplies no `context.who`.
    pub device_reference: String,
    /// The composer an inbound run fills in when no mapping does.
    pub composer: String,
    /// The composition language, as an ISO 639-1 code.
    pub composition_language: Option<String>,
    /// The composition territory, as an ISO 3166-1 alpha-2 code.
    pub composition_territory: Option<String>,
}

impl Default for Operations {
    fn default() -> Self {
        Self {
            enabled: true,
            device_reference: format!(
                "Device/{}-{}",
                crate::state::PRODUCT.to_ascii_lowercase(),
                crate::state::VERSION
            ),
            composer: String::from("FHIRconnect"),
            composition_language: None,
            composition_territory: None,
        }
    }
}

/// The HTTP surface.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Server {
    /// The socket address to bind.
    pub listen: String,
    /// How long one request may take before the server answers `408`.
    pub request_timeout_ms: u64,
    /// How long the drain may take after the stop signal.
    pub shutdown_timeout_ms: u64,
    /// The largest request body the server reads before answering `413`.
    pub body_limit_bytes: usize,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            listen: String::from("127.0.0.1:8080"),
            request_timeout_ms: 30_000,
            shutdown_timeout_ms: 10_000,
            body_limit_bytes: 1024 * 1024,
        }
    }
}

/// The console.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Telemetry {
    /// The rendering: `auto`, `json` or `pretty`.
    pub format: Format,
    /// The `tracing` filter directive.
    pub filter: String,
    /// The query parameter names whose values may reach the request log.
    ///
    /// Empty by default, so no query value is ever logged until a deployment
    /// names a parameter it knows carries no identifiable content.
    pub logged_query_parameters: Vec<String>,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            format: Format::Auto,
            filter: String::from(DEFAULT_FILTER),
            logged_query_parameters: Vec::new(),
        }
    }
}

/// The openEHR CDR lane.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cdr {
    /// The openEHR REST API root, the path `/ehr` and `/query` live under.
    pub base_url: String,
    /// How long one call may take, connection included.
    pub timeout_ms: u64,
    /// The retry budget for idempotent calls.
    pub retry: Retry,
    /// The credentials the CDR expects.
    pub credentials: Credentials,
}

impl Default for Cdr {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            timeout_ms: 30_000,
            retry: Retry::default(),
            credentials: Credentials::default(),
        }
    }
}

/// The FHIR terminology lane.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Terminology {
    /// The FHIR service base URL.
    pub base_url: String,
    /// The release the server answers in: `r4` or `r4b`.
    pub wire_version: String,
    /// How long one call may take, connection included.
    pub timeout_ms: u64,
    /// The retry budget.
    pub retry: Retry,
    /// The credentials the server expects.
    pub credentials: Credentials,
}

impl Default for Terminology {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            wire_version: String::from("r4"),
            timeout_ms: 30_000,
            retry: Retry::default(),
            credentials: Credentials::default(),
        }
    }
}

/// The OMOP CDM database lane.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cdm {
    /// The PostgreSQL connection URL.
    pub url: Option<String>,
    /// A file holding the connection URL, read at boot.
    pub url_file: Option<PathBuf>,
    /// The PEM CA the database's certificate is checked against, for the
    /// URL's `sslmode` of `require`, `verify-ca` or `verify-full`.
    pub tls_ca: Option<String>,
    /// A file holding that CA, read at boot.
    pub tls_ca_file: Option<PathBuf>,
    /// The schema the CDM tables live in.
    pub schema: String,
    /// The schema the bridge keeps its natural-key side table in.
    pub bridge_schema: String,
    /// What the writer does for an EHR with no `PERSON`:
    /// `create_on_first_sight` or `existing`.
    pub person_policy: String,
}

impl Default for Cdm {
    fn default() -> Self {
        Self {
            url: None,
            url_file: None,
            tls_ca: None,
            tls_ca_file: None,
            schema: String::from("cdm"),
            bridge_schema: String::from("ferrobridge"),
            person_policy: String::from("create_on_first_sight"),
        }
    }
}

/// The OMOP ETL job.
///
/// The queries are parsed and checked when the configuration loads, so a
/// query the runner cannot read refuses the start of any job.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Etl {
    /// The AQL that selects each composition whole, aliased `ehr_id`,
    /// `versioned_object_uid`, `version_uid` and `composition`.
    pub aql: String,
    /// How many rows one page of a query asks for.
    pub page_size: u32,
    /// The `*_type_concept_id` the mappings write; no default, because it
    /// records a provenance only the deployment knows.
    pub type_concept_id: Option<i32>,
    /// The `period_type_concept_id` of every observation period.
    pub observation_period_type_concept_id: Option<i32>,
    /// The visit derivation, off until its section is present.
    pub visits: Option<EtlVisits>,
}

impl Default for Etl {
    fn default() -> Self {
        Self {
            aql: String::new(),
            page_size: 100,
            type_concept_id: None,
            observation_period_type_concept_id: None,
            visits: None,
        }
    }
}

/// The visit derivation of the OMOP ETL job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EtlVisits {
    /// The AQL aliased `ehr_id`, `visit_source`, `visit_start` and
    /// `visit_end`.
    pub aql: String,
    /// The `visit_concept_id` every visit carries.
    pub visit_concept_id: Option<i32>,
    /// The `visit_type_concept_id` every visit carries.
    pub visit_type_concept_id: Option<i32>,
}

/// How often, and how far apart, a call is retried.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Retry {
    /// How many times a call is sent at most, the first try included.
    pub max_attempts: u32,
    /// The delay before the second attempt.
    pub initial_backoff_ms: u64,
    /// The ceiling every later delay is clamped to.
    pub max_backoff_ms: u64,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 200,
            max_backoff_ms: 5_000,
        }
    }
}

/// The credentials one upstream expects.
///
/// Every secret is reachable inline or through its `_file` sibling; setting
/// both is a boot error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Credentials {
    /// An RFC 6750 bearer token.
    pub bearer_token: Option<String>,
    /// A file holding the bearer token, read at boot.
    pub bearer_token_file: Option<PathBuf>,
    /// The user name of RFC 7617 basic authentication.
    pub user: Option<String>,
    /// The password of RFC 7617 basic authentication.
    pub password: Option<String>,
    /// A file holding the password, read at boot.
    pub password_file: Option<PathBuf>,
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
fn resolve_mappings(mappings: &Mappings) -> Result<Option<MappingSettings>, Error> {
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

/// The facade lane, resolved.
#[derive(Debug, Clone)]
pub struct FacadeSettings {
    /// What the facade itself was configured with.
    pub settings: crate::facade::Settings,
    /// The file the identity map is kept in.
    pub identity_store: PathBuf,
}

/// Returns the facade settings `facade` describes.
fn resolve_facade(facade: &Facade) -> Result<FacadeSettings, Error> {
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
fn resolve_hl7v2(hl7v2: &Hl7v2, facade: bool) -> Result<Hl7v2Settings, Error> {
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

/// Applies one environment override onto `table`.
fn apply_override(table: &mut toml::Table, name: &str, raw: &str) -> Result<(), Error> {
    let Some(path) = name.strip_prefix(ENV_PREFIX) else {
        return Ok(());
    };
    let segments: Vec<String> = path.split("__").map(str::to_ascii_lowercase).collect();
    let Some((key, parents)) = segments.split_last() else {
        return Err(Error::EnvName {
            name: name.to_owned(),
        });
    };
    if key.is_empty() || parents.is_empty() || parents.iter().any(String::is_empty) {
        return Err(Error::EnvName {
            name: name.to_owned(),
        });
    }
    let mut cursor = table;
    for parent in parents {
        let entry = cursor
            .entry(parent.clone())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let toml::Value::Table(next) = entry else {
            return Err(Error::EnvShape {
                name: name.to_owned(),
            });
        };
        cursor = next;
    }
    cursor.insert(key.clone(), env_value(raw));
    Ok(())
}

/// Applies [`FORMAT_ENV`] and [`FILTER_ENV`] onto `[telemetry]`.
///
/// They run after every `FERROBRIDGE__` override, so they win over the file
/// and over `FERROBRIDGE__TELEMETRY__FORMAT` and `FERROBRIDGE__TELEMETRY__FILTER`:
/// they are the names an operator sets for one run. Each value is text, so a
/// format outside `auto`, `json` and `pretty` is refused by the parse that
/// follows, naming `telemetry.format`.
fn apply_console_overrides(
    table: &mut toml::Table,
    environment: &BTreeMap<String, String>,
) -> Result<(), Error> {
    for (variable, key) in [(FORMAT_ENV, "format"), (FILTER_ENV, "filter")] {
        let Some(raw) = environment.get(variable) else {
            continue;
        };
        let entry = table
            .entry("telemetry")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let toml::Value::Table(telemetry) = entry else {
            return Err(Error::EnvShape {
                name: variable.to_owned(),
            });
        };
        telemetry.insert(key.to_owned(), toml::Value::String(raw.clone()));
    }
    Ok(())
}

/// Reads `raw` as a TOML value, or as the string it is.
///
/// An environment variable carries text, so a number, a boolean and an array
/// are spelled in TOML syntax and everything else is the string itself. A
/// value that would read as another type is quoted.
fn env_value(raw: &str) -> toml::Value {
    // NOTE: no specification governs this: our own design. A parse failure IS
    // the answer here, because text that is not TOML syntax is a plain string.
    let parsed = toml::from_str::<toml::Table>(&format!("value = {raw}"))
        .ok()
        .and_then(|table| table.get("value").cloned());
    parsed.unwrap_or_else(|| toml::Value::String(raw.to_owned()))
}

/// Returns the CDR client configuration `cdr` describes.
fn resolve_cdr(cdr: &Cdr) -> Result<crate::cdr::config::CdrConfig, Error> {
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
fn resolve_terminology(
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
fn resolve_cdm(cdm: &Cdm) -> Result<CdmSettings, Error> {
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
fn required(key: &str, value: Option<i32>) -> Result<i32, Error> {
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
fn resolve_etl(etl: &Etl) -> Result<crate::etl::EtlSettings, Error> {
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

/// The authentication scheme a credentials section resolves to.
enum Scheme {
    /// An RFC 6750 bearer token.
    Bearer(SecretString),
    /// RFC 7617 basic authentication.
    Basic {
        /// The user name, which is not a secret.
        user: String,
        /// The password.
        password: SecretString,
    },
}

/// Returns the scheme `credentials` describes, or `None` when it names none.
fn resolve_credentials(section: &str, credentials: &Credentials) -> Result<Option<Scheme>, Error> {
    let token = secret(
        &format!("{section}.bearer_token"),
        credentials.bearer_token.as_deref(),
        credentials.bearer_token_file.as_deref(),
    )?;
    let password = secret(
        &format!("{section}.password"),
        credentials.password.as_deref(),
        credentials.password_file.as_deref(),
    )?;
    match (token, credentials.user.as_deref(), password) {
        (Some(_), Some(_), _) | (Some(_), None, Some(_)) => Err(Error::Scheme {
            section: section.to_owned(),
        }),
        (Some(token), None, None) => Ok(Some(Scheme::Bearer(token))),
        (None, Some(user), Some(password)) => Ok(Some(Scheme::Basic {
            user: user.to_owned(),
            password,
        })),
        (None, Some(_), None) => Err(Error::Missing {
            key: format!("{section}.password"),
        }),
        (None, None, Some(_)) => Err(Error::Missing {
            key: format!("{section}.user"),
        }),
        (None, None, None) => Ok(None),
    }
}

/// Returns the key whose value `error` refuses: the CA's for a CA fault, the
/// URL's for every other.
fn cdm_connection_key(error: &omop_cdm::connection::ConnectionError, cdm: &Cdm) -> &'static str {
    use omop_cdm::connection::ConnectionError;
    match error {
        ConnectionError::NoCertificate
        | ConnectionError::Pem { .. }
        | ConnectionError::Root { .. }
        | ConnectionError::CaWithoutTls => {
            if cdm.tls_ca_file.is_some() {
                "cdm.tls_ca_file"
            } else {
                "cdm.tls_ca"
            }
        }
        _ => {
            if cdm.url_file.is_some() {
                "cdm.url_file"
            } else {
                "cdm.url"
            }
        }
    }
}

/// Returns the secret `key` names, inline or from its `_file` sibling.
fn secret(
    key: &str,
    inline: Option<&str>,
    file: Option<&Path>,
) -> Result<Option<SecretString>, Error> {
    match (inline, file) {
        (Some(_), Some(_)) => Err(Error::Conflict {
            key: key.to_owned(),
        }),
        (Some(value), None) => Ok(Some(SecretString::from(value))),
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path).map_err(|source| Error::Secret {
                key: format!("{key}_file"),
                path: path.to_path_buf(),
                source,
            })?;
            Ok(Some(SecretString::from(text.trim())))
        }
        (None, None) => Ok(None),
    }
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

#[cfg(test)]
mod tests {
    use super::{Config, Error, apply_override, env_value};
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
    fn an_environment_value_reads_as_toml_syntax_or_as_the_string_it_is() {
        assert_eq!(toml::Value::Integer(5), env_value("5"));
        assert_eq!(toml::Value::Boolean(true), env_value("true"));
        assert_eq!(
            toml::Value::String(String::from("http://cdr.invalid/v1")),
            env_value("http://cdr.invalid/v1")
        );
        assert_eq!(
            toml::Value::String(String::from("0.0.0.0:8080")),
            env_value("0.0.0.0:8080")
        );
    }

    #[test]
    fn an_override_name_without_a_section_and_a_key_is_refused() {
        let mut table = toml::Table::new();
        let error = apply_override(&mut table, "FERROBRIDGE__LISTEN", "x")
            .expect_err("a section and a key are both required");
        assert!(matches!(error, Error::EnvName { .. }), "{error:?}");
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
