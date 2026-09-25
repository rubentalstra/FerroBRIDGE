// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Converting a v2 value to the FHIR primitive its target element holds.
//!
//! The date and time forms follow the mapping guidelines, which leave the v2
//! `DTM`, `DT`, `TS` and `TM` to FHIR `dateTime`, `date`, `instant` and `time`
//! conversions to ISO 8601 (`mapping_guidelines.md` §Data Type Spreadsheet).
//! A v2 `DTM` is `YYYY[MM[DD[HH[MM[SS[.S[S[S[S]]]]]]]]][+/-ZZZZ]` (HL7 v2.5.1
//! chapter 2A §2.A.22); the FHIR forms are those of
//! <https://hl7.org/fhir/R4/datatypes.html#dateTime>, where a value with a time
//! carries seconds and a time-zone offset. A number is a v2 `NM` (§2.A.47): an
//! optional sign, digits and an optional decimal point, leading zeros not
//! significant.
//!
//! A value the target cannot hold is refused, never approximated: a time with
//! no offset is no FHIR `dateTime`.
//!
//! A v2 `HD` written into a FHIR `url` is converted by [`endpoint`]: the
//! guide's url form of a typed universal ID, else a derived `urn:` form.

use fhir_types::codec::{Number, Value};

/// Why a value could not be converted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    /// The value is not of the v2 form the target type is converted from.
    #[error("the value is no v2 {expected}")]
    Form {
        /// The v2 form expected.
        expected: &'static str,
    },
    /// The value carries a time without a time-zone offset, which FHIR
    /// `dateTime` and `instant` cannot hold.
    #[error("the value has a time without a time-zone offset")]
    NoOffset,
    /// The value lacks the precision the target requires.
    #[error("the value lacks the precision a FHIR {target} requires")]
    Precision {
        /// The FHIR type.
        target: &'static str,
    },
    /// The target is a boolean, which a v2 value reaches only through a table
    /// map.
    #[error("a FHIR boolean is reached only through a table map")]
    Boolean,
}

/// Converts `text` for an element of the FHIR type `fhir_type`.
///
/// # Errors
///
/// Returns [`ConvertError`] when the value is not of the form the type is
/// converted from.
pub fn primitive(fhir_type: &str, text: &str) -> Result<Value, ConvertError> {
    match fhir_type {
        "date" => date(text).map(Value::String),
        "dateTime" => date_time(text, false).map(Value::String),
        "instant" => date_time(text, true).map(Value::String),
        "time" => time(text).map(Value::String),
        "decimal" => decimal(text).map(Value::Number),
        "integer" | "positiveInt" | "unsignedInt" => integer(fhir_type, text).map(Value::Number),
        "boolean" => Err(ConvertError::Boolean),
        _ => Ok(Value::String(String::from(text))),
    }
}

/// The prefix of an endpoint derived from an `HD` that names no url of its
/// own.
///
/// No specification governs this: our own design. The guide leaves an `HD`
/// without a universal ID of a url-yielding type to the implementer, and the
/// derived endpoint names the product so it is never read as an identifier
/// the sender assigned.
pub const DERIVED_ENDPOINT: &str = "urn:ferrobridge:hl7v2-hd:";

/// The endpoints a v2 `HD` yields for a FHIR `url` element, in the order they
/// are tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The url the guide's `HD` map builds from a universal ID of type `ISO`,
    /// `UUID`, `DNS` or `URI`, when the `HD` carries one.
    pub universal: Option<String>,
    /// The url derived from every component: [`DERIVED_ENDPOINT`], the
    /// namespace ID and, when either is valued, the universal ID and its
    /// type, each percent-encoded and joined by `:`.
    pub derived: String,
}

