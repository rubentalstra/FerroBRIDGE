// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! The three CDM datatypes Rust has no type for: a bounded string, a date and
//! a date with a time.
//!
//! A CDM `varchar(n)` is a PostgreSQL `character varying(n)`, which refuses a
//! value longer than `n` characters
//! (<https://www.postgresql.org/docs/16/datatype-character.html>), so
//! [`Varchar`] refuses it at construction rather than at the `COPY`. A `date`
//! and a `datetime` are kept lexically: the bridge moves text between two
//! systems and never does calendar arithmetic on it, so converting to an
//! instant and back is a chance to change the value and no help at all.

use std::fmt;
use std::str::FromStr;

/// A value the CDM's column type refuses.
///
/// The variants carry the shape of the offending value, never the value
/// itself, because a column of a clinical row is patient data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    /// The string is longer than the column's `varchar(n)` bound.
    #[error("the value is {length} characters, more than the {limit} the column holds")]
    TooLong {
        /// The length of the offending value, in characters.
        length: usize,
        /// The column's bound.
        limit: usize,
    },
    /// The string is not an ISO 8601 calendar date.
    #[error("the value is not an ISO 8601 calendar date (YYYY-MM-DD)")]
    NotADate,
    /// The string is not an ISO 8601 date and time.
    #[error("the value is not an ISO 8601 date and time (YYYY-MM-DDThh:mm:ss)")]
    NotADatetime,
}

/// A string of at most `N` characters: the CDM's `varchar(n)`.
///
/// # Examples
///
/// ```
/// use omop_cdm::value::Varchar;
///
/// let source_value: Varchar<50> = "GP-0042".parse()?;
/// assert_eq!("GP-0042", source_value.as_str());
/// # Ok::<(), omop_cdm::value::ValueError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Varchar<const N: usize>(String);

impl<const N: usize> Varchar<N> {
    /// The number of characters the column holds.
    pub const LIMIT: usize = N;

