// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! `DV_INTERVAL<DV_DATE_TIME>` against FHIR `Period`.
//!
//! `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/data-mappings.adoc`
//! states the pairing: "if a period is provided FHIRconnect maps this period
//! to an interval in the corresponding path in openEHR". The bound rows come
//! from the `DV_INTERVAL` page, whose `Range` table pairs the two FHIR bounds
//! with `lower` and `upper` and fixes `lower_included` and `upper_included`
//! true and the two unbounded flags false.
//!
//! A `Period` may carry one bound only
//! (<https://hl7.org/fhir/R4/datatypes.html#Period>), and the reference
//! model's `Interval` states `lower_unbounded` as the absence of `lower`, so
//! an absent bound is the unbounded one and is not included. The `DV_QUANTITY`
//! interval against `Range` is not in this module.

use fhir_types::r4::period::Period;
use fhir_types::r4::primitives::DateTime;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;

use crate::engine::lens::Fallback;
use crate::engine::lens::Lens;
use crate::engine::lens::LensError;
use crate::engine::lens::code_phrase::text_of;
use crate::engine::lens::date_time::instant;

/// The name this cell carries in a diagnostic.
pub const CELL: &str = "DV_INTERVAL<DV_DATE_TIME> against Period";

/// `DV_INTERVAL<DV_DATE_TIME>` against `Period`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DateTimeIntervalLens;

impl Lens<DvInterval<DvDateTime>, Period> for DateTimeIntervalLens {
    fn cell(&self) -> &'static str {
        CELL
    }

    fn get(&self, source: &DvInterval<DvDateTime>) -> Result<Period, LensError> {
        Ok(Period {
            start: source
                .lower
                .as_ref()
                .map(|lower| DateTime::from(lower.value.clone())),
            end: source
                .upper
                .as_ref()
                .map(|upper| DateTime::from(upper.value.clone())),
            ..Period::default()
        })
    }

    fn put(
        &self,
        view: &Period,
        existing: Option<&DvInterval<DvDateTime>>,
    ) -> Result<DvInterval<DvDateTime>, LensError> {
        let lower = bound(
            view.start.as_ref(),
            existing.and_then(|held| held.lower.as_ref()),
        );
        let upper = bound(
            view.end.as_ref(),
            existing.and_then(|held| held.upper.as_ref()),
        );
        Ok(DvInterval {
            lower_unbounded: lower.is_none(),
            upper_unbounded: upper.is_none(),
            lower_included: lower.is_some(),
            upper_included: upper.is_some(),
            lower,
            upper,
        })
    }

    fn fallbacks(&self, _view: &Period) -> Vec<Fallback> {
        Vec::new()
    }
}

/// Returns one bound of the interval, `None` when the period leaves it open.
fn bound(value: Option<&DateTime>, existing: Option<&DvDateTime>) -> Option<DvDateTime> {
    let text = value.and_then(|value| text_of(Some(&value.value)))?;
    Some(instant(String::from(text), existing))
}

#[cfg(test)]
mod tests {
    use super::DateTimeIntervalLens;
    use crate::engine::lens::Lens;
    use crate::engine::lens::date_time::instant;
    use fhir_types::r4::period::Period;
    use fhir_types::r4::primitives::DateTime;
    use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;
    use proptest::prop_compose;
    use proptest::proptest;

    /// Returns the interval of `lower` and `upper`, with the flags the table
    /// fixes.
    fn interval(lower: Option<&str>, upper: Option<&str>) -> DvInterval<DvDateTime> {
        let lower = lower.map(|text| instant(String::from(text), None));
        let upper = upper.map(|text| instant(String::from(text), None));
        DvInterval {
            lower_unbounded: lower.is_none(),
            upper_unbounded: upper.is_none(),
            lower_included: lower.is_some(),
            upper_included: upper.is_some(),
            lower,
            upper,
        }
    }

    prop_compose! {
        /// An interval whose bounds are present or absent independently.
        fn bounded()(
            lower in proptest::option::of("20[0-9]{2}-[01][0-9]-[0-3][0-9]"),
            upper in proptest::option::of("20[0-9]{2}-[01][0-9]-[0-3][0-9]"),
        ) -> DvInterval<DvDateTime> {
            interval(lower.as_deref(), upper.as_deref())
        }
    }

    prop_compose! {
        /// A `Period` whose bounds are present or absent independently.
        fn period()(
            start in proptest::option::of("20[0-9]{2}-[01][0-9]-[0-3][0-9]"),
            end in proptest::option::of("20[0-9]{2}-[01][0-9]-[0-3][0-9]"),
        ) -> Period {
            Period {
                start: start.map(DateTime::from),
                end: end.map(DateTime::from),
                ..Period::default()
            }
        }
    }

    proptest! {
        #[test]
        fn get_put(source in bounded()) {
            let view: Period = DateTimeIntervalLens.get(&source).expect("the bounds travel");
            let back = DateTimeIntervalLens.put(&view, Some(&source)).expect("the view is complete");
            assert_eq!(back, source, "GetPut failed for {source:?}");
        }

        #[test]
        fn put_get(view in period()) {
            let source: DvInterval<DvDateTime> = DateTimeIntervalLens
                .put(&view, None)
                .expect("the view is complete");
            let back: Period = DateTimeIntervalLens.get(&source).expect("the bounds travel");
            assert_eq!(back, view, "PutGet failed for {view:?}");
        }
    }

    #[test]
    fn a_bounded_interval_includes_both_ends() {
        let view = Period {
            start: Some(DateTime::from("2026-09-13T10:00:00+01:00")),
            end: Some(DateTime::from("2026-09-14T10:00:00+01:00")),
            ..Period::default()
        };
        let source = DateTimeIntervalLens
            .put(&view, None)
            .expect("both bounds are present");
        assert!(source.lower_included, "a present lower bound is included");
        assert!(source.upper_included, "a present upper bound is included");
        assert!(!source.lower_unbounded, "a present lower bound is bounded");
        assert!(!source.upper_unbounded, "a present upper bound is bounded");
        assert_eq!(
            source.lower.map(|lower| lower.value),
            Some(String::from("2026-09-13T10:00:00+01:00")),
            "the offset travels byte for byte"
        );
    }

    #[test]
    fn an_open_period_leaves_the_upper_bound_unbounded() {
        let view = Period {
            start: Some(DateTime::from("2026-09-13")),
            ..Period::default()
        };
        let source = DateTimeIntervalLens
            .put(&view, None)
            .expect("one bound is present");
        assert!(source.upper_unbounded, "an absent end is unbounded");
        assert!(
            !source.upper_included,
            "an unbounded limit is not an included one"
        );
    }
}
