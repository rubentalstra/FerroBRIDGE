// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The template fetch: one call, two routes behind it.
//!
//! `docs/specs/its-rest/computable/OAS/definition-codegen.openapi.yaml` serves
//! both template generations, and a composition names its template by a string
//! with no generation marker, so the client tries `adl1.4` first and falls
//! back to `adl2` on a `404` or a `406`.

use crate::client::{Call, Client, Idempotency};
use crate::error::{BodyError, Error, UpstreamError};
use crate::ids::entity_tag;
use crate::prefer::Prefer;
use http::{Method, StatusCode};
use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_base::v1_3::base_types::identification::template_id::TemplateId;
use serde::Deserialize;

/// The canonical XML media type the ADL 1.4 route answers with.
const CANONICAL_XML: &str = "application/xml";
/// The `_type` an AOM2 operational template declares.
const OPERATIONAL_TEMPLATE: &str = "OPERATIONAL_TEMPLATE";

/// A template, in the generation the service served it in.
///
/// Both generations resolve to the same Web Template downstream, so the
/// generation is decided by which route answered, never by a field of the
/// body.
#[derive(Debug)]
#[non_exhaustive]
pub enum TemplateSource {
    /// The canonical OPT 1.4 XML of the `adl1.4` route.
    Opt14(Box<openehr_its::opt14::types::OperationalTemplate>),
    /// The AOM2 canonical JSON of the `adl2` route.
    Opt2 {
        /// The operational template itself.
        template: Box<OperationalTemplate>,
        /// The full HRID the partial request resolved to; its
        /// `physical_id` is the identifier to cache under.
        resolved_id: ArchetypeHrid,
    },
}

/// What the two-route template fetch answered.
#[derive(Debug)]
#[non_exhaustive]
pub enum TemplateOutcome {
    /// `200` from one of the two routes.
    Found(TemplateSource),
    /// `404` from both routes: no template with this identifier exists.
    UnknownTemplate,
    /// `400` from the route that was tried.
    BadRequest(UpstreamError),
}

/// The `_type` discriminator of a canonical JSON body.
#[derive(Debug, Deserialize)]
struct TypeProbe {
    /// The `_type` the body declares, when it declares one.
    #[serde(rename = "_type")]
    type_name: Option<String>,
}

impl Client {
    /// Retrieves the operational template `template_id` names.
    ///
    /// The `adl1.4` route is asked first, with `Accept: application/xml` for
    /// the canonical OPT. A `404` or a `406` from it means the template is not
    /// an ADL 1.4 one, so the `adl2` route is asked with
    /// `Accept: application/json` for the AOM2 canonical JSON. The ADL 2
    /// request never offers `text/plain`, whose body is ADL 2 source, and
    /// never offers `application/xml` alone.
    ///
    /// # Errors
    /// Returns [`Error::NotOperationalTemplate`] when the `adl2` body is not
    /// an AOM2 operational template, and [`Error`] otherwise when a route
    /// answered a status the `OpenAPI` does not document or a body that cannot
    /// be decoded.
    pub async fn template(&self, template_id: &TemplateId) -> Result<TemplateOutcome, Error> {
        let url = self.url(&["definition", "template", "adl1.4", &template_id.value])?;
        let call = Call::new(Method::GET, url, Idempotency::Idempotent)
            .accepting(CANONICAL_XML)
            .preferring(Prefer::Representation);
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::OK => {
                let template =
                    openehr_its::opt14::from_xml(&answer.body).map_err(|source| Error::Body {
                        url: answer.url.clone(),
                        media: "canonical OPT 1.4 XML",
                        source: Box::new(BodyError::Xml(source)),
                    })?;
                Ok(TemplateOutcome::Found(TemplateSource::Opt14(Box::new(
                    template,
                ))))
            }
            StatusCode::BAD_REQUEST => Ok(TemplateOutcome::BadRequest(answer.upstream())),
            StatusCode::NOT_FOUND | StatusCode::NOT_ACCEPTABLE => {
                self.template_adl2(template_id).await
            }
            _ => Err(answer.undocumented()),
        }
    }

    /// Retrieves the ADL 2 operational template `template_id` names.
    async fn template_adl2(&self, template_id: &TemplateId) -> Result<TemplateOutcome, Error> {
        let url = self.url(&["definition", "template", "adl2", &template_id.value])?;
        let call = Call::new(Method::GET, url, Idempotency::Idempotent)
            .accepting(crate::client::CANONICAL_JSON)
            .preferring(Prefer::Representation);
        let answer = self.execute(call).await?;
        match answer.status {
            StatusCode::OK => {
                let probe = serde_json::from_str::<TypeProbe>(&answer.body).map_err(|source| {
                    Error::Body {
                        url: answer.url.clone(),
                        media: "AOM2 canonical JSON",
                        source: Box::new(BodyError::Json(source)),
                    }
                })?;
                if probe.type_name.as_deref() != Some(OPERATIONAL_TEMPLATE) {
                    return Err(Error::NotOperationalTemplate {
                        found: probe.type_name,
                    });
                }
                let template =
                    openehr_its::json::from_canonical_json::<OperationalTemplate>(&answer.body)
                        .map_err(|source| Error::Body {
                            url: answer.url.clone(),
                            media: "AOM2 canonical JSON",
                            source: Box::new(BodyError::CanonicalJson(source)),
                        })?;
                let resolved_id = resolved_id(&answer, &template);
                Ok(TemplateOutcome::Found(TemplateSource::Opt2 {
                    template: Box::new(template),
                    resolved_id,
                }))
            }
            StatusCode::BAD_REQUEST => Ok(TemplateOutcome::BadRequest(answer.upstream())),
            StatusCode::NOT_FOUND => Ok(TemplateOutcome::UnknownTemplate),
            _ => Err(answer.undocumented()),
        }
    }
}

