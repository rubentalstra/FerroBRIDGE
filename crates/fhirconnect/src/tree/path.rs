// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `with.fhir` expression: its parts, its parser and its writability.
//!
//! The grammar is the one FHIRconnect writes. It is FHIRPath navigation plus
//! the two head forms the specification adds, `$resource` and `$fhirRoot`, and
//! the `^` parent operator it borrows from JSONPath Plus
//! (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/path_operators.html>).
//! The function forms are the ones FHIR profiles onto FHIRPath: `ofType()` and
//! the `as` operation on a polymorphic element, `extension(url)` and
//! `resolve()` (<https://hl7.org/fhir/R4/fhirpath.html>).
//!
//! ```text
//! path      := head ( "." step )*
//! head      := "$resource" | "$fhirRoot" | "^"+ | step
//! step      := function | name ( "[" digits "]" )?
//! function  := "ofType(" type ")" | "as(" type ")" | "extension(" string ")"
//!            | "resolve()" | "where(" expression ")" | "first()" | "last()"
//! ```
//!
//! Every expression is classified when it is parsed. A step that selects among
//! the values already in a document, rather than naming one element, makes the
//! whole expression [`Writability::ReadOnly`], and a mapping whose write side
//! names such an expression is refused when it is loaded.

use core::fmt;
use core::num::NonZeroUsize;
use core::str::FromStr;

use crate::tree::error::AnchorError;
use crate::tree::error::ParseError;

/// What a path expression starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Head {
    /// `$resource`, the root of the resource the mapping file names.
    Resource,
    /// `$fhirRoot`, the anchor the enclosing mapping step left.
    FhirRoot,
    /// A run of `^`, that many parents above the anchor.
    Parent(NonZeroUsize),
    /// No head variable, so the first step is a child of the anchor.
    Anchor,
}

impl fmt::Display for Head {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resource => f.write_str("$resource"),
            Self::FhirRoot => f.write_str("$fhirRoot"),
            Self::Parent(hops) => {
                for _ in 0..hops.get() {
                    f.write_str("^")?;
                }
                Ok(())
            }
            Self::Anchor => Ok(()),
        }
    }
}

/// Which spelling a type filter used.
///
/// FHIR gives a polymorphic element two spellings, the `as` operation and the
/// `ofType()` function (<https://hl7.org/fhir/R4/fhirpath.html>, §Polymorphism
/// in FHIR and §Additional functions). They select the same alternative, and
/// this model keeps the spelling so a diagnostic renders the expression as it
/// was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeForm {
    /// `ofType(Period)`.
    OfType,
    /// `as(Period)`.
    As,
}

/// Which end of a repeating element an ordinal filter takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ordinal {
    /// `first()`.
    First,
    /// `last()`.
    Last,
}

impl fmt::Display for Ordinal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::First => f.write_str("first()"),
            Self::Last => f.write_str("last()"),
        }
    }
}

/// One step of a path expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Step {
    /// An element name, or a choice element's suffixed name (`onsetPeriod`).
    Element {
        /// The name as written.
        name: String,
    },
    /// `ofType(T)` or `as(T)`.
    Type {
        /// The type the filter names, without any namespace qualifier.
        name: String,
        /// Which spelling was written.
        form: TypeForm,
    },
    /// `extension(url)`, the shortcut for the extension with that url.
    Extension {
        /// The url the literal carried.
        url: String,
    },
    /// `resolve()`, which hands the reference to the engine.
    Resolve,
    /// `first()` or `last()`.
    Ordinal(Ordinal),
    /// `[n]`, one position of a repeating element.
    Index(usize),
    /// `where(expression)`, a FHIRPath predicate.
    Predicate {
        /// The predicate as written, without the parentheses.
        expression: String,
    },
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Element { name } => f.write_str(name),
            Self::Type {
                name,
                form: TypeForm::OfType,
            } => write!(f, "ofType({name})"),
            Self::Type {
                name,
                form: TypeForm::As,
            } => write!(f, "as({name})"),
            Self::Extension { url } => write!(f, "extension('{url}')"),
            Self::Resolve => f.write_str("resolve()"),
            Self::Ordinal(ordinal) => write!(f, "{ordinal}"),
            Self::Index(index) => write!(f, "[{index}]"),
            Self::Predicate { expression } => write!(f, "where({expression})"),
        }
    }
}

