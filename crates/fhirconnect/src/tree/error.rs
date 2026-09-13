// SPDX-FileCopyrightText: Ruben Talstra
// SPDX-License-Identifier: BUSL-1.1

//! Why a path expression, its resolution, a read or a write was refused.
//!
//! Every refusal is a typed variant carrying the token, the byte offset or the
//! element path it is about, so a caller branches on the variant and a reader
//! of the diagnostic sees the offending piece of the expression. A wrapping
//! variant carries its cause, so the chain can be walked (RFC 0201).

/// Why a `with.fhir` expression could not be parsed.
///
/// Every variant names the offending token and the byte offset it starts at,
/// counted from the first byte of the expression.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The expression is empty.
    #[error("a `with.fhir` expression is never empty")]
    Empty,
    /// A `$name` head that FHIRconnect does not define for the FHIR side.
    ///
    /// The FHIR-side variables are `$resource` and `$fhirRoot`
    /// (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/Variables.html>).
    #[error(
        "`${name}` at byte {at} is no FHIR-side variable; FHIRconnect defines `$resource` and `$fhirRoot`"
    )]
    UnknownVariable {
        /// The name written after the `$`.
        name: String,
        /// The byte offset of the `$`.
        at: usize,
    },
    /// A function call this path model does not implement.
    ///
    /// The functions accepted are `ofType`, `as`, `extension`, `resolve`,
    /// `where`, `first` and `last`.
    #[error("`{name}(` at byte {at} is no path function of this model")]
    UnknownFunction {
        /// The name written before the `(`.
        name: String,
        /// The byte offset of the name.
        at: usize,
    },
    /// The expression holds something else where a construct was required.
    #[error("expected {expected} at byte {at}, found `{found}`")]
    Expected {
        /// What the grammar required there.
        expected: &'static str,
        /// What stands there, or `<end of input>`.
        found: String,
        /// The byte offset it starts at.
        at: usize,
    },
    /// A string literal that never closes.
    #[error("the string literal opened at byte {at} never closes")]
    UnterminatedString {
        /// The byte offset of the opening quote.
        at: usize,
    },
    /// A function call whose parentheses never balance.
    #[error("the call to `{name}` opened at byte {at} never closes")]
    UnterminatedCall {
        /// The function name.
        name: String,
        /// The byte offset of the name.
        at: usize,
    },
    /// An escape sequence a FHIRPath string literal does not define.
    ///
    /// FHIRPath defines `\'`, `\"`, `` \` ``, `\r`, `\n`, `\t`, `\f`, `\\`,
    /// `\/` and `\uXXXX` (<https://hl7.org/fhirpath/N1/#literals>).
    #[error("`\\{escape}` at byte {at} is no FHIRPath string escape")]
    InvalidEscape {
        /// The character written after the backslash.
        escape: char,
        /// The byte offset of the backslash.
        at: usize,
    },
    /// An index that is no `usize`.
    #[error("the index `{text}` at byte {at} is no array position")]
    Index {
        /// The digits as written.
        text: String,
        /// The byte offset of the `[`.
        at: usize,
    },
}

/// Why a parsed path could not be bound to an anchor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnchorError {
    /// The anchor is itself relative, so it names no place in a resource.
    #[error("the anchor `{anchor}` is not rooted at `$resource`")]
    RelativeAnchor {
        /// The anchor as written.
        anchor: String,
    },
    /// More `^` operators than the anchor has steps.
    ///
    /// `^` names the parent of the current FHIR anchor, so a run of them that
    /// passes the resource root names nothing
    /// (<https://sevkohler.github.io/FHIRconnect-spec/build/site/FHIRconnect/v1.0.0/basics/path_operators.html>).
    #[error(
        "`{expression}` steps {hops} parents above the anchor `{anchor}`, which is above `$resource`"
    )]
    AboveResource {
        /// The expression as written.
        expression: String,
        /// The anchor as written.
        anchor: String,
        /// How many parents the expression asks for.
        hops: usize,
    },
}

