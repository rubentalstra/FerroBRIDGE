// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR CDR, reached through the generated ITS-REST 1.1.0 client of
//! `openehr-its`.
//!
//! The generated per-group clients (`openehr_its::rest::generated::<group>::client`)
//! own every operation: the path, the parameters, `Prefer`, `If-Match`, the
//! documented statuses as one outcome enum per call, and the retry over
//! idempotent methods. This module holds what the bridge adds on top: the
//! handle built from `[cdr]`, the committal metadata values, the
//! `X-Request-Id` echo, the upstream answer kept for the response headers the
//! generated outcomes do not declare, the typed identifiers an `ETag` names,
//! the two-route template fetch and the AQL paging.
//!
//! openEHR is a registered trademark of the openEHR Foundation.

pub mod commit;
pub mod config;
pub mod error;
pub mod ids;
pub mod query;
pub mod template;
mod transport;

use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use http::StatusCode;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_its::rest::client::Client;
use openehr_its::rest::client::ClientError;
use openehr_its::rest::client::Credentials;
use openehr_its::rest::client::ReqwestTransport;
use openehr_its::rest::client::RetryPolicy;
use openehr_its::rest::generated::common::Identifier;
use openehr_its::rest::generated::ehr::CompositionCreateParams;
use openehr_its::rest::generated::ehr::CompositionDeleteParams;
use openehr_its::rest::generated::ehr::CompositionGetParams;
use openehr_its::rest::generated::ehr::CompositionUpdateParams;
use openehr_its::rest::generated::ehr::ContributionCreateParams;
use openehr_its::rest::generated::ehr::ContributionGetParams;
use openehr_its::rest::generated::ehr::EhrCreateParams;
use openehr_its::rest::generated::ehr::EhrGetByIdParams;
use openehr_its::rest::generated::ehr::EhrGetBySubjectParams;
use openehr_its::rest::generated::ehr::NewContribution;
use openehr_its::rest::generated::ehr::client::CompositionCreateOutcome;
use openehr_its::rest::generated::ehr::client::CompositionDeleteOutcome;
use openehr_its::rest::generated::ehr::client::CompositionGetOutcome;
use openehr_its::rest::generated::ehr::client::CompositionUpdateOutcome;
use openehr_its::rest::generated::ehr::client::ContributionCreateOutcome;
use openehr_its::rest::generated::ehr::client::ContributionGetOutcome;
use openehr_its::rest::generated::ehr::client::EhrClient;
use openehr_its::rest::generated::ehr::client::EhrCreateOutcome;
use openehr_its::rest::generated::ehr::client::EhrGetByIdOutcome;
use openehr_its::rest::generated::ehr::client::EhrGetBySubjectOutcome;
use openehr_rm::v1_2::common::change_control::contribution::Contribution;
use openehr_rm::v1_2::composition::composition::Composition;
use openehr_rm::v1_2::ehr::ehr_status::EhrStatus;
use serde::de::DeserializeOwned;

use crate::cdr::commit::CommitContext;
use crate::cdr::config::CdrConfig;
use crate::cdr::error::CdrError;
use crate::cdr::error::Upstream;
use crate::cdr::ids::ContributionUid;
use crate::cdr::ids::EhrId;
use crate::cdr::ids::IdError;
use crate::cdr::ids::RequestId;
use crate::cdr::ids::SubjectId;
use crate::cdr::ids::SubjectNamespace;
use crate::cdr::transport::Scoped;

/// The openEHR ITS-REST release the bridge speaks.
///
/// The release is published at <https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/>.
pub const ITS_REST_VERSION: &str = "1.1.0";

/// The correlation header the bridge echoes onto every CDR call.
const REQUEST_ID: &str = "x-request-id";

/// The `Prefer` request header name.
const PREFER: &str = "prefer";

/// What the caller wants back from a state-changing call.
///
/// ITS-REST 1.1.0 §Requests and responses/HTTP headers/Prefer defines the
/// three values and tells clients to send one explicitly: "Although the
/// current default behavior is equivalent to `Prefer=minimal`, this might
/// change in the near future to `Prefer=identifier`." The bridge therefore
/// never omits the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Prefer {
    /// No body: the answer is the status, the `ETag` and the `Location`.
    Minimal,
    /// Only the identifier of the affected resource.
    Identifier,
    /// The full resource representation.
    Representation,
}