impl Step {
    /// Why this step cannot be written through, `None` when it can be.
    const fn read_only(&self) -> Option<&'static str> {
        match self {
            Self::Predicate { .. } => {
                Some("is a predicate over the values a document already holds")
            }
            Self::Ordinal(_) => Some("picks one of the values a document already holds"),
            Self::Index(_) => Some("picks one position of the values a document already holds"),
            Self::Resolve => Some("names a resource the engine resolves, not an element to set"),
            Self::Element { .. } | Self::Type { .. } | Self::Extension { .. } => None,
        }
    }
}

/// Whether a path expression can be written through.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Writability {
    /// Every step names or creates one element.
    Writable,
    /// A step selects among the values a document already holds.
    ReadOnly {
        /// The step that makes it read-only, as written.
        step: String,
        /// Why that step cannot be written through.
        reason: &'static str,
    },
}

/// A parsed `with.fhir` expression.
///
/// # Examples
///
/// ```
/// use fhirconnect::tree::path::{FhirPath, Head, Writability};
///
/// let path: FhirPath = "$resource.onset.ofType(Period).start".parse()?;
/// assert_eq!(path.head(), Head::Resource);
/// assert_eq!(path.steps().len(), 3);
/// assert_eq!(path.writability(), &Writability::Writable);
///
/// let filtered: FhirPath = "$resource.category.first().coding".parse()?;
/// assert!(matches!(filtered.writability(), Writability::ReadOnly { .. }));
/// # Ok::<(), fhirconnect::tree::error::ParseError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FhirPath {
    text: String,
    head: Head,
    steps: Vec<Step>,
    writability: Writability,
}

impl FhirPath {
    /// Returns the expression as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns what the expression starts from.
    #[must_use]
    pub const fn head(&self) -> Head {
        self.head
    }

    /// Returns the steps, in path order.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Returns whether the expression can be written through.
    #[must_use]
    pub const fn writability(&self) -> &Writability {
        &self.writability
    }

    /// Builds the `$resource`-rooted expression this one names under `anchor`.
    ///
    /// `$fhirRoot` and a bare first step name the anchor itself, and each `^`
    /// names one parent above it
    /// (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/path_operators.html>).
    /// A `$resource`-rooted expression is already absolute and comes back
    /// unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`AnchorError::RelativeAnchor`] when `anchor` is itself not
    /// rooted at `$resource`, and [`AnchorError::AboveResource`] when the run
    /// of `^` passes the resource root.
    pub fn anchored(&self, anchor: &Self) -> Result<Self, AnchorError> {
        if self.head == Head::Resource {
            return Ok(self.clone());
        }
        if anchor.head != Head::Resource {
            return Err(AnchorError::RelativeAnchor {
                anchor: anchor.text.clone(),
            });
        }
        let hops = match self.head {
            Head::Parent(hops) => hops.get(),
            Head::Resource | Head::FhirRoot | Head::Anchor => 0,
        };
        let kept =
            anchor
                .steps
                .len()
                .checked_sub(hops)
                .ok_or_else(|| AnchorError::AboveResource {
                    expression: self.text.clone(),
                    anchor: anchor.text.clone(),
                    hops,
                })?;
        let mut steps: Vec<Step> = anchor.steps.iter().take(kept).cloned().collect();
        steps.extend(self.steps.iter().cloned());
        Ok(Self::assembled(Head::Resource, steps))
    }