/// Why a path does not resolve against the element table.
///
/// Every variant names the element path the table holds, for example
/// `Condition.onset[x]`, so the refusal points at the definition rather than
/// at the expression alone.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// The path is not rooted at `$resource`, so it names no starting type.
    #[error("`{expression}` must be bound to an anchor before it resolves")]
    NotAnchored {
        /// The expression as written.
        expression: String,
    },
    /// The document's `resourceType` names no resource this table defines.
    #[error("`{name}` is no resource type of this FHIR version")]
    UnknownResource {
        /// The name the document carried.
        name: String,
    },
    /// The type has no element of that name.
    #[error("`{owner}` defines no element `{name}`")]
    UnknownElement {
        /// The element path of the type the step was taken in.
        owner: String,
        /// The name the step asked for.
        name: String,
    },
    /// A type filter that names no alternative of a choice element.
    #[error("`{element}` admits {admitted}, not `{requested}`")]
    ChoiceType {
        /// The element path of the choice, for example `Condition.onset[x]`.
        element: String,
        /// The type the filter named.
        requested: String,
        /// The type codes the definition lists.
        admitted: String,
    },
    /// A type filter that is not the type of the element it stands on.
    #[error("`{element}` is `{actual}`, not `{requested}`")]
    TypeAssertion {
        /// The element path the filter stands on.
        element: String,
        /// The type the filter named.
        requested: String,
        /// The type the definition gives the element.
        actual: String,
    },
    /// A step through a choice element no type filter has resolved.
    #[error(
        "`{element}` is a choice element; name its type with `ofType()`, `as()` or the suffixed element name before stepping through it"
    )]
    UnresolvedChoice {
        /// The element path of the choice.
        element: String,
    },
    /// A step through an element that holds no further elements.
    #[error("`{element}` holds no element `{name}`")]
    NoChildren {
        /// The element path the step was taken from.
        element: String,
        /// The name the step asked for.
        name: String,
    },
    /// A step through an element whose resource type only the document names.
    #[error(
        "`{element}` holds a resource; name its type with `ofType()` or `as()` before stepping through it"
    )]
    ResourceContent {
        /// The element path of the resource-typed element.
        element: String,
    },
    /// `resolve()` on an element that is no reference.
    ///
    /// `resolve()` reads "a string that is a uri (or canonical or url)"
    /// (<https://hl7.org/fhir/R4/fhirpath.html>, §Additional functions), which
    /// in the object model is a `Reference` or a uri-typed primitive.
    #[error("`{element}` is no reference, so `resolve()` has nothing to resolve")]
    NotAReference {
        /// The element path the call stands on.
        element: String,
    },
    /// An index filter on an element that does not repeat.
    #[error("`{element}` does not repeat, so `[{index}]` selects nothing")]
    NotRepeating {
        /// The element path of the scalar element.
        element: String,
        /// The index the filter asked for.
        index: usize,
    },
}

/// Why a read over a document was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The document carries no `resourceType`.
    ///
    /// Every FHIR resource names its type in JSON
    /// (<https://hl7.org/fhir/R4/json.html>).
    #[error("the document carries no `resourceType`")]
    NoResourceType,
    /// The path does not resolve against the element table.
    #[error("the path does not resolve")]
    Resolve {
        /// The refusal the element table returned.
        #[source]
        source: ResolveError,
    },
    /// The document holds a JSON kind the element's cardinality forbids.
    #[error("`{element}` is {expected} in the JSON representation, and the document holds {found}")]
    Shape {
        /// The element path from the table.
        element: String,
        /// The JSON kind the representation requires.
        expected: &'static str,
        /// The JSON kind the document holds.
        found: &'static str,
    },
    /// The document carries more than one alternative of a choice element.
    ///
    /// A choice element takes one type
    /// (<https://hl7.org/fhir/R4/formats.html#choice>), so two suffixed
    /// members of one stem is a defective document.
    #[error(
        "`{element}` carries both `{first}` and `{second}`, and a choice element takes one type"
    )]
    AmbiguousChoice {
        /// The element path of the choice.
        element: String,
        /// The first suffixed member found.
        first: String,
        /// The second suffixed member found.
        second: String,
    },
    /// A `where()` predicate, which this path model does not evaluate.
    ///
    /// The forms implemented are the ones FHIRconnect uses, `ofType()`,
    /// `as()`, `extension()` and `resolve()`; a general FHIRPath predicate is
    /// a whole evaluator.
    #[error(
        "`where({expression})` is a FHIRPath predicate, which this path model does not evaluate"
    )]
    Predicate {
        /// The predicate as written.
        expression: String,
    },
}