impl Prefer {
    /// Returns the header value this preference travels as.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "return=minimal",
            Self::Identifier => "return=identifier",
            Self::Representation => "return=representation",
        }
    }

    /// Returns the header value as the generated parameter structs carry it.
    fn param(self) -> String {
        String::from(self.as_str())
    }
}

/// What a successful state-changing call returned, per the [`Prefer`] it was
/// asked with.
#[derive(Debug, Clone)]
pub enum Returned<T> {
    /// [`Prefer::Minimal`]: the service sent no representation.
    Minimal,
    /// [`Prefer::Identifier`]: the service sent the resource identifier.
    Identifier(Identifier),
    /// [`Prefer::Representation`]: the service sent the whole resource.
    Representation(Box<T>),
}

/// A documented answer of one operation, with the upstream answer it was read
/// from.
///
/// `outcome` is the generated outcome enum of the operation, whose refusal
/// variants carry the error body; `upstream` keeps the status, the headers and
/// the body the CDR sent, because a generated outcome carries only the
/// response headers the `OpenAPI` documents declare for it.
#[derive(Debug)]
pub struct Answered<O> {
    /// The generated outcome.
    pub outcome: O,
    /// The status and body the outcome was read from.
    pub upstream: Upstream,
}

/// A client for one openEHR CDR.
///
/// The client is cheap to clone: the HTTP engine and its connection pool are
/// shared, so a clone carries the same pool.
#[derive(Debug, Clone)]
pub struct CdrClient {
    /// The configured generated-client runtime.
    client: Client<ReqwestTransport>,
    /// The credentials the runtime was given, which a scoped call repeats.
    credentials: Option<Credentials>,
    /// The correlation identifier to echo, when the caller set one.
    request_id: Option<RequestId>,
}

impl CdrClient {
    /// Returns a client for the CDR `config` describes.
    ///
    /// # Errors
    /// Returns [`CdrError::Build`] when the HTTP engine refuses the
    /// configuration, and [`CdrError::Client`] when the base URL cannot carry
    /// a path.
    pub fn new(config: &CdrConfig) -> Result<Self, CdrError> {
        let transport = ReqwestTransport::with_timeout(config.timeout)
            .map_err(|source| CdrError::Build { source })?;
        let credentials = config.credentials.clone();
        let mut client = Client::new(transport, config.base_url.clone())
            .map_err(|source| CdrError::client(source, None))?
            .with_retry(config.retry);
        if let Some(credentials) = credentials.clone() {
            client = client.with_credentials(credentials);
        }
        Ok(Self {
            client,
            credentials,
            request_id: None,
        })
    }

    /// Returns this client with `request_id` echoed into `X-Request-Id`.
    #[must_use]
    pub fn with_request_id(&self, request_id: RequestId) -> Self {
        Self {
            client: self.client.clone(),
            credentials: self.credentials.clone(),
            request_id: Some(request_id),
        }
    }

    /// Returns the openEHR REST API root every call is resolved under.
    #[must_use]
    pub fn base_url(&self) -> &url::Url {
        self.client.base()
    }

    /// Returns the configured generated-client runtime.
    #[must_use]
    pub const fn runtime(&self) -> &Client<ReqwestTransport> {
        &self.client
    }

    /// Returns the retry budget the runtime was configured with.
    #[must_use]
    pub fn retry(&self) -> RetryPolicy {
        self.client.retry()
    }