    /// Builds the expression this one names under a stack of anchors.
    ///
    /// `anchors` runs innermost first. A run of `^` that exhausts one anchor
    /// carries on in the next, which is what the worked `^^` example needs:
    /// its two `^` start at the root of a resource a `reference` mapping
    /// initializes and reach an element of the enclosing resource
    /// (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/path_operators.html>,
    /// §Recurrence and parent elements). Crossing from one anchor to the next
    /// costs no `^`, because the boundary is not a step of either path. The
    /// index that comes back is the anchor the expression bound in.
    ///
    /// # Errors
    ///
    /// Returns [`AnchorError::RelativeAnchor`] when an anchor it reaches is
    /// itself not rooted at `$resource`, and [`AnchorError::AboveResource`]
    /// when the run of `^` passes the root of the outermost anchor.
    pub fn anchored_in(&self, anchors: &[&Self]) -> Result<(Self, usize), AnchorError> {
        if self.head == Head::Resource {
            return Ok((self.clone(), 0));
        }
        let requested = match self.head {
            Head::Parent(count) => count.get(),
            Head::Resource | Head::FhirRoot | Head::Anchor => 0,
        };
        let mut hops = requested;
        for (index, anchor) in anchors.iter().enumerate() {
            if anchor.head != Head::Resource {
                return Err(AnchorError::RelativeAnchor {
                    anchor: anchor.text.clone(),
                });
            }
            if let Some(kept) = anchor.steps.len().checked_sub(hops) {
                let mut steps: Vec<Step> = anchor.steps.iter().take(kept).cloned().collect();
                steps.extend(self.steps.iter().cloned());
                return Ok((Self::assembled(Head::Resource, steps), index));
            }
            hops = hops.saturating_sub(anchor.steps.len());
        }
        let outermost = anchors
            .last()
            .map_or_else(String::new, |anchor| anchor.text.clone());
        Err(AnchorError::AboveResource {
            expression: self.text.clone(),
            anchor: outermost,
            hops: requested,
        })
    }

    /// Assembles a path from a head and its steps, rendering and classifying
    /// it.
    fn assembled(head: Head, steps: Vec<Step>) -> Self {
        let writability = classify(&steps);
        let text = render(head, &steps);
        Self {
            text,
            head,
            steps,
            writability,
        }
    }
}

impl fmt::Display for FhirPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl FromStr for FhirPath {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut scanner = Scanner {
            text,
            at: 0usize,
            steps: Vec::new(),
        };
        let head = scanner.head()?;
        scanner.tail(head)?;
        let steps = scanner.steps;
        let writability = classify(&steps);
        Ok(Self {
            text: String::from(text),
            head,
            steps,
            writability,
        })
    }
}

/// Renders a head and its steps as an expression.
fn render(head: Head, steps: &[Step]) -> String {
    let mut text = head.to_string();
    for step in steps {
        if matches!(step, Step::Index(_)) {
            text.push_str(&step.to_string());
            continue;
        }
        if !text.is_empty() {
            text.push('.');
        }
        text.push_str(&step.to_string());
    }
    text
}

/// Classifies a step sequence as writable or read-only.
fn classify(steps: &[Step]) -> Writability {
    for step in steps {
        if let Some(reason) = step.read_only() {
            return Writability::ReadOnly {
                step: step.to_string(),
                reason,
            };
        }
    }
    Writability::Writable
}

/// The parser state: the expression, the byte offset reached and the steps
/// taken so far.
struct Scanner<'a> {
    text: &'a str,
    at: usize,
    steps: Vec<Step>,
}