/// Why a write into a document was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The expression selects among existing values, so nothing writes through
    /// it.
    #[error("`{expression}` is read-only: `{step}` {reason}")]
    ReadOnly {
        /// The expression as written.
        expression: String,
        /// The step that makes it read-only.
        step: String,
        /// Why that step cannot be written through.
        reason: &'static str,
    },
    /// The document carries no `resourceType`.
    #[error("the document carries no `resourceType`")]
    NoResourceType,
    /// The path does not resolve against the element table.
    #[error("the path does not resolve")]
    Resolve {
        /// The refusal the element table returned.
        #[source]
        source: ResolveError,
    },
    /// The path ends on a choice element no type filter has resolved.
    #[error(
        "`{element}` is a choice element; the type to write is named by `ofType()`, `as()` or the suffixed element name"
    )]
    UnresolvedChoice {
        /// The element path of the choice.
        element: String,
    },
    /// The path ends on a reference the engine resolves later.
    #[error("`{expression}` ends on a reference the engine resolves, which is read-only here")]
    ReferenceTarget {
        /// The expression as written.
        expression: String,
    },
    /// A repeating element with no index to write at.
    #[error("`{element}` repeats, so a write names the occurrence index to set")]
    MissingOccurrence {
        /// The element path of the repeating element.
        element: String,
    },
    /// More indices than the path has repeating elements.
    #[error("`{element}` does not repeat, so the occurrence index {index} selects nothing")]
    NotRepeating {
        /// The element path the write ends on.
        element: String,
        /// The first index with no repeating element to spend it on.
        index: usize,
    },
    /// An index past the end of the array plus one.
    ///
    /// A FHIR array holds no gap, so a write extends it by one at a time
    /// (<https://hl7.org/fhir/R4/json.html>).
    #[error("`{element}` holds {length} occurrences, so index {index} would leave a gap")]
    OccurrenceGap {
        /// The element path of the repeating element.
        element: String,
        /// The index the write asked for.
        index: usize,
        /// How many occurrences the document holds.
        length: usize,
    },
    /// The document holds a JSON kind the write cannot step through.
    #[error("`{element}` is {expected} in the JSON representation, and the document holds {found}")]
    Shape {
        /// The element path from the table.
        element: String,
        /// The JSON kind the representation requires.
        expected: &'static str,
        /// The JSON kind the document holds.
        found: &'static str,
    },
    /// A value of a kind the element does not admit.
    #[error("`{element}` is {expected} in the JSON representation, and the value is {found}")]
    TypeNotAdmitted {
        /// The element path from the table.
        element: String,
        /// The JSON kind the element admits.
        expected: &'static str,
        /// The JSON kind the value has.
        found: &'static str,
    },
    /// A `null` value.
    ///
    /// JSON `null` appears in FHIR only as the padding of a repeating
    /// primitive's element array (<https://hl7.org/fhir/R4/json.html>), never
    /// as a value a mapping writes.
    #[error("`{element}` takes no `null`; an absent value is an absent element")]
    NullValue {
        /// The element path from the table.
        element: String,
    },
}
