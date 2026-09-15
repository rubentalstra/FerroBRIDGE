// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Configuration: an optional TOML file, then environment overrides, then one
//! typed [`Settings`] the run path holds.
//!
//! Every struct refuses an unknown key, every default lives inline in its own
//! `Default` impl, and every credential is reachable through a `<key>_file`
//! sibling read at boot. A lane is off until its section is present. No
//! specification governs the configuration: our own design.

use secrecy::SecretString;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::telemetry::{DEFAULT_FILTER, Format};

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
    /// Where the mapping files are read from.
    pub mappings: Mappings,
    /// The FHIR facade, off until its section turns it on.
    pub facade: Facade,
}

/// Where the mapping files this deployment runs are read from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Mappings {
    /// The directory the FHIRconnect files are read from, recursively.
    pub directory: Option<PathBuf>,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cdm {
    /// The PostgreSQL connection URL.
    pub url: Option<String>,
    /// A file holding the connection URL, read at boot.
    pub url_file: Option<PathBuf>,
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
            .filter(|(name, _)| name.starts_with(ENV_PREFIX))
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
    /// [`Error::Client`]) each naming the key that carries the fault.
    pub fn resolve(&self) -> Result<Settings, Error> {
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
            cdm_url: self.cdm.as_ref().map(resolve_cdm).transpose()?,
            mapping_directory: self.mappings.directory.clone(),
            facade: if self.facade.enabled {
                Some(resolve_facade(&self.facade)?)
            } else {
                None
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

/// Returns the facade settings `facade` describes.
fn resolve_facade(facade: &Facade) -> Result<FacadeSettings, Error> {
    let ehr_policy = match facade.ehr_policy.as_str() {
        "existing" => crate::facade::ehr::Policy::Existing,
        "create_on_first_write" => crate::facade::ehr::Policy::CreateOnFirstWrite,
        _ => {
            return Err(Error::EhrPolicy {
                key: String::from("facade.ehr_policy"),
                value: facade.ehr_policy.clone(),
            });
        }
    };
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

/// The settings the run path holds, with every secret already read.
#[derive(Debug)]
pub struct Settings {
    /// The HTTP surface.
    pub server: ServerSettings,
    /// The console.
    pub telemetry: TelemetrySettings,
    /// The CDR client configuration, when the lane is on.
    pub cdr: Option<ferrobridge_openehr::config::Config>,
    /// The terminology client configuration, when the lane is on.
    pub terminology: Option<ferrobridge_term::config::Config>,
    /// The OMOP CDM connection URL, when the lane is on.
    pub cdm_url: Option<SecretString>,
    /// The directory the mapping files are read from, when one is configured.
    pub mapping_directory: Option<PathBuf>,
    /// The facade lane, when it is on.
    pub facade: Option<FacadeSettings>,
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

impl Settings {
    /// Logs which lanes are on, and which of them carry identifiable data.
    ///
    /// The line names the section and never a value, so a start-up log states
    /// what this process can reach without stating any of it.
    pub fn log_lanes(&self) {
        if let Some(cdr) = self.cdr.as_ref() {
            tracing::info!(
                credentials = cdr.credentials.is_some(),
                "[cdr] is configured and carries identifiable data"
            );
        } else {
            tracing::info!("no [cdr] section: the CDR lane is off");
        }
        if self.terminology.is_some() {
            tracing::info!("[terminology] is configured");
        } else {
            tracing::info!("no [terminology] section: the terminology lane is off");
        }
        if self.cdm_url.is_some() {
            tracing::info!("[cdm] is configured and carries identifiable data");
        } else {
            tracing::info!("no [cdm] section: the OMOP lane is off");
        }
        if self.facade.is_some() {
            tracing::info!("[facade] is enabled and carries identifiable data");
        } else {
            tracing::info!("[facade] is not enabled: the FHIR facade mounts no route");
        }
    }
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
fn resolve_cdr(cdr: &Cdr) -> Result<ferrobridge_openehr::config::Config, Error> {
    let base_url = url_of("cdr.base_url", &cdr.base_url)?;
    let mut config = ferrobridge_openehr::config::Config::new(base_url)
        .with_timeout(Duration::from_millis(cdr.timeout_ms))
        .with_retry(ferrobridge_openehr::config::RetryPolicy {
            max_attempts: cdr.retry.max_attempts,
            initial_backoff: Duration::from_millis(cdr.retry.initial_backoff_ms),
            max_backoff: Duration::from_millis(cdr.retry.max_backoff_ms),
        });
    if let Some(scheme) = resolve_credentials("cdr.credentials", &cdr.credentials)? {
        config = config.with_credentials(match scheme {
            Scheme::Bearer(token) => ferrobridge_openehr::config::Credentials::Bearer(token),
            Scheme::Basic { user, password } => {
                ferrobridge_openehr::config::Credentials::Basic { user, password }
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

/// Returns the CDM connection URL `cdm` describes.
fn resolve_cdm(cdm: &Cdm) -> Result<SecretString, Error> {
    // TODO(#91): hand this URL to the CDM writer, which owns the connection.
    secret("cdm.url", cdm.url.as_deref(), cdm.url_file.as_deref())?.ok_or(Error::Missing {
        key: String::from("cdm.url"),
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
