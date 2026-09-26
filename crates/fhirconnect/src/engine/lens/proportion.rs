// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `DV_PROPORTION` against FHIR `Quantity`, for a percentage only.
//!
//! `docs/specs/fhirconnect/modules/ROOT/pages/types-of-mappings/data-type/DV_PROPORTION.adoc`
//! maps the type onto `Ratio`, which carries a denominator. FHIR `Quantity`
//! carries none (<https://hl7.org/fhir/R4/datatypes.html#Quantity>), so the
//! only kind with a faithful `Quantity` is `pk_percent`, whose denominator the
//! reference model fixes at 100
//! (<https://specifications.openehr.org/releases/RM/Release-1.1.0/data_types.html#_dv_proportion_class>).
//! Any other kind is a typed refusal naming the element, and the engine writes
//! no extension URL of its own for it.

use fhir_types::r4::primitives::Decimal;
use fhir_types::r4::primitives::String as FhirString;
use fhir_types::r4::quantity::Quantity;
use openehr_rm::v1_2::data_types::quantity::dv_proportion::DvProportion;
use openehr_rm::v1_2::data_types::quantity::proportion_kind::ProportionKind;

use crate::engine::lens::Fallback;
use crate::engine::lens::Lens;
use crate::engine::lens::LensError;
use crate::engine::lens::code_phrase::text_of;
use crate::model::ast::keyword::Direction;

/// The name this cell carries in a diagnostic.
pub const CELL: &str = "DV_PROPORTION against Quantity";

/// The unit a percentage is written with.
///
/// UCUM spells a percentage `%` (<https://hl7.org/fhir/R4/datatypes.html#Quantity>).
pub const PERCENT: &str = "%";

/// The denominator the reference model fixes for `pk_percent`.
const PERCENT_DENOMINATOR: f64 = 100.0;

/// `DV_PROPORTION` against `Quantity`, for `pk_percent`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PercentLens;

impl Lens<DvProportion, Quantity> for PercentLens {
    fn cell(&self) -> &'static str {
        CELL
    }

    fn get(&self, source: &DvProportion) -> Result<Quantity, LensError> {
        if ProportionKind::from_value(source.r#type) != ProportionKind::PkPercent {
            return Err(LensError::UnrepresentableProportion {
                kind: source.r#type,
            });
        }
        if source.denominator.to_bits() != PERCENT_DENOMINATOR.to_bits() {
            return Err(LensError::Shape {
                cell: CELL,
                attribute: "DV_PROPORTION.denominator",
                found: render(source.denominator, None),
                reason: "the reference model fixes a percentage's denominator at 100",
                direction: Direction::OpenehrToFhir,
            });
        }
        Ok(Quantity {
            value: Some(Decimal {
                value: Some(render(source.numerator, source.precision)),
                ..Decimal::default()
            }),
            unit: Some(FhirString::from(PERCENT)),
            ..Quantity::default()
        })
    }

    fn put(
        &self,
        view: &Quantity,
        existing: Option<&DvProportion>,
    ) -> Result<DvProportion, LensError> {
        let unit = text_of(view.unit.as_ref().map(|unit| &unit.value))
            .or_else(|| text_of(view.code.as_ref().map(|code| &code.value)));
        if unit != Some(PERCENT) {
            return Err(LensError::Shape {
                cell: CELL,
                attribute: "Quantity.unit",
                found: String::from(unit.unwrap_or("")),
                reason: "a Quantity carries no denominator, so only a percentage is a proportion",
                direction: Direction::FhirToOpenehr,
            });
        }
        let text = view
            .value
            .as_ref()
            .and_then(|value| text_of(Some(&value.value)))
            .ok_or(LensError::Missing {
                cell: CELL,
                attribute: "DV_PROPORTION.numerator",
                direction: Direction::FhirToOpenehr,
            })?;
        // NOTE: the reference model declares `DV_PROPORTION.numerator` a
        // `Real`, so the openEHR side of this cell is binary floating point by
        // definition and the lexical decimal cannot be kept.
        let numerator = text.parse::<f64>().map_err(|_refused| LensError::Shape {
            cell: CELL,
            attribute: "Quantity.value",
            found: String::from(text),
            reason: "a proportion's numerator is a real number",
            direction: Direction::FhirToOpenehr,
        })?;
        Ok(DvProportion {
            normal_status: existing.and_then(|held| held.normal_status.clone()),
            normal_range: existing.and_then(|held| held.normal_range.clone()),
            other_reference_ranges: existing.and_then(|held| held.other_reference_ranges.clone()),
            magnitude_status: existing.and_then(|held| held.magnitude_status.clone()),
            accuracy: existing.and_then(|held| held.accuracy),
            accuracy_is_percent: existing.and_then(|held| held.accuracy_is_percent),
            numerator,
            denominator: PERCENT_DENOMINATOR,
            r#type: ProportionKind::PkPercent.value(),
            precision: places(text),
        })
    }

    fn fallbacks(&self, _view: &Quantity) -> Vec<Fallback> {
        Vec::new()
    }
}

