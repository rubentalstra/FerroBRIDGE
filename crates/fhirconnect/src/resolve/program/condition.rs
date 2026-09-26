// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A resolved condition.

use core::fmt;

use crate::model::ast::keyword::ConditionOperator;
use crate::model::ast::keyword::Direction;

use crate::resolve::program::target::Attachment;
use crate::resolve::program::target::Target;

/// The parts of a compiled condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionParts {
    /// The direction that evaluates the condition.
    pub direction: Direction,
    /// The resolved `targetRoot`.
    pub target: Target,
    /// The resolved target attributes.
    pub attributes: Vec<Target>,
    /// The operator.
    pub operator: ConditionOperator,
    /// The values the operator tests against.
    pub criteria: Vec<String>,
    /// Whether the condition identifies the element it guards.
    pub identifying: bool,
    /// How the `targetRoot` stands to the path the condition guards.
    pub attachment: Attachment,
}

/// A compiled `fhirCondition` or `openehrCondition`.
///
/// A condition is evaluated on the input side only, so [`Condition::direction`]
/// says which direction runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    direction: Direction,
    target: Target,
    attributes: Vec<Target>,
    operator: ConditionOperator,
    criteria: Vec<String>,
    identifying: bool,
    attachment: Attachment,
}

impl Condition {
    /// Assembles a compiled condition.
    #[must_use]
    pub fn new(parts: ConditionParts) -> Self {
        Self {
            direction: parts.direction,
            target: parts.target,
            attributes: parts.attributes,
            operator: parts.operator,
            criteria: parts.criteria,
            identifying: parts.identifying,
            attachment: parts.attachment,
        }
    }

    /// Returns how the `targetRoot` stands to the path this condition guards.
    #[must_use]
    pub const fn attachment(&self) -> Attachment {
        self.attachment
    }

    /// Returns the direction that evaluates this condition.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Returns the resolved `targetRoot`.
    #[must_use]
    pub const fn target(&self) -> &Target {
        &self.target
    }

    /// Returns the resolved target attributes, which are tested with OR.
    #[must_use]
    pub fn attributes(&self) -> &[Target] {
        &self.attributes
    }

    /// Returns the operator.
    #[must_use]
    pub const fn operator(&self) -> ConditionOperator {
        self.operator
    }

    /// Returns the criteria the operator tests against.
    #[must_use]
    pub fn criteria(&self) -> &[String] {
        &self.criteria
    }

    /// Whether the condition identifies the element it guards.
    #[must_use]
    pub const fn identifying(&self) -> bool {
        self.identifying
    }
}

/// Writes one compiled condition.
pub(super) fn render_condition(
    f: &mut fmt::Formatter<'_>,
    pad: &str,
    label: &str,
    condition: &Condition,
) -> fmt::Result {
    writeln!(
        f,
        "{pad}{label} {} {} on {} ({})",
        condition.operator(),
        render_criteria(condition.criteria()),
        condition.target(),
        condition.attachment()
    )?;
    for attribute in condition.attributes() {
        writeln!(f, "{pad}  attribute {attribute}")?;
    }
    Ok(())
}

/// Renders a criteria list as one stable string.
fn render_criteria(criteria: &[String]) -> String {
    if criteria.is_empty() {
        return String::from("[]");
    }
    format!("[{}]", criteria.join(", "))
}