/// Builds the endpoints of a v2 `HD` from its namespace ID (`HD.1`),
/// universal ID (`HD.2`) and universal ID type (`HD.3`).
///
/// The universal forms are the assignments of the guide's
/// `datatype-hd-endpoint-to-messageheader-source` map: `urn:oid:`,
/// `urn:uuid:`, `urn:dns:` or `urn:uri:` before the universal ID.
///
/// # Errors
///
/// Returns [`ConvertError::Form`] when no component is valued.
pub fn endpoint(
    namespace: Option<&str>,
    universal: Option<&str>,
    universal_type: Option<&str>,
) -> Result<Endpoint, ConvertError> {
    if namespace.is_none() && universal.is_none() && universal_type.is_none() {
        return Err(ConvertError::Form { expected: "HD" });
    }
    let scheme = match universal_type {
        Some("ISO") => Some("urn:oid:"),
        Some("UUID") => Some("urn:uuid:"),
        Some("DNS") => Some("urn:dns:"),
        Some("URI") => Some("urn:uri:"),
        _ => None,
    };
    let universal_url = scheme
        .zip(universal)
        .map(|(scheme, universal)| format!("{scheme}{universal}"));
    let mut derived = String::from(DERIVED_ENDPOINT);
    percent_encode(namespace.unwrap_or_default(), &mut derived);
    if universal.is_some() || universal_type.is_some() {
        derived.push(':');
        percent_encode(universal.unwrap_or_default(), &mut derived);
        derived.push(':');
        percent_encode(universal_type.unwrap_or_default(), &mut derived);
    }
    Ok(Endpoint {
        universal: universal_url,
        derived,
    })
}

/// Appends `text` to `out` with every byte outside the RFC 3986 §3.3 `pchar`
/// set percent-encoded, and `:` too, which the derived endpoint uses as its
/// separator.
fn percent_encode(text: &str, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in text.bytes() {
        let kept = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b'@'
            );
        if kept {
            out.push(char::from(byte));
        } else {
            out.push('%');
            for nibble in [byte >> 4, byte & 0x0F] {
                if let Some(digit) = HEX.get(usize::from(nibble)) {
                    out.push(char::from(*digit));
                }
            }
        }
    }
}

/// The parts of a v2 `DTM`.
struct Parts<'a> {
    date: &'a str,
    time: &'a str,
    fraction: Option<&'a str>,
    offset: Option<&'a str>,
}

/// Splits a v2 `DTM` into its date, time, fraction and offset.
fn parts(text: &str) -> Result<Parts<'_>, ConvertError> {
    let form = ConvertError::Form { expected: "DTM" };
    let (body, offset) = match text.find(['+', '-']) {
        Some(at) => (text.get(..at).ok_or(form.clone())?, text.get(at..)),
        None => (text, None),
    };
    if let Some(offset) = offset
        && (offset.len() != 5
            || !offset
                .get(1..)
                .is_some_and(|digits| digits.bytes().all(|byte| byte.is_ascii_digit())))
    {
        return Err(form);
    }
    let (digits, fraction) = match body.split_once('.') {
        Some((digits, fraction)) => (digits, Some(fraction)),
        None => (body, None),
    };
    let valid_length = matches!(digits.len(), 4 | 6 | 8 | 10 | 12 | 14);
    if !valid_length || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(form);
    }
    if let Some(fraction) = fraction
        && (digits.len() != 14
            || fraction.is_empty()
            || fraction.len() > 4
            || !fraction.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(form);
    }
    Ok(Parts {
        date: digits.get(..digits.len().min(8)).unwrap_or_default(),
        time: digits.get(8..).unwrap_or_default(),
        fraction,
        offset,
    })
}

/// Renders the date part as `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
fn render_date(date: &str) -> String {
    let mut out = String::from(date.get(..4).unwrap_or_default());
    for range in [4..6, 6..8] {
        if let Some(part) = date.get(range) {
            out.push('-');
            out.push_str(part);
        }
    }
    out
}

/// Renders `+hhmm` as `+hh:mm`.
fn render_offset(offset: &str) -> String {
    let sign = offset.get(..1).unwrap_or_default();
    let hours = offset.get(1..3).unwrap_or_default();
    let minutes = offset.get(3..5).unwrap_or_default();
    format!("{sign}{hours}:{minutes}")
}