    /// Returns a runtime for one call that adds `headers` to every attempt and
    /// keeps the last answer.
    fn scoped(
        &self,
        headers: HeaderMap,
        retry: RetryPolicy,
    ) -> Result<Client<Scoped<'_>>, CdrError> {
        let mut extra = headers;
        if let Some(request_id) = self.request_id.as_ref() {
            let value = HeaderValue::from_str(request_id.as_str()).map_err(|source| {
                CdrError::HeaderValue {
                    header: REQUEST_ID,
                    source,
                }
            })?;
            extra.insert(HeaderName::from_static(REQUEST_ID), value);
        }
        let transport = Scoped::new(self.client.transport(), extra);
        let mut client = Client::new(transport, self.client.base().clone())
            .map_err(|source| CdrError::client(source, None))?
            .with_retry(retry);
        if let Some(credentials) = self.credentials.clone() {
            client = client.with_credentials(credentials);
        }
        Ok(client)
    }

    /// Returns a runtime for one call under the configured retry budget.
    fn call(&self, headers: HeaderMap) -> Result<Client<Scoped<'_>>, CdrError> {
        self.scoped(headers, self.client.retry())
    }

    /// Returns the status the service answers a `GET` on its base URL with.
    ///
    /// The call is sent once, with the configured credentials and timeout, and
    /// every status the service produces is returned as it stands, `401` and
    /// `5xx` included: the question is whether the service answered, and a
    /// caller reading reachability decides what each status means.
    ///
    /// # Errors
    /// Returns [`CdrError::Client`] when the request never reached the
    /// service, the configured timeout included.
    pub async fn reachability(&self) -> Result<StatusCode, CdrError> {
        let once = RetryPolicy {
            max_attempts: 1,
            ..self.client.retry()
        };
        let client = self.scoped(HeaderMap::new(), once)?;
        let request = openehr_its::rest::client::Request::new(http::Method::GET, String::new());
        let answered = client.execute(request).await;
        let status = match answered {
            Ok(answer) => answer.status(),
            Err(ClientError::Unauthorized { .. }) => StatusCode::UNAUTHORIZED,
            Err(ClientError::Forbidden { .. }) => StatusCode::FORBIDDEN,
            Err(ClientError::ServiceFailure { status, .. }) => status,
            Err(source) => return Err(CdrError::client(source, None)),
        };
        tracing::debug!(status = %status, "the openEHR service answered a probe");
        Ok(status)
    }

    /// Creates an EHR with a server-assigned identifier.
    ///
    /// `status` is the optional `EHR_STATUS` the service commits with the new
    /// EHR (`ehr-codegen.openapi.yaml`, `ehr_create`). The call is a `POST`,
    /// so it is never retried.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn create_ehr(
        &self,
        status: Option<&EhrStatus>,
        prefer: Prefer,
    ) -> Result<Answered<EhrCreateOutcome>, CdrError> {
        let client = self.call(HeaderMap::new())?;
        let params = EhrCreateParams {
            prefer: Some(prefer.param()),
            accept: None,
            content_type: None,
            openehr_version: None,
            openehr_audit_details: None,
        };
        let answered = EhrClient::new(&client).ehr_create(&params, status).await;
        answered_by(&client, answered)
    }

    /// Retrieves the EHR with `ehr_id` (`ehr-codegen.openapi.yaml`,
    /// `ehr_get_by_id`).
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn ehr(&self, ehr_id: &EhrId) -> Result<Answered<EhrGetByIdOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = EhrGetByIdParams {
            ehr_id: String::from(ehr_id.as_str()),
            accept: None,
        };
        let answered = EhrClient::new(&client).ehr_get_by_id(&params).await;
        answered_by(&client, answered)
    }

    /// Retrieves the EHR whose subject is `subject_id` in `namespace`.
    ///
    /// The parameters are matched against
    /// `EHR_STATUS.subject.external_ref.id.value` and
    /// `EHR_STATUS.subject.external_ref.namespace`
    /// (`ehr-codegen.openapi.yaml`, `ehr_get_by_subject`).
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn ehr_by_subject(
        &self,
        subject_id: &SubjectId,
        namespace: &SubjectNamespace,
    ) -> Result<Answered<EhrGetBySubjectOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = EhrGetBySubjectParams {
            subject_id: String::from(subject_id.as_str()),
            subject_namespace: String::from(namespace.as_str()),
            accept: None,
        };
        let answered = EhrClient::new(&client).ehr_get_by_subject(&params).await;
        answered_by(&client, answered)
    }

    /// Commits the first version of a composition into the EHR `ehr_id`.
    ///
    /// The body is canonical JSON, the mandatory composition representation of
    /// ITS-REST 1.1.0, and `commit` renders the committal metadata headers.
    /// The call is a `POST`, so it is never retried.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the commit metadata cannot travel in a header
    /// or the call did not reach a documented answer.
    pub async fn create_composition(
        &self,
        ehr_id: &EhrId,
        composition: &Composition,
        commit: &CommitContext,
        prefer: Prefer,
    ) -> Result<Answered<CompositionCreateOutcome>, CdrError> {
        let headers = commit.headers()?;
        let client = self.call(HeaderMap::new())?;
        let params = CompositionCreateParams {
            ehr_id: String::from(ehr_id.as_str()),
            prefer: Some(prefer.param()),
            accept: None,
            content_type: None,
            openehr_item_tag: None,
            openehr_version_item_tag: None,
            openehr_version: headers.version,
            openehr_audit_details: headers.audit_details,
            openehr_template_id: headers.template_id,
        };
        let answered = EhrClient::new(&client)
            .composition_create(&params, composition)
            .await;
        answered_by(&client, answered)
    }

    /// Commits a new version of the composition `versioned_object_uid`.
    ///
    /// `preceding` is the latest version the caller knows of; it travels as
    /// the `If-Match` header, quoted and without the `W/` an `ETag` carries.
    /// The call is idempotent under that precondition, so the retry budget
    /// applies.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the commit metadata cannot travel in a header
    /// or the call did not reach a documented answer.
    pub async fn update_composition(
        &self,
        ehr_id: &EhrId,
        versioned_object_uid: &HierObjectId,
        preceding: &ObjectVersionId,
        composition: &Composition,
        commit: &CommitContext,
        prefer: Prefer,
    ) -> Result<Answered<CompositionUpdateOutcome>, CdrError> {
        let headers = commit.headers()?;
        let client = self.call(HeaderMap::new())?;
        let params = CompositionUpdateParams {
            ehr_id: String::from(ehr_id.as_str()),
            uid_based_id: String::from(versioned_object_uid.value()),
            if_match: if_match_value(preceding),
            prefer: Some(prefer.param()),
            accept: None,
            content_type: None,
            openehr_item_tag: None,
            openehr_version_item_tag: None,
            openehr_version: headers.version,
            openehr_audit_details: headers.audit_details,
            openehr_template_id: headers.template_id,
        };
        let answered = EhrClient::new(&client)
            .composition_update(&params, composition)
            .await;
        answered_by(&client, answered)
    }

    /// Retrieves a version of a composition.
    ///
    /// "The `uid_based_id` can take a form of an `OBJECT_VERSION_ID` identifier
    /// taken from `VERSION.uid.value` …, or a form of a `HIER_OBJECT_ID`
    /// identifier taken from `VERSIONED_OBJECT.uid.value`"
    /// (`ehr-codegen.openapi.yaml`, `composition_get`). `at` selects the
    /// version extant at that time and is only meaningful when `uid_based_id`
    /// is a version container.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn composition(
        &self,
        ehr_id: &EhrId,
        uid_based_id: &UidBasedId,
        at: Option<&VersionAtTime>,
    ) -> Result<Answered<CompositionGetOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = CompositionGetParams {
            ehr_id: String::from(ehr_id.as_str()),
            uid_based_id: String::from(uid_based_id.value()),
            version_at_time: at.map(|at| String::from(at.as_str())),
            accept: None,
        };
        let answered = EhrClient::new(&client).composition_get(&params).await;
        answered_by(&client, answered)
    }

    /// Deletes the composition whose latest version is `preceding`.
    ///
    /// "The `uid_based_id` MUST be in a form of an `OBJECT_VERSION_ID`
    /// identifier taken from the last (most recent) `VERSION.uid.value`"
    /// (`ehr-codegen.openapi.yaml`, `composition_delete`), which is why this
    /// call takes a version identifier and is retryable.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn delete_composition(
        &self,
        ehr_id: &EhrId,
        preceding: &ObjectVersionId,
    ) -> Result<Answered<CompositionDeleteOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = CompositionDeleteParams {
            ehr_id: String::from(ehr_id.as_str()),
            uid_based_id: String::from(preceding.value()),
            openehr_version: None,
            openehr_audit_details: None,
        };
        let answered = EhrClient::new(&client).composition_delete(&params).await;
        answered_by(&client, answered)
    }

    /// Commits a CONTRIBUTION into the EHR `ehr_id`.
    ///
    /// The versions and their audits travel in the body, which is why this
    /// call sends no committal metadata header: the `NewContribution` schema
    /// carries `audit` and each `versions[i].commit_audit` itself
    /// (`ehr-codegen.openapi.yaml`, `contribution_create`). The call is a
    /// `POST`, so it is never retried.
    ///
    /// # Errors
    /// Returns [`CdrError::CommittedBody`], naming the committed contribution,
    /// when a `201` body is not JSON, and [`CdrError`] otherwise when the call
    /// did not reach a documented answer.
    pub async fn create_contribution(
        &self,
        ehr_id: &EhrId,
        contribution: &NewContribution,
        prefer: Prefer,
    ) -> Result<Answered<ContributionCreateOutcome>, CdrError> {
        let client = self.call(HeaderMap::new())?;
        let params = ContributionCreateParams {
            ehr_id: String::from(ehr_id.as_str()),
            prefer: Some(prefer.param()),
            accept: None,
            content_type: None,
            openehr_template_id: None,
        };
        let answered = EhrClient::new(&client)
            .contribution_create(&params, contribution)
            .await;
        match answered {
            Err(source @ ClientError::Body { status, .. }) if status == StatusCode::CREATED => {
                let upstream = client.transport().answered();
                let etag = upstream
                    .as_ref()
                    .and_then(|upstream| upstream.header("etag"));
                let contribution_uid =
                    contribution_uid_from_etag("contribution_create", status, etag.as_deref())?;
                Err(CdrError::CommittedBody {
                    contribution_uid,
                    source: Box::new(source),
                })
            }
            answered => answered_by(&client, answered),
        }
    }

    /// Retrieves the CONTRIBUTION `contribution_uid` of the EHR `ehr_id`
    /// (`ehr-codegen.openapi.yaml`, `contribution_get`). The call is a `GET`,
    /// so the retry budget applies.
    ///
    /// # Errors
    /// Returns [`CdrError`] when the call did not reach a documented answer.
    pub async fn contribution(
        &self,
        ehr_id: &EhrId,
        contribution_uid: &ContributionUid,
    ) -> Result<Answered<ContributionGetOutcome>, CdrError> {
        let client = self.call(representation())?;
        let params = ContributionGetParams {
            ehr_id: String::from(ehr_id.as_str()),
            contribution_uid: String::from(contribution_uid.as_str()),
            accept: None,
        };
        let answered = EhrClient::new(&client).contribution_get(&params).await;
        answered_by(&client, answered)
    }
}

