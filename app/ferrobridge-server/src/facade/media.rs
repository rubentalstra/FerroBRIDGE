// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Media types, `Prefer`, and the query parameters the facade reads.
//!
//! "The MIME-type for JSON content is `application/fhir+json`" and a server
//! "SHALL accept … `application/json`" as an alias
//! (<https://hl7.org/fhir/R4/http.html#mime-type>). Every response this facade
//! writes carries `application/fhir+json`; a request body in anything else is
//! `415`, and an `Accept` header naming nothing the facade produces is `406`.

use http::HeaderMap;
use http::header;

/// The FHIR JSON media type.
pub const FHIR_JSON: &str = "application/fhir+json";

/// The plain JSON media type a FHIR server also accepts.
pub const JSON: &str = "application/json";

/// What a client asked the facade to return.
///
/// "The client can indicate whether the response should include the resource
/// or an `OperationOutcome`" through the `Prefer` header
/// (<https://hl7.org/fhir/R4/http.html#return>).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Prefer {
    /// `return=minimal`: no body.
    Minimal,
    /// `return=representation`: the resource itself, the facade's default.
    #[default]
    Representation,
    /// `return=OperationOutcome`: an `OperationOutcome` about the interaction.
    Outcome,
}

impl Prefer {
    /// Returns what `headers` asked for, the default when they ask for
    /// nothing this facade understands.
    ///
    /// R4 leaves an unrecognised `Prefer` to the server ("the server MAY
    /// ignore it"), so an unknown token falls back to the default rather than
    /// refusing the request (<https://hl7.org/fhir/R4/http.html#return>).
    #[must_use]
    pub fn of(headers: &HeaderMap) -> Self {
        for value in headers.get_all("prefer") {
            let Ok(text) = value.to_str() else {
                continue;
            };
            for part in text.split(',') {
                match part.trim().to_ascii_lowercase().as_str() {
                    "return=minimal" => return Self::Minimal,
                    "return=representation" => return Self::Representation,
                    "return=operationoutcome" => return Self::Outcome,
                    _ => {}
                }
            }
        }
        Self::Representation
    }
}

/// Why a request's media types are not ones the facade speaks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaError {
    /// The request body carries a media type the facade does not read.
    #[error("{found} is not a media type this server reads; send {FHIR_JSON} or {JSON}")]
    Unsupported {
        /// The `Content-Type` the client sent.
        found: String,
    },
    /// The `Accept` header names nothing the facade produces.
    #[error("this server produces {FHIR_JSON}, which {found} does not accept")]
    NotAcceptable {
        /// The `Accept` header the client sent.
        found: String,
    },
}

/// Returns the type and subtype of a media type, without its parameters.
fn essence(value: &str) -> &str {
    value.split(';').next().unwrap_or(value).trim()
}

/// Checks the `Content-Type` of a request that carries a body.
///
/// # Errors
///
/// Returns [`MediaError::Unsupported`] for any media type other than
/// `application/fhir+json` and `application/json`. An absent `Content-Type` is
/// read as `application/fhir+json`, the media type of the interaction.
pub fn check_content_type(headers: &HeaderMap) -> Result<(), MediaError> {
    let Some(value) = headers.get(header::CONTENT_TYPE) else {
        return Ok(());
    };
    // NOTE: a header value that is not visible ASCII cannot name a media type,
    // so the refusal reports it as sent rather than as text
    // (<https://hl7.org/fhir/R4/http.html#mime-type>).
    let Ok(text) = value.to_str() else {
        return Err(MediaError::Unsupported {
            found: String::from("a Content-Type that is not text"),
        });
    };
    let found = essence(text).to_ascii_lowercase();
    if found == FHIR_JSON || found == JSON {
        return Ok(());
    }
    Err(MediaError::Unsupported { found })
}

/// Checks that the client accepts what the facade produces.
///
/// # Errors
///
/// Returns [`MediaError::NotAcceptable`] when the `Accept` header names
/// neither `application/fhir+json`, `application/json`, `application/*` nor
/// `*/*`. An absent `Accept` accepts everything (RFC 9110 §12.5.1).
pub fn check_accept(headers: &HeaderMap) -> Result<(), MediaError> {
    let Some(value) = headers.get(header::ACCEPT) else {
        return Ok(());
    };
    let Ok(text) = value.to_str() else {
        return Err(MediaError::NotAcceptable {
            found: String::from("an Accept header that is not text"),
        });
    };
    let acceptable = text.split(',').any(|entry| {
        matches!(
            essence(entry).to_ascii_lowercase().as_str(),
            FHIR_JSON | JSON | "application/*" | "*/*"
        )
    });
    if acceptable {
        return Ok(());
    }
    Err(MediaError::NotAcceptable {
        found: text.to_owned(),
    })
}