/// Renders `hh[mm[ss]]` as `hh:mm:ss`, the seconds and minutes zero when
/// absent.
fn render_time(time: &str, fraction: Option<&str>) -> String {
    let hours = time.get(..2).unwrap_or("00");
    let minutes = time.get(2..4).unwrap_or("00");
    let seconds = time.get(4..6).unwrap_or("00");
    let mut out = format!("{hours}:{minutes}:{seconds}");
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

/// A FHIR `date` from a v2 `DT` or `DTM`; the time of a `DTM` is not part of
/// a date and is left out.
fn date(text: &str) -> Result<String, ConvertError> {
    Ok(render_date(parts(text)?.date))
}

/// A FHIR `dateTime`, or with `instant` a FHIR `instant`, from a v2 `DTM`.
fn date_time(text: &str, instant: bool) -> Result<String, ConvertError> {
    let parts = parts(text)?;
    if parts.time.is_empty() {
        if instant {
            return Err(ConvertError::Precision { target: "instant" });
        }
        return Ok(render_date(parts.date));
    }
    if instant && parts.time.len() < 6 {
        return Err(ConvertError::Precision { target: "instant" });
    }
    if parts.date.len() < 8 {
        return Err(ConvertError::Form { expected: "DTM" });
    }
    let offset = parts.offset.ok_or(ConvertError::NoOffset)?;
    // NOTE: HL7 R4 datatypes §dateTime requires seconds once a time is given,
    // so a v2 time to the hour or minute is written with zero seconds.
    Ok(format!(
        "{}T{}{}",
        render_date(parts.date),
        render_time(parts.time, parts.fraction),
        render_offset(offset)
    ))
}

/// A FHIR `time` from a v2 `TM`, `HH[MM[SS[.S]]]` with no offset.
fn time(text: &str) -> Result<String, ConvertError> {
    let form = ConvertError::Form { expected: "TM" };
    if text.contains(['+', '-']) {
        return Err(form);
    }
    let (digits, fraction) = match text.split_once('.') {
        Some((digits, fraction)) => (digits, Some(fraction)),
        None => (text, None),
    };
    if !matches!(digits.len(), 2 | 4 | 6) || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(form);
    }
    if fraction.is_some_and(|fraction| {
        digits.len() != 6
            || fraction.is_empty()
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return Err(form);
    }
    Ok(render_time(digits, fraction))
}

/// A FHIR `decimal` from a v2 `NM`, the leading sign `+` and the leading
/// zeros dropped.
fn decimal(text: &str) -> Result<Number, ConvertError> {
    let form = ConvertError::Form { expected: "NM" };
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|fraction| fraction.is_empty() || !digits(fraction)) {
        return Err(form);
    }
    if whole.is_empty() && fraction.is_none() {
        return Err(form);
    }
    let trimmed = whole.trim_start_matches('0');
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(if trimmed.is_empty() { "0" } else { trimmed });
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    match out.parse() {
        Ok(number) => Ok(number),
        Err(fhir_types::codec::NotANumber) => Err(form),
    }
}

/// A FHIR `integer`, `positiveInt` or `unsignedInt` from a v2 `NM` with no
/// fraction.
fn integer(fhir_type: &str, text: &str) -> Result<Number, ConvertError> {
    let form = ConvertError::Form { expected: "NM" };
    let Ok(value) = text.strip_prefix('+').unwrap_or(text).parse::<i64>() else {
        return Err(form);
    };
    let admitted = match fhir_type {
        "positiveInt" => value >= 1,
        "unsignedInt" => value >= 0,
        _ => true,
    } && i32::try_from(value).is_ok();
    if !admitted {
        return Err(form);
    }
    Ok(Number::from(value))
}

#[cfg(test)]
mod tests {
    use super::{ConvertError, DERIVED_ENDPOINT, Endpoint, endpoint, primitive};
    use fhir_types::codec::Value;

    fn text(value: &str) -> Value {
        Value::String(String::from(value))
    }

    #[test]
    fn a_dtm_becomes_a_date_time_with_its_offset() {
        assert_eq!(
            primitive("dateTime", "20260925143000+0200"),
            Ok(text("2026-09-25T14:30:00+02:00"))
        );
        assert_eq!(primitive("dateTime", "202609"), Ok(text("2026-09")));
        assert_eq!(
            primitive("instant", "20260925143015.25-0500"),
            Ok(text("2026-09-25T14:30:15.25-05:00"))
        );
    }