impl<'a> Scanner<'a> {
    /// The text from the current offset on.
    fn rest(&self) -> &'a str {
        self.text.get(self.at..).unwrap_or_default()
    }

    /// The next character, without consuming it.
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Consumes and returns the next character.
    fn bump(&mut self) -> Option<char> {
        let next = self.peek()?;
        self.at = self.at.saturating_add(next.len_utf8());
        Some(next)
    }

    /// Consumes `wanted` when it stands next.
    fn eat(&mut self, wanted: char) -> bool {
        if self.peek() == Some(wanted) {
            self.at = self.at.saturating_add(wanted.len_utf8());
            return true;
        }
        false
    }

    /// Consumes `wanted` when it stands next, refusing anything else.
    fn expect(&mut self, wanted: char, expected: &'static str) -> Result<(), ParseError> {
        if self.eat(wanted) {
            return Ok(());
        }
        Err(self.expected(expected))
    }

    /// The refusal for whatever stands at the current offset.
    fn expected(&self, expected: &'static str) -> ParseError {
        let found = self
            .peek()
            .map_or_else(|| String::from("<end of input>"), String::from);
        ParseError::Expected {
            expected,
            found,
            at: self.at,
        }
    }

    /// Consumes the run of characters `accept` admits.
    fn take_while(&mut self, accept: impl Fn(char) -> bool) -> &'a str {
        let start = self.at;
        while self.peek().is_some_and(&accept) {
            let _consumed = self.bump();
        }
        self.text.get(start..self.at).unwrap_or_default()
    }

    /// Reads the head of the expression.
    fn head(&mut self) -> Result<Head, ParseError> {
        if self.text.is_empty() {
            return Err(ParseError::Empty);
        }
        if self.peek() == Some('$') {
            return self.variable();
        }
        if self.peek() == Some('^') {
            let start = self.at;
            let carets = self.take_while(|c| c == '^').chars().count();
            let hops = NonZeroUsize::new(carets).ok_or(ParseError::Expected {
                expected: "a `^` parent operator",
                found: String::from("^"),
                at: start,
            })?;
            return Ok(Head::Parent(hops));
        }
        self.step()?;
        Ok(Head::Anchor)
    }

    /// Reads a `$name` head.
    fn variable(&mut self) -> Result<Head, ParseError> {
        let start = self.at;
        let _dollar = self.bump();
        let name = self.take_while(char::is_alphanumeric);
        match name {
            "resource" => Ok(Head::Resource),
            "fhirRoot" => Ok(Head::FhirRoot),
            _ => Err(ParseError::UnknownVariable {
                name: String::from(name),
                at: start,
            }),
        }
    }

    /// Reads every step after the head.
    fn tail(&mut self, head: Head) -> Result<(), ParseError> {
        if head != Head::Anchor && self.peek().is_some() {
            self.expect('.', "`.` or the end of the expression")?;
            self.step()?;
        }
        while self.peek().is_some() {
            self.expect('.', "`.` or the end of the expression")?;
            self.step()?;
        }
        Ok(())
    }

    /// Reads one step, plus the index filter that may follow it.
    fn step(&mut self) -> Result<(), ParseError> {
        let start = self.at;
        let name = self.take_while(|c| c.is_alphanumeric() || c == '_');
        if name.is_empty() {
            return Err(self.expected("an element name or a path function"));
        }
        if self.eat('(') {
            let step = self.call(name, start)?;
            self.steps.push(step);
            return Ok(());
        }
        self.steps.push(Step::Element {
            name: String::from(name),
        });
        self.index(start)
    }

    /// Reads the `[n]` filter that may follow an element name.
    fn index(&mut self, start: usize) -> Result<(), ParseError> {
        if !self.eat('[') {
            return Ok(());
        }
        let digits = self.take_while(|c| c.is_ascii_digit());
        self.expect(']', "`]`")?;
        let index = digits.parse().map_err(|_error| ParseError::Index {
            text: String::from(digits),
            at: start,
        })?;
        self.steps.push(Step::Index(index));
        Ok(())
    }

    /// Reads a function call whose `(` is already consumed.
    fn call(&mut self, name: &str, start: usize) -> Result<Step, ParseError> {
        match name {
            "ofType" => self.type_filter(TypeForm::OfType),
            "as" => self.type_filter(TypeForm::As),
            "extension" => {
                let url = self.string()?;
                self.expect(')', "`)`")?;
                Ok(Step::Extension { url })
            }
            "resolve" => {
                self.expect(')', "`)`")?;
                Ok(Step::Resolve)
            }
            "first" => {
                self.expect(')', "`)`")?;
                Ok(Step::Ordinal(Ordinal::First))
            }
            "last" => {
                self.expect(')', "`)`")?;
                Ok(Step::Ordinal(Ordinal::Last))
            }
            "where" => {
                let expression = self.balanced(name, start)?;
                Ok(Step::Predicate { expression })
            }
            _ => Err(ParseError::UnknownFunction {
                name: String::from(name),
                at: start,
            }),
        }
    }

    /// Reads the type specifier of `ofType()` or `as()`.
    ///
    /// FHIRPath writes a type specifier as an identifier that may carry a
    /// namespace (`FHIR.Period`), and FHIR admits only concrete core types
    /// there (<https://hl7.org/fhir/R4/fhirpath.html>, §Additional functions).
    fn type_filter(&mut self, form: TypeForm) -> Result<Step, ParseError> {
        let mut name;
        loop {
            let segment = self.take_while(|c| c.is_alphanumeric() || c == '_');
            if segment.is_empty() {
                return Err(self.expected("a type name"));
            }
            name = String::from(segment);
            if !self.eat('.') {
                break;
            }
        }
        self.expect(')', "`)`")?;
        Ok(Step::Type { name, form })
    }

    /// Reads a FHIRPath string literal.
    fn string(&mut self) -> Result<String, ParseError> {
        let start = self.at;
        if !self.eat('\'') {
            return Err(self.expected("a string literal in single quotes"));
        }
        let mut text = String::new();
        loop {
            let Some(next) = self.bump() else {
                return Err(ParseError::UnterminatedString { at: start });
            };
            match next {
                '\'' => return Ok(text),
                '\\' => text.push(self.escape()?),
                other => text.push(other),
            }
        }
    }

    /// Reads the character after a backslash in a string literal.
    fn escape(&mut self) -> Result<char, ParseError> {
        let at = self.at.saturating_sub(1);
        let Some(next) = self.bump() else {
            return Err(ParseError::UnterminatedString { at });
        };
        match next {
            '\'' | '"' | '`' | '\\' | '/' => Ok(next),
            'r' => Ok('\r'),
            'n' => Ok('\n'),
            't' => Ok('\t'),
            'f' => Ok('\u{c}'),
            other => Err(ParseError::InvalidEscape { escape: other, at }),
        }
    }

    /// Reads the text up to the `)` that balances the already-consumed `(`.
    fn balanced(&mut self, name: &str, start: usize) -> Result<String, ParseError> {
        let opened = self.at;
        let mut depth = 1usize;
        loop {
            let Some(next) = self.bump() else {
                return Err(ParseError::UnterminatedCall {
                    name: String::from(name),
                    at: start,
                });
            };
            match next {
                '(' => depth = depth.saturating_add(1),
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let end = self.at.saturating_sub(1);
                        let text = self.text.get(opened..end).unwrap_or_default();
                        return Ok(String::from(text));
                    }
                }
                '\'' => {
                    self.at = self.at.saturating_sub(1);
                    let _literal = self.string()?;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
#[expect(clippy::panic_in_result_fn, reason = "test assertions")]
mod tests {
    use core::num::NonZeroUsize;

    use crate::tree::error::AnchorError;
    use crate::tree::error::ParseError;
    use crate::tree::path::FhirPath;
    use crate::tree::path::Head;
    use crate::tree::path::Ordinal;
    use crate::tree::path::Step;
    use crate::tree::path::TypeForm;
    use crate::tree::path::Writability;

    fn parse(text: &str) -> Result<FhirPath, ParseError> {
        text.parse()
    }

    #[test]
    fn the_two_fhir_side_variables_are_the_heads_the_specification_defines()
    -> Result<(), ParseError> {
        assert_eq!(parse("$resource")?.head(), Head::Resource);
        assert_eq!(parse("$fhirRoot")?.head(), Head::FhirRoot);
        assert_eq!(parse("code")?.head(), Head::Anchor);
        assert_eq!(
            parse("^^.use.coding")?.head(),
            Head::Parent(NonZeroUsize::new(2).unwrap())
        );
        Ok(())
    }

    #[test]
    fn an_openehr_side_variable_is_no_fhir_head() {
        let refused = parse("$archetype/data[at0001]");
        assert!(matches!(
            refused,
            Err(ParseError::UnknownVariable { ref name, at: 0 }) if name == "archetype"
        ));
    }

    #[test]
    fn the_three_fhirpath_forms_the_mapping_library_uses_parse() -> Result<(), ParseError> {
        assert_eq!(
            parse("onset.ofType(Period)")?.steps().last(),
            Some(&Step::Type {
                name: String::from("Period"),
                form: TypeForm::OfType,
            })
        );
        assert_eq!(
            parse("onset.as(DateTime)")?.steps().last(),
            Some(&Step::Type {
                name: String::from("DateTime"),
                form: TypeForm::As,
            })
        );
        assert_eq!(
            parse("$resource.extension('http://example.invalid/x')")?
                .steps()
                .last(),
            Some(&Step::Extension {
                url: String::from("http://example.invalid/x"),
            })
        );
        assert_eq!(
            parse("reasonReference.resolve().as(Condition).code")?
                .steps()
                .first(),
            Some(&Step::Element {
                name: String::from("reasonReference")
            })
        );
        Ok(())
    }

    #[test]
    fn a_namespace_qualifier_on_a_type_specifier_keeps_the_type_name() -> Result<(), ParseError> {
        assert_eq!(
            parse("value.ofType(FHIR.Quantity)")?.steps().last(),
            Some(&Step::Type {
                name: String::from("Quantity"),
                form: TypeForm::OfType,
            })
        );
        Ok(())
    }

    #[test]
    fn a_filter_makes_the_whole_expression_read_only() -> Result<(), ParseError> {
        for (expression, step) in [
            (
                "$resource.category.where(coding.code = 'x').text",
                "where(coding.code = 'x')",
            ),
            ("$resource.category.first().text", "first()"),
            ("$resource.category.last().text", "last()"),
            ("$resource.category[1].text", "[1]"),
            ("$resource.recorder.resolve().name", "resolve()"),
        ] {
            let path = parse(expression)?;
            let Writability::ReadOnly { step: named, .. } = path.writability() else {
                panic!("`{expression}` should have been classified read-only")
            };
            assert_eq!(named, step, "`{expression}` names the wrong offending step");
        }
        Ok(())
    }

    #[test]
    fn an_expression_of_element_names_and_type_filters_is_writable() -> Result<(), ParseError> {
        assert_eq!(
            parse("$resource.onset.ofType(Period).start")?.writability(),
            &Writability::Writable
        );
        Ok(())
    }

    #[test]
    fn an_index_filter_parses_as_its_own_step() -> Result<(), ParseError> {
        let path = parse("category[2].coding")?;
        assert_eq!(
            path.steps(),
            [
                Step::Element {
                    name: String::from("category")
                },
                Step::Index(2),
                Step::Element {
                    name: String::from("coding")
                },
            ]
        );
        assert_eq!(
            path.steps().get(1).map(ToString::to_string).as_deref(),
            Some("[2]")
        );
        Ok(())
    }

    #[test]
    fn a_predicate_keeps_its_nested_parentheses_and_quotes() -> Result<(), ParseError> {
        let path = parse("extension.where(url = 'a(b)')")?;
        assert_eq!(
            path.steps().last(),
            Some(&Step::Predicate {
                expression: String::from("url = 'a(b)'")
            })
        );
        Ok(())
    }

    #[test]
    fn every_refusal_names_its_token_and_offset() {
        assert!(matches!(parse(""), Err(ParseError::Empty)));
        assert!(matches!(
            parse("$resource.trim()"),
            Err(ParseError::UnknownFunction { ref name, at: 10 }) if name == "trim"
        ));
        assert!(matches!(
            parse("$resource..code"),
            Err(ParseError::Expected { at: 10, .. })
        ));
        assert!(matches!(
            parse("extension('unclosed"),
            Err(ParseError::UnterminatedString { at: 10 })
        ));
        assert!(matches!(
            parse("extension('a\\qb')"),
            Err(ParseError::InvalidEscape {
                escape: 'q',
                at: 12
            })
        ));
        assert!(matches!(
            parse("category.where(url = 'a'"),
            Err(ParseError::UnterminatedCall { ref name, at: 9 }) if name == "where"
        ));
    }

    #[test]
    fn an_ordinal_renders_as_written() {
        assert_eq!(Ordinal::First.to_string(), "first()");
        assert_eq!(Ordinal::Last.to_string(), "last()");
    }

    #[test]
    fn an_anchor_binds_the_relative_heads_to_a_resource_path()
    -> Result<(), Box<dyn core::error::Error>> {
        let anchor = parse("$resource.diagnosis.condition")?;
        assert_eq!(
            parse("$fhirRoot.reference")?.anchored(&anchor)?.as_str(),
            "$resource.diagnosis.condition.reference"
        );
        assert_eq!(
            parse("reference")?.anchored(&anchor)?.as_str(),
            "$resource.diagnosis.condition.reference"
        );
        assert_eq!(
            parse("^^.use.coding")?.anchored(&anchor)?.as_str(),
            "$resource.use.coding"
        );
        assert_eq!(
            parse("$resource.code")?.anchored(&anchor)?.as_str(),
            "$resource.code"
        );
        Ok(())
    }

    #[test]
    fn a_parent_run_above_the_resource_is_refused() -> Result<(), ParseError> {
        let anchor = parse("$resource.code")?;
        let refused = parse("^^.use")?.anchored(&anchor);
        assert!(matches!(
            refused,
            Err(AnchorError::AboveResource { hops: 2, .. })
        ));
        let relative = parse("code")?;
        assert!(matches!(
            parse("^.use")?.anchored(&relative),
            Err(AnchorError::RelativeAnchor { .. })
        ));
        Ok(())
    }

    #[test]
    fn a_caret_run_carries_on_in_the_next_anchor() -> Result<(), Box<dyn core::error::Error>> {
        let inside = parse("$resource")?;
        let outside = parse("$resource.diagnosis.condition.reference")?;
        let (bound, index) = parse("^^.use.coding")?.anchored_in(&[&inside, &outside])?;
        assert_eq!(bound.as_str(), "$resource.diagnosis.use.coding");
        assert_eq!(index, 1, "the expression binds in the enclosing resource");
        let (near, index) = parse("^.use")?.anchored_in(&[&inside, &outside])?;
        assert_eq!(near.as_str(), "$resource.diagnosis.condition.use");
        assert_eq!(index, 1);
        Ok(())
    }

    #[test]
    fn a_caret_run_past_the_outermost_anchor_is_refused() -> Result<(), Box<dyn core::error::Error>>
    {
        let inside = parse("$resource")?;
        let outside = parse("$resource.diagnosis")?;
        let refused = parse("^^^.use")?.anchored_in(&[&inside, &outside]);
        assert!(matches!(
            refused,
            Err(AnchorError::AboveResource { hops: 3, .. })
        ));
        Ok(())
    }

    #[test]
    fn anchoring_re_classifies_the_assembled_expression() -> Result<(), Box<dyn core::error::Error>>
    {
        let anchor = parse("$resource.category.first()")?;
        let bound = parse("coding.code")?.anchored(&anchor)?;
        assert!(matches!(bound.writability(), Writability::ReadOnly { .. }));
        assert_eq!(bound.as_str(), "$resource.category.first().coding.code");
        Ok(())
    }
}