/// Returns the full HRID the `adl2` answer resolved a partial request to.
///
/// The `ETag` is "an identifier of Template"
/// (`definition-codegen.openapi.yaml`,
/// `components.headers.ETag_Template_adl2`), so it is read as the resolved
/// HRID when it is one; otherwise the template's own `archetype_id`, which
/// AM 2.4 §Identification defines as the artefact's physical identifier,
/// answers instead.
fn resolved_id(answer: &crate::client::Answer, template: &OperationalTemplate) -> ArchetypeHrid {
    answer
        .header(http::header::ETAG.as_str())
        .and_then(hrid_from_etag)
        .unwrap_or_else(|| template.archetype_id.clone())
}

// TODO(#241): parse through `ArchetypeHrid` itself once openehr-am offers
// `FromStr` for it (sibling request S2), and drop the openehr-adl dependency.
/// Returns the archetype HRID an `ETag` value carries, when it carries one.
fn hrid_from_etag(etag: &str) -> Option<ArchetypeHrid> {
    // NOTE: the OAS example for ETag_Template_adl2 is a UUID, so an entity
    // tag that is not an HRID is legitimately not of this form.
    openehr_adl::hrid::parse_hrid(entity_tag(etag)).ok()
}

#[cfg(test)]
mod tests {
    use super::hrid_from_etag;

    #[test]
    fn an_hrid_accepts_the_namespaced_three_part_version() {
        let hrid = "org.highmed::openEHR-EHR-COMPOSITION.t_vital_signs.v1.0.0";
        assert_eq!(
            Some(hrid.to_owned()),
            hrid_from_etag(&format!("W/\"{hrid}\"")).map(|parsed| parsed.physical_id())
        );
    }

    #[test]
    fn an_hrid_accepts_a_partial_version_and_a_release_candidate() {
        assert!(hrid_from_etag("openEHR-EHR-COMPOSITION.t_vital_signs.v1").is_some());
        assert!(hrid_from_etag("openEHR-EHR-COMPOSITION.t_vital_signs.v1.8.2-rc.4").is_some());
    }

    #[test]
    fn an_hrid_refuses_a_missing_version_marker() {
        assert!(hrid_from_etag("openEHR-EHR-COMPOSITION.t_vital_signs.1.0.0").is_none());
    }

    #[test]
    fn an_hrid_refuses_a_two_part_root() {
        assert!(hrid_from_etag("openEHR-COMPOSITION.t_vital_signs.v1.0.0").is_none());
    }

    #[test]
    fn a_uuid_entity_tag_is_no_hrid() {
        assert!(hrid_from_etag("W/\"8849182c-82ad-4088-a07f-48ead4180515\"").is_none());
    }
}