    #[test]
    fn a_time_without_an_offset_is_no_date_time() {
        assert_eq!(
            primitive("dateTime", "202609251430"),
            Err(ConvertError::NoOffset)
        );
        assert_eq!(
            primitive("instant", "20260925"),
            Err(ConvertError::Precision { target: "instant" })
        );
    }

    #[test]
    fn a_date_keeps_only_the_date_of_a_dtm() {
        assert_eq!(primitive("date", "19800101"), Ok(text("1980-01-01")));
        assert_eq!(
            primitive("date", "198001011230+0100"),
            Ok(text("1980-01-01"))
        );
        assert!(primitive("date", "1980-01-01").is_err());
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn a_number_drops_its_plus_and_leading_zeros() -> Result<(), ConvertError> {
        assert_eq!(
            primitive("decimal", "+007.50")?,
            Value::Number("7.50".parse().expect("a number"))
        );
        assert_eq!(
            primitive("decimal", ".5")?,
            Value::Number("0.5".parse().expect("a number"))
        );
        assert!(primitive("decimal", "5,4").is_err());
        assert!(primitive("positiveInt", "0").is_err());
        Ok(())
    }

    #[test]
    fn a_time_is_written_with_seconds() {
        assert_eq!(primitive("time", "0930"), Ok(text("09:30:00")));
        assert!(primitive("time", "0930+0100").is_err());
    }

    #[test]
    fn a_universal_id_of_a_url_yielding_type_is_the_endpoint() {
        assert_eq!(
            endpoint(Some("LAB"), Some("1.2.3.4"), Some("ISO")),
            Ok(Endpoint {
                universal: Some(String::from("urn:oid:1.2.3.4")),
                derived: format!("{DERIVED_ENDPOINT}LAB:1.2.3.4:ISO"),
            })
        );
        let uuid = "2b3c4d5e-0000-4000-8000-000000000001";
        assert_eq!(
            endpoint(None, Some(uuid), Some("UUID")).map(|found| found.universal),
            Ok(Some(format!("urn:uuid:{uuid}")))
        );
        assert_eq!(
            endpoint(None, Some("lab.example.org"), Some("DNS")).map(|found| found.universal),
            Ok(Some(String::from("urn:dns:lab.example.org")))
        );
        assert_eq!(
            endpoint(None, Some("http://lab.example.org/v2"), Some("URI"))
                .map(|found| found.universal),
            Ok(Some(String::from("urn:uri:http://lab.example.org/v2")))
        );
    }

    #[test]
    fn a_namespace_id_alone_is_derived_with_every_non_pchar_byte_encoded() {
        assert_eq!(
            endpoint(Some("North Lab App"), None, None),
            Ok(Endpoint {
                universal: None,
                derived: format!("{DERIVED_ENDPOINT}North%20Lab%20App"),
            })
        );
        assert_eq!(
            endpoint(Some("Lab:M\u{FC}ller/1%"), None, None).map(|found| found.derived),
            Ok(format!("{DERIVED_ENDPOINT}Lab%3AM%C3%BCller%2F1%25"))
        );
    }

    #[test]
    fn a_universal_id_of_another_type_is_kept_in_the_derived_endpoint() {
        assert_eq!(
            endpoint(None, Some("LAB-7"), Some("L")),
            Ok(Endpoint {
                universal: None,
                derived: format!("{DERIVED_ENDPOINT}:LAB-7:L"),
            })
        );
        assert_eq!(
            endpoint(Some("LAB"), Some("1.2.3"), None).map(|found| found.derived),
            Ok(format!("{DERIVED_ENDPOINT}LAB:1.2.3:"))
        );
    }

    #[test]
    fn an_empty_hd_yields_no_endpoint() {
        assert_eq!(
            endpoint(None, None, None),
            Err(ConvertError::Form { expected: "HD" })
        );
    }
}