/// A point in time a version is read at, in the extended ISO 8601 form.
///
/// The value travels as the `version_at_time` query parameter
/// (`ehr-codegen.openapi.yaml`, `components.parameters.version_at_time`); the
/// service interprets it, and the bridge only carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionAtTime(String);

impl VersionAtTime {
    /// Returns the instant `text` names.
    ///
    /// # Errors
    /// Returns [`IdError`] when `text` is empty or carries a control
    /// character.
    pub fn new(text: &str) -> Result<Self, IdError> {
        if text.is_empty() {
            return Err(IdError::Empty {
                kind: "version_at_time",
            });
        }
        if let Some(character) = text.chars().find(|c| c.is_control()) {
            return Err(IdError::ForbiddenCharacter {
                kind: "version_at_time",
                character,
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns the instant as it travels on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Returns the extra headers of a read.
///
/// A read asks for a representation, because ITS-REST 1.1.0 §Requests and
/// responses/HTTP status codes makes error details conditional on it:
/// "services MAY return additional error details if the `Prefer:
/// return=representation` header is present in the request".
fn representation() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(PREFER),
        HeaderValue::from_static(Prefer::Representation.as_str()),
    );
    headers
}

/// Returns the generated answer of `client` with the upstream answer it kept.
fn answered_by<O>(
    client: &Client<Scoped<'_>>,
    answered: Result<O, ClientError>,
) -> Result<Answered<O>, CdrError> {
    let upstream = client.transport().answered();
    match answered {
        Ok(outcome) => upstream
            .map(|upstream| Answered { outcome, upstream })
            .ok_or(CdrError::Unrecorded),
        Err(source) => Err(CdrError::client(source, upstream)),
    }
}

/// Returns the `If-Match` field value for `version_id`.
///
/// "The format is always an `version_uid` identifier enclosed by double
/// quotes" (`ehr-codegen.openapi.yaml`, `components.parameters.If-Match`), so
/// the weakness indicator an `ETag` carries never travels back.
#[must_use]
pub fn if_match_value(version_id: &ObjectVersionId) -> String {
    format!("\"{}\"", version_id.value())
}

/// Returns the version identifier the `ETag` of a `status` answer of
/// `operation` names.
///
/// # Errors
/// Returns [`CdrError::MissingHeader`] when the answer carries no `ETag`, and
/// [`CdrError::MalformedHeader`] when it names no `OBJECT_VERSION_ID`.
pub fn version_from_etag(
    operation: &'static str,
    status: StatusCode,
    etag: Option<&str>,
) -> Result<ObjectVersionId, CdrError> {
    let etag = etag.ok_or(CdrError::MissingHeader {
        operation,
        status,
        header: "ETag",
    })?;
    ids::version_from_etag(etag).map_err(|source| CdrError::MalformedHeader {
        operation,
        header: "ETag",
        source: Box::new(source),
    })
}

/// Returns the version identifier the `ETag` of an answer of `operation`
/// names, when it sent one.
///
/// The `412` and `409` answers SHOULD carry the latest `version_uid` in the
/// `ETag` (ITS-REST 1.1.0 §Requests and responses/HTTP headers/If-Match and
/// accidental overwrites), so an absent header is a legal answer and stays an
/// absent value.
///
/// # Errors
/// Returns [`CdrError::MalformedHeader`] when the `ETag` names no
/// `OBJECT_VERSION_ID`.
pub fn optional_version_from_etag(
    operation: &'static str,
    etag: Option<&str>,
) -> Result<Option<ObjectVersionId>, CdrError> {
    etag.map(|etag| {
        ids::version_from_etag(etag).map_err(|source| CdrError::MalformedHeader {
            operation,
            header: "ETag",
            source: Box::new(source),
        })
    })
    .transpose()
}

/// Returns the `ehr_id` the `ETag` of a `201` or `204` of `ehr_create` names.
///
/// "The `ETag` (i.e. entity tag) response header is the `ehr_id` identifier,
/// enclosed by double quotes" (`ehr-codegen.openapi.yaml`,
/// `components.headers.ETag_EHR`).
///
/// # Errors
/// Returns [`CdrError::MissingHeader`] when the answer carries no `ETag`, and
/// [`CdrError::MalformedHeader`] when it names no `ehr_id`.
pub fn ehr_id_from_etag(status: StatusCode, etag: Option<&str>) -> Result<EhrId, CdrError> {
    let etag = etag.ok_or(CdrError::MissingHeader {
        operation: "ehr_create",
        status,
        header: "ETag",
    })?;
    EhrId::new(ids::entity_tag(etag)).map_err(|source| CdrError::MalformedHeader {
        operation: "ehr_create",
        header: "ETag",
        source: Box::new(source),
    })
}

/// Returns the `contribution_uid` the `ETag` of a `status` answer of
/// `operation` names.
///
/// # Errors
/// Returns [`CdrError::MissingHeader`] when the answer carries no `ETag`, and
/// [`CdrError::MalformedHeader`] when it names no contribution.
pub fn contribution_uid_from_etag(
    operation: &'static str,
    status: StatusCode,
    etag: Option<&str>,
) -> Result<ContributionUid, CdrError> {
    let etag = etag.ok_or(CdrError::MissingHeader {
        operation,
        status,
        header: "ETag",
    })?;
    ContributionUid::new(ids::entity_tag(etag)).map_err(|source| CdrError::MalformedHeader {
        operation,
        header: "ETag",
        source: Box::new(source),
    })
}

/// Returns what a `2xx` body carries, per the `Prefer` it was asked with.
///
/// ITS-REST 1.1.0 pairs each preference with a body: no body for
/// `return=minimal`, the `Identifier` schema for `return=identifier`, and the
/// resource itself for `return=representation`. An absent body answers
/// [`Returned::Minimal`] whatever was asked, because the answer's status and
/// headers already name what the service stored.
///
/// # Errors
/// Returns [`CdrError::Body`] when a body is not the schema its preference
/// selects.
pub fn returned<T: DeserializeOwned>(
    operation: &'static str,
    body: Option<&serde_json::Value>,
    prefer: Prefer,
) -> Result<Returned<T>, CdrError> {
    // NOTE: RFC 7240 §2 lets a server ignore a preference it cannot honour, so
    // an absent body is the minimal answer and never a decoding failure.
    let Some(body) = body else {
        return Ok(Returned::Minimal);
    };
    match prefer {
        Prefer::Minimal => Ok(Returned::Minimal),
        Prefer::Identifier => serde_json::from_value::<Identifier>(body.clone())
            .map(Returned::Identifier)
            .map_err(|source| CdrError::Body {
                operation,
                source: Box::new(error::BodyError::Json(source)),
            }),
        Prefer::Representation => openehr_its::json::from_canonical_value::<T>(body)
            .map(|value| Returned::Representation(Box::new(value)))
            .map_err(|source| CdrError::Body {
                operation,
                source: Box::new(error::BodyError::CanonicalJson(source)),
            }),
    }
}

/// Returns what the `201` of a committed contribution carries.
///
/// The schema is `oneOf` CONTRIBUTION and `Identifier`, and the body is empty
/// for `return=minimal` (`ehr-codegen.openapi.yaml`, `201_CONTRIBUTION`), so
/// the body is read as whichever of the three it is.
///
/// # Errors
/// Returns [`CdrError::CommittedBody`], naming `contribution_uid`, when the
/// body is neither schema: the commit already happened, so the uid is what
/// the answer still carries.
pub fn committed(
    contribution_uid: &ContributionUid,
    body: Option<&serde_json::Value>,
    prefer: Prefer,
) -> Result<Returned<Contribution>, CdrError> {
    let Some(body) = body else {
        return Ok(Returned::Minimal);
    };
    if prefer == Prefer::Minimal {
        return Ok(Returned::Minimal);
    }
    match openehr_its::json::from_canonical_value::<Contribution>(body) {
        Ok(contribution) => Ok(Returned::Representation(Box::new(contribution))),
        // NOTE: `201_CONTRIBUTION` is `oneOf` CONTRIBUTION and `Identifier`, so
        // a body that is no CONTRIBUTION is legitimately read as the other.
        Err(first) => serde_json::from_value::<Identifier>(body.clone())
            .map(Returned::Identifier)
            .map_err(|_identifier| CdrError::CommittedBody {
                contribution_uid: contribution_uid.clone(),
                source: Box::new(first),
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::{Prefer, VersionAtTime, if_match_value};
    use crate::cdr::ids::version_from_etag;

    #[test]
    fn each_preference_renders_its_rfc_7240_value() {
        assert_eq!("return=minimal", Prefer::Minimal.as_str());
        assert_eq!("return=identifier", Prefer::Identifier.as_str());
        assert_eq!("return=representation", Prefer::Representation.as_str());
    }

    #[test]
    fn an_if_match_value_is_the_bare_quoted_version_id() {
        let version_id = version_from_etag("W/\"8849182c-82ad-4088-a07f-48ead4180515::system::1\"")
            .expect("a well-formed version id");
        assert_eq!(
            "\"8849182c-82ad-4088-a07f-48ead4180515::system::1\"",
            if_match_value(&version_id)
        );
    }

    #[test]
    fn a_version_at_time_refuses_a_control_character() {
        assert!(VersionAtTime::new("2015-01-20T19:30:22\u{7}").is_err());
        assert!(VersionAtTime::new("").is_err());
    }
}