    /// Creates a bounded string, refusing more than `N` characters.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::TooLong`] when the value is longer than `N`
    /// characters.
    pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
        let value = value.into();
        let length = value.chars().count();
        if length > N {
            return Err(ValueError::TooLong { length, limit: N });
        }
        Ok(Self(value))
    }

    /// Returns the value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the owned value.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl<const N: usize> fmt::Display for Varchar<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const N: usize> AsRef<str> for Varchar<N> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> FromStr for Varchar<N> {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<const N: usize> TryFrom<String> for Varchar<N> {
    type Error = ValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<const N: usize> TryFrom<&str> for Varchar<N> {
    type Error = ValueError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A calendar date in the ISO 8601 extended form `YYYY-MM-DD`: the CDM's
/// `date`.
///
/// The form is the one OHDSI's rendered PostgreSQL DDL declares as `date` and
/// the one PostgreSQL writes with the default `DateStyle`
/// (<https://www.postgresql.org/docs/16/datatype-datetime.html>). The value is
/// kept as text, exactly as it was given.
///
/// # Examples
///
/// ```
/// use omop_cdm::value::CdmDate;
///
/// let start = CdmDate::new("2026-09-12")?;
/// assert_eq!("2026-09-12", start.as_str());
/// assert!(CdmDate::new("2026-9-12").is_err());
/// # Ok::<(), omop_cdm::value::ValueError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CdmDate(String);

impl CdmDate {
    /// Creates a date, refusing anything that is not `YYYY-MM-DD`.
    ///
    /// The month is `01` to `12` and the day is one the month has, leap years
    /// included. Every component is zero-padded, so `2026-9-12` is refused.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::NotADate`] when the value is not an ISO 8601
    /// calendar date.
    pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
        let value = value.into();
        if value.len() == DATE_LENGTH && calendar_date(value.as_bytes()).is_some() {
            Ok(Self(value))
        } else {
            Err(ValueError::NotADate)
        }
    }

    /// Returns the value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the owned value.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for CdmDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for CdmDate {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for CdmDate {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for CdmDate {
    type Error = ValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for CdmDate {
    type Error = ValueError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A date and time in the ISO 8601 extended form `YYYY-MM-DDThh:mm:ss`: the
/// CDM's `datetime`.
///
/// The DDL declares the column `TIMESTAMP`, which is a timestamp without a
/// time zone
/// (<https://www.postgresql.org/docs/16/datatype-datetime.html>), so an offset
/// is refused rather than dropped. A fractional second of one to nine digits
/// is accepted. The value is kept as text, exactly as it was given.
///
/// # Examples
///
/// ```
/// use omop_cdm::value::CdmDatetime;
///
/// let start = CdmDatetime::new("2026-09-12T10:00:00")?;
/// assert_eq!("2026-09-12T10:00:00", start.as_str());
/// assert!(CdmDatetime::new("2026-09-12").is_err());
/// # Ok::<(), omop_cdm::value::ValueError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CdmDatetime(String);

impl CdmDatetime {
    /// Creates a date and time, refusing anything that is not
    /// `YYYY-MM-DDThh:mm:ss` with an optional fractional second.
    ///
    /// The hour is `00` to `23`, the minute and the second `00` to `59`; a
    /// leap second, a time zone offset and a bare date are all refused.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::NotADatetime`] when the value is not an ISO 8601
    /// date and time.
    pub fn new(value: impl Into<String>) -> Result<Self, ValueError> {
        let value = value.into();
        if datetime(value.as_bytes()) {
            Ok(Self(value))
        } else {
            Err(ValueError::NotADatetime)
        }
    }

    /// Returns the value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the owned value.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for CdmDatetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for CdmDatetime {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for CdmDatetime {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for CdmDatetime {
    type Error = ValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for CdmDatetime {
    type Error = ValueError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// The length of `YYYY-MM-DD`.
const DATE_LENGTH: usize = 10;

/// The length of `YYYY-MM-DDThh:mm:ss`.
const DATETIME_LENGTH: usize = 19;

/// The longest fractional second accepted, in digits.
const FRACTION_DIGITS: usize = 9;

/// Returns the year, month and day of `YYYY-MM-DD` at the head of `bytes`,
/// or `None` when they are not a calendar date.
fn calendar_date(bytes: &[u8]) -> Option<(u32, u32, u32)> {
    if separator(bytes, 4) != Some(b'-') || separator(bytes, 7) != Some(b'-') {
        return None;
    }
    let year = number(bytes, 0, 4)?;
    let month = number(bytes, 5, 7)?;
    let day = number(bytes, 8, DATE_LENGTH)?;
    if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some((year, month, day))
}

/// Returns whether `bytes` is `YYYY-MM-DDThh:mm:ss` with an optional
/// fractional second.
fn datetime(bytes: &[u8]) -> bool {
    if bytes.len() < DATETIME_LENGTH || calendar_date(bytes).is_none() {
        return false;
    }
    if separator(bytes, DATE_LENGTH) != Some(b'T')
        || separator(bytes, 13) != Some(b':')
        || separator(bytes, 16) != Some(b':')
    {
        return false;
    }
    let (Some(hour), Some(minute), Some(second)) = (
        number(bytes, 11, 13),
        number(bytes, 14, 16),
        number(bytes, 17, DATETIME_LENGTH),
    ) else {
        return false;
    };
    if hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    fraction(bytes)
}

/// Returns whether what follows the seconds is nothing, or a decimal point and
/// one to nine digits.
fn fraction(bytes: &[u8]) -> bool {
    let Some(rest) = bytes.get(DATETIME_LENGTH..) else {
        return false;
    };
    match rest.split_first() {
        None => true,
        Some((b'.', digits)) => {
            !digits.is_empty()
                && digits.len() <= FRACTION_DIGITS
                && digits.iter().all(u8::is_ascii_digit)
        }
        Some(_) => false,
    }
}

/// Returns the byte at `index`, when there is one.
fn separator(bytes: &[u8], index: usize) -> Option<u8> {
    bytes.get(index).copied()
}

/// Returns the decimal number the ASCII digits in `start..end` spell.
fn number(bytes: &[u8], start: usize, end: usize) -> Option<u32> {
    let digits = bytes.get(start..end)?;
    let mut value = 0_u32;
    for byte in digits.iter().copied() {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    Some(value)
}

/// Returns the number of days in `month` of `year`, per the proleptic
/// Gregorian calendar ISO 8601 uses.
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}