/// Renders a real to the number of decimal places the proportion declares.
///
/// `precision` is "the number of decimal places", and -1 means no limit, so a
/// negative or absent precision renders the shortest form that reads back.
fn render(value: f64, precision: Option<i32>) -> String {
    match precision.and_then(|places| usize::try_from(places).ok()) {
        Some(places) => format!("{value:.places$}"),
        None => format!("{value}"),
    }
}

/// Returns the decimal places a lexical decimal was written with.
fn places(text: &str) -> Option<i32> {
    let (_whole, fraction) = text.split_once('.')?;
    i32::try_from(fraction.chars().count()).ok()
}

#[cfg(test)]
mod tests {
    use super::PercentLens;
    use crate::engine::lens::Lens;
    use crate::engine::lens::LensError;
    use fhir_types::r4::primitives::Decimal;
    use fhir_types::r4::primitives::String as FhirString;
    use fhir_types::r4::quantity::Quantity;
    use openehr_rm::v1_2::data_types::quantity::dv_proportion::DvProportion;
    use openehr_rm::v1_2::data_types::quantity::proportion_kind::ProportionKind;
    use proptest::prop_compose;
    use proptest::proptest;

    /// Returns a proportion of `kind` over `numerator` and `denominator`.
    fn proportion(kind: ProportionKind, numerator: f64, denominator: f64) -> DvProportion {
        DvProportion {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            numerator,
            denominator,
            r#type: kind.value(),
            precision: None,
        }
    }

    prop_compose! {
        /// A percentage written with one decimal place.
        fn percent()(tenths in 0i32..1000) -> DvProportion {
            let value = f64::from(tenths) / 10.0;
            DvProportion { precision: Some(1), ..proportion(ProportionKind::PkPercent, value, 100.0) }
        }
    }

    proptest! {
        #[test]
        fn get_put(source in percent()) {
            let view: Quantity = PercentLens.get(&source).expect("a percentage travels");
            let back = PercentLens.put(&view, Some(&source)).expect("the view is complete");
            assert_eq!(back, source, "GetPut failed for {source:?}");
        }

        #[test]
        fn put_get(source in percent()) {
            let view: Quantity = PercentLens.get(&source).expect("a percentage travels");
            let back: DvProportion = PercentLens.put(&view, None).expect("the view is complete");
            let again: Quantity = PercentLens.get(&back).expect("a percentage travels");
            assert_eq!(again, view, "PutGet failed for {view:?}");
        }
    }

    #[test]
    fn a_percentage_is_a_quantity_in_percent() {
        let view: Quantity = PercentLens
            .get(&proportion(ProportionKind::PkPercent, 42.0, 100.0))
            .expect("a percentage travels");
        assert_eq!(
            view.unit.as_ref().and_then(|unit| unit.value.as_deref()),
            Some("%"),
            "the unit is a percent"
        );
        assert_eq!(
            view.value.as_ref().and_then(|value| value.value.as_deref()),
            Some("42"),
            "the numerator is the magnitude"
        );
    }

    #[test]
    fn a_ratio_has_no_faithful_quantity() {
        let error = PercentLens
            .get(&proportion(ProportionKind::PkRatio, 5.0, 24.0))
            .expect_err("a ratio needs a denominator");
        assert_eq!(
            error,
            LensError::UnrepresentableProportion {
                kind: ProportionKind::PkRatio.value()
            },
            "the refusal names the kind"
        );
    }

    #[test]
    fn a_unitary_proportion_has_no_faithful_quantity() {
        let error = PercentLens
            .get(&proportion(ProportionKind::PkUnitary, 1.0, 3.0))
            .expect_err("a unitary proportion needs a denominator");
        assert!(
            matches!(error, LensError::UnrepresentableProportion { .. }),
            "the refusal names the kind: {error}"
        );
    }

    #[test]
    fn a_quantity_in_another_unit_is_not_a_proportion() {
        let view = Quantity {
            value: Some(Decimal {
                value: Some(String::from("5")),
                ..Decimal::default()
            }),
            unit: Some(FhirString::from("mg")),
            ..Quantity::default()
        };
        let error = PercentLens
            .put(&view, None)
            .expect_err("only a percentage is a proportion");
        assert!(
            matches!(error, LensError::Shape { attribute, .. } if attribute == "Quantity.unit"),
            "the refusal names the unit: {error}"
        );
    }
}