/// Why a `_count` is not a page size.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("_count is `{found}`, which is no non-negative whole number")]
pub struct CountError {
    /// The value the client sent.
    found: String,
}

impl CountError {
    /// Returns the value the client sent.
    #[must_use]
    pub fn found(&self) -> &str {
        &self.found
    }
}

/// Returns the `_count` a query string names, when it names one.
///
/// "`_count` … is an instruction to the server regarding how many resources
/// should be returned in a single page"
/// (<https://hl7.org/fhir/R4/search.html#count>), so the value is a
/// non-negative whole number. A malformed one is refused rather than ignored.
///
/// # Errors
///
/// Returns [`CountError`] when `_count` is present and is not a non-negative
/// whole number.
pub fn count(query: Option<&str>) -> Result<Option<u32>, CountError> {
    let Some(query) = query else {
        return Ok(None);
    };
    for pair in query.split('&') {
        let mut halves = pair.splitn(2, '=');
        if halves.next() != Some("_count") {
            continue;
        }
        let raw = halves.next().unwrap_or_default();
        return raw.parse::<u32>().map(Some).map_err(|_parse| CountError {
            found: raw.to_owned(),
        });
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{CountError, MediaError, Prefer, check_accept, check_content_type, count};
    use http::{HeaderMap, HeaderValue, header};

    /// Returns headers carrying `name: value`.
    fn headers(name: header::HeaderName, value: &str) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(name, HeaderValue::from_str(value).expect("a legal header"));
        map
    }

    #[test]
    fn both_json_media_types_are_read_and_nothing_else_is() {
        for value in [
            "application/fhir+json",
            "application/json",
            "application/fhir+json; charset=utf-8",
            "APPLICATION/FHIR+JSON",
        ] {
            assert!(
                check_content_type(&headers(header::CONTENT_TYPE, value)).is_ok(),
                "{value} is a media type this server reads"
            );
        }
        let refused = check_content_type(&headers(header::CONTENT_TYPE, "application/fhir+xml"))
            .expect_err("XML is refused");
        assert!(matches!(refused, MediaError::Unsupported { .. }));
        assert!(
            check_content_type(&HeaderMap::new()).is_ok(),
            "an absent Content-Type reads as the interaction's media type"
        );
    }

    #[test]
    fn an_accept_header_naming_nothing_produced_is_refused() {
        for value in [
            "*/*",
            "application/*",
            "application/fhir+json",
            "text/html, application/json",
        ] {
            assert!(
                check_accept(&headers(header::ACCEPT, value)).is_ok(),
                "{value} accepts what this server produces"
            );
        }
        assert!(matches!(
            check_accept(&headers(header::ACCEPT, "application/fhir+xml")),
            Err(MediaError::NotAcceptable { .. })
        ));
    }

    #[test]
    fn prefer_reads_the_three_return_forms_and_defaults_to_representation() {
        assert_eq!(
            Prefer::Minimal,
            Prefer::of(&headers(
                header::HeaderName::from_static("prefer"),
                "return=minimal"
            ))
        );
        assert_eq!(
            Prefer::Outcome,
            Prefer::of(&headers(
                header::HeaderName::from_static("prefer"),
                "return=OperationOutcome"
            ))
        );
        assert_eq!(
            Prefer::Representation,
            Prefer::of(&headers(
                header::HeaderName::from_static("prefer"),
                "handling=strict"
            ))
        );
        assert_eq!(Prefer::Representation, Prefer::of(&HeaderMap::new()));
    }

    #[test]
    fn a_malformed_count_is_refused_and_a_well_formed_one_reads() {
        assert_eq!(Ok(None), count(None));
        assert_eq!(Ok(None), count(Some("_id=abc")));
        assert_eq!(Ok(Some(20)), count(Some("_count=20&_id=abc")));
        let refused: CountError = count(Some("_count=many")).expect_err("a word is no page size");
        assert_eq!("many", refused.found());
        assert!(
            count(Some("_count=-1")).is_err(),
            "a negative page size is refused"
        );
        assert!(
            count(Some("_count=")).is_err(),
            "an empty page size is refused"
        );
    }
}
