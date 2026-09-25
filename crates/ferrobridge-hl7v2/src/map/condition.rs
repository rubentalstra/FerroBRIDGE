// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `Computable-ANTLR` conditions of the v2-to-FHIR `ConceptMaps`.
//!
//! The mapping guidelines list the forms (`mapping_guidelines.md` §Conditions):
//! `IF X EQUALS "A"`, `NOT EQUALS`, `VALUED`, `NOT VALUED`, `NOT VALUED ERROR`,
//! `IN (...)`, `NOT IN (...)`, `AND`, `OR`, `NOT (...)`, parentheses, and the
//! `LST.COUNT` and `LENGTH` comparisons. The guide publishes no grammar file,
//! so the grammar below is written from that list and from the forms the
//! vendored corpus uses; where the list is silent the grammar is our own
//! design. A `logos` lexer feeds a `chumsky` parser.
//!
//! ```text
//! condition := "IF" or
//! or        := and ( "OR" "IF"? and )*
//! and       := unary ( "AND" "IF"? unary )*
//! unary     := "NOT" group | group | test
//! group     := "(" "IF"? or ")"
//! test      := operand ( ( "AND" | "OR" ) operand )* check
//! check     := "IS"? "VALUED" | "IS"? "NOT" "VALUED" "ERROR"? | "DOES" "NOT" "EXIST"
//!            | "IS" "EMPTY" | ( "EQUALS" | "=" | "IS" ) literal
//!            | ( "NOT" "EQUALS" | "IS" "NOT" ) literal | "IS"? "NOT"? "IN" list
//!            | ( "LST.COUNT" | "LENGTH" ) compare number
//! compare   := "EQUALS" | "=" | "NOT" "EQUALS"
//!            | ( "GREATER" | "LESS" ) "THAN" ( "OR" "EQUALS" )?
//! operand   := SEG "-" n ( "." n )* | TYPE ( "." | "-" ) n ( "." n )* | SEG
//! ```
//!
//! Keywords are matched without regard to case, because the corpus writes
//! `LST.count` beside `LST.COUNT`. An operand list before one check
//! (`IF PID-33 AND PID-34 VALUED`) applies the check to each operand, joined
//! by the connective written. A name before `-` is a field of a segment when
//! the v2 definitions name that segment, else a component of the data type
//! they name, as the `hd-endpoint` maps write `HD-3` beside `HD.2`. An `IN`,
//! `NOT IN`, `VALUED` or `NOT VALUED` with no operand before it reads the
//! row's own source ([`parse_for`]); those on a row with no source, and any
//! other operand-less check, are [`ConditionError::NoOperand`].
//! Anything else is refused at load, and the row is a counted outcome.
//!
//! The `assignment` of a row ([`assignment`]) shares the lexer: quoted
//! literals and operands joined by `+`, as `"urn:oid:"+HD.2`.
//!
//! ```text
//! assignment := part ( "+" part )*
//! part       := literal | operand
//! ```

use core::fmt;

use chumsky::prelude::*;
use logos::Logos;

/// A literal in a condition, always a double-quoted string.
pub type Literal = String;

/// A comparison of a count or a length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compare {
    /// `EQUALS`.
    Equal,
    /// `NOT EQUALS`.
    NotEqual,
    /// `GREATER THAN`.
    Greater,
    /// `GREATER THAN OR EQUALS`.
    GreaterOrEqual,
    /// `LESS THAN`.
    Less,
    /// `LESS THAN OR EQUALS`.
    LessOrEqual,
}

impl Compare {
    /// Applies the comparison.
    #[must_use]
    pub const fn holds(self, left: u64, right: u64) -> bool {
        match self {
            Self::Equal => left == right,
            Self::NotEqual => left != right,
            Self::Greater => left > right,
            Self::GreaterOrEqual => left >= right,
            Self::Less => left < right,
            Self::LessOrEqual => left <= right,
        }
    }
}

/// What a check asks of an operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// `VALUED`.
    Valued,
    /// `NOT VALUED`; with `error`, the mapper stops and raises an error when it
    /// holds.
    NotValued {
        /// Whether `ERROR` followed.
        error: bool,
    },
    /// `EQUALS "A"`.
    Equals(Literal),
    /// `NOT EQUALS "A"`.
    NotEquals(Literal),
    /// `IN ("A", "B")`.
    In(Vec<Literal>),
    /// `NOT IN ("A", "B")`.
    NotIn(Vec<Literal>),
    /// `LST.COUNT` compared with a number: the repetitions of a field.
    Count(Compare, u64),
    /// `LENGTH` compared with a number: the characters of the value.
    Length(Compare, u64),
}

/// What an operand names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// `SEG-n.m`: a field of a segment, then components.
    Field {
        /// The segment id.
        segment: String,
        /// The field position, then component and subcomponent positions.
        path: Vec<usize>,
    },
    /// `TYPE.n.m`: a component of the data type the map is about.
    Component {
        /// The data type code.
        datatype: String,
        /// The component position, then the subcomponent position.
        path: Vec<usize>,
    },
    /// `SEG`: a whole segment of the message.
    Segment(String),
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let join = |path: &[usize]| {
            path.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(".")
        };
        match self {
            Self::Field { segment, path } => write!(f, "{segment}-{}", join(path)),
            Self::Component { datatype, path } => write!(f, "{datatype}.{}", join(path)),
            Self::Segment(segment) => f.write_str(segment),
        }
    }
}

/// A parsed condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// Both hold.
    And(Box<Expr>, Box<Expr>),
    /// Either holds.
    Or(Box<Expr>, Box<Expr>),
    /// The inner one does not hold.
    Not(Box<Expr>),
    /// A check of one operand.
    Test(Operand, Check),
}

/// Why a condition could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConditionError {
    /// A character sequence no token of the grammar matches.
    #[error("no token of the condition grammar matches {slice:?} at byte {offset}")]
    Lex {
        /// The byte offset.
        offset: usize,
        /// The text no token matches.
        slice: String,
    },
    /// The tokens do not match the grammar.
    #[error("the condition leaves the grammar at token {token}")]
    Syntax {
        /// The index of the first token not admitted.
        token: usize,
    },
    /// A check follows `IF`, `AND` or `OR` with no operand before it, as
    /// `IF NOT VALUED` does.
    #[error("the check at token {token} names no operand")]
    NoOperand {
        /// The index of the check's first token.
        token: usize,
    },
}

/// One part of an assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// A double-quoted literal, written as given.
    Literal(Literal),
    /// An operand, whose value is written.
    Operand(Operand),
}

/// Why an assignment could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssignmentError {
    /// A character sequence no token of the grammar matches.
    #[error("no token of the assignment grammar matches {slice:?} at byte {offset}")]
    Lex {
        /// The byte offset.
        offset: usize,
        /// The text no token matches.
        slice: String,
    },
    /// The tokens do not match the grammar.
    #[error("the assignment leaves the grammar at token {token}")]
    Syntax {
        /// The index of the first token not admitted.
        token: usize,
    },
    /// Two parts follow each other with no `+` between them, as
    /// `RP.3"/"RP.4` does.
    #[error("parts {token} and the next are joined by no +")]
    MissingOperator {
        /// The index of the first of the two parts.
        token: usize,
    },
}

/// One token of a condition.
#[derive(Logos, Debug, Clone, PartialEq, Eq)]
#[logos(skip r"[ \t]+")]
enum Token {
    #[token("if", ignore(case))]
    If,
    #[token("and", ignore(case))]
    And,
    #[token("or", ignore(case))]
    Or,
    #[token("not", ignore(case))]
    Not,
    #[token("is", ignore(case))]
    Is,
    #[token("valued", ignore(case))]
    Valued,
    #[token("error", ignore(case))]
    Error,
    #[token("equals", ignore(case))]
    Equals,
    #[token("in", ignore(case))]
    In,
    #[token("does", ignore(case))]
    Does,
    #[token("exist", ignore(case))]
    Exist,
    #[token("empty", ignore(case))]
    Empty,
    #[token("lst.count", ignore(case))]
    Count,
    #[token("length", ignore(case))]
    Length,
    #[token("greater", ignore(case))]
    Greater,
    #[token("less", ignore(case))]
    Less,
    #[token("than", ignore(case))]
    Than,
    #[token("=")]
    EqualSign,
    #[token("(")]
    Open,
    #[token(")")]
    Close,
    #[token(",")]
    Comma,
    #[token("+")]
    Plus,
    #[regex(r#""[^"]*""#, |lexer| unquote(lexer.slice()))]
    Text(String),
    #[regex(r"[0-9]+", |lexer| lexer.slice().parse::<u64>().ok())]
    Number(u64),
    #[regex(r"[A-Z][A-Z0-9]{1,3}-[0-9]+(\.[0-9]+)*", |lexer| dashed(lexer.slice()))]
    Field(Operand),
    #[regex(r"[A-Z][A-Z0-9]{1,3}\.[0-9]+(\.[0-9]+)*", |lexer| component(lexer.slice()))]
    Component(Operand),
    #[regex(r"[A-Z][A-Z0-9]{2}", |lexer| String::from(lexer.slice()), priority = 1)]
    Segment(String),
}

/// The text of a double-quoted literal.
fn unquote(slice: &str) -> String {
    String::from(
        slice
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(slice),
    )
}

/// The positions of `n.m`, `None` for a zero.
fn positions(text: &str) -> Option<Vec<usize>> {
    text.split('.')
        .map(|part| part.parse::<usize>().ok().filter(|position| *position > 0))
        .collect()
}

/// A `NAME-n.m` operand: a field when the v2 definitions name the segment,
/// else a component when they name the data type, else none.
///
/// No specification governs this: our own design; the guide's `hd-endpoint`
/// maps spell one component `HD.3` and `HD-3`, and no segment id is a data
/// type code in the v2 definitions.
fn dashed(slice: &str) -> Option<Operand> {
    let (name, path) = slice.split_once('-')?;
    let path = positions(path)?;
    if hl7v2_types::segment::find(name).is_some() {
        Some(Operand::Field {
            segment: String::from(name),
            path,
        })
    } else {
        hl7v2_types::data_type::find(name).map(|_| Operand::Component {
            datatype: String::from(name),
            path,
        })
    }
}

/// A `TYPE.n.m` operand.
fn component(slice: &str) -> Option<Operand> {
    let (datatype, path) = slice.split_once('.')?;
    Some(Operand::Component {
        datatype: String::from(datatype),
        path: positions(path)?,
    })
}

/// The chumsky extra parameter of this grammar.
#[expect(
    unused_qualifications,
    reason = "the alias shadows the name it qualifies, so the path keeps the definition from naming itself"
)]
type Extra<'a> = chumsky::extra::Err<Simple<'a, Token>>;

/// Parses a `Computable-ANTLR` condition.
///
/// # Errors
///
/// Returns [`ConditionError::Lex`] for text no token matches and
/// [`ConditionError::Syntax`] for tokens outside the grammar.
pub fn parse(text: &str) -> Result<Expr, ConditionError> {
    parse_for(text, None)
}

/// Parses a `Computable-ANTLR` condition of a row whose source is `source`,
/// reading an `IN`, `NOT IN`, `VALUED` or `NOT VALUED` check with no operand
/// before it as a check of `source`.
///
/// The mapping guidelines list `IN` and `NOT IN` without an operand
/// (`mapping_guidelines.md` §Conditions: `IF IN ("A","B", "C")`, `IF NOT IN
/// ("A","B", "C")`), and a condition decides whether the row's own v2
/// element is mapped. They list only those two; reading `VALUED` and
/// `NOT VALUED` the same way is our own reading. Every other check names its
/// operand.
///
/// # Errors
///
/// Returns [`ConditionError::Lex`] for text no token matches,
/// [`ConditionError::NoOperand`] for a check with no operand that `source`
/// does not supply, and [`ConditionError::Syntax`] for tokens outside the
/// grammar.
pub fn parse_for(text: &str, source: Option<&Operand>) -> Result<Expr, ConditionError> {
    let tokens = lex(text).map_err(|(offset, slice)| ConditionError::Lex { offset, slice })?;
    condition(source.cloned())
        .parse(tokens.as_slice())
        .into_result()
        .map_err(|errors| match no_operand(&tokens) {
            Some(token) => ConditionError::NoOperand { token },
            None => ConditionError::Syntax {
                token: errors
                    .first()
                    .map_or(tokens.len(), |error| error.span().start),
            },
        })
}

/// Returns the operand a row's source code names.
///
/// That is a field for `MSH-24`, a component for `HD.3`, a segment for `PID`,
/// and `None` for a message path such as `ORU_R01.MSH` or a whole data type
/// such as `MSG`.
#[must_use]
pub fn source_operand(code: &str) -> Option<Operand> {
    match lex(code).ok()?.as_slice() {
        [Token::Field(operand) | Token::Component(operand)] => Some(operand.clone()),
        [Token::Segment(id)] => {
            hl7v2_types::segment::find(id).map(|_| Operand::Segment(id.clone()))
        }
        _ => None,
    }
}

/// Parses a row's `assignment` as literals and operands joined by `+`.
///
/// # Errors
///
/// Returns [`AssignmentError::Lex`] for text no token matches,
/// [`AssignmentError::MissingOperator`] for two parts with no `+` between
/// them, and [`AssignmentError::Syntax`] for other tokens outside the
/// grammar.
pub fn assignment(text: &str) -> Result<Vec<Part>, AssignmentError> {
    let tokens = lex(text).map_err(|(offset, slice)| AssignmentError::Lex { offset, slice })?;
    concatenation()
        .parse(tokens.as_slice())
        .into_result()
        .map_err(|errors| {
            let adjacent = tokens
                .windows(2)
                .position(|pair| matches!(pair, [left, right] if is_part(left) && is_part(right)));
            match adjacent {
                Some(token) => AssignmentError::MissingOperator { token },
                None => AssignmentError::Syntax {
                    token: errors
                        .first()
                        .map_or(tokens.len(), |error| error.span().start),
                },
            }
        })
}

/// The assignment grammar.
fn concatenation<'a>() -> impl Parser<'a, &'a [Token], Vec<Part>, Extra<'a>> {
    let part = select! {
        Token::Text(text) => Part::Literal(text),
        Token::Field(operand) => Part::Operand(operand),
        Token::Component(operand) => Part::Operand(operand),
    };
    part.separated_by(just(Token::Plus))
        .at_least(1)
        .collect::<Vec<_>>()
        .then_ignore(end())
}

/// Whether `token` is a part of an assignment.
const fn is_part(token: &Token) -> bool {
    matches!(
        token,
        Token::Text(_) | Token::Field(_) | Token::Component(_)
    )
}

/// The tokens of `text`, or the offset and text of the first sequence no
/// token matches.
fn lex(text: &str) -> Result<Vec<Token>, (usize, String)> {
    let mut tokens = Vec::new();
    let mut lexer = Token::lexer(text);
    while let Some(token) = lexer.next() {
        match token {
            Ok(token) => tokens.push(token),
            Err(()) => return Err((lexer.span().start, String::from(lexer.slice()))),
        }
    }
    Ok(tokens)
}

/// The index of the first check that directly follows `IF`, `AND` or `OR`,
/// with no operand before it.
fn no_operand(tokens: &[Token]) -> Option<usize> {
    tokens
        .windows(3)
        .position(|window| match window {
            [Token::If | Token::And | Token::Or, next, after] => match next {
                Token::Valued | Token::In | Token::Equals | Token::EqualSign => true,
                Token::Not => matches!(
                    after,
                    Token::Valued | Token::In | Token::Equals | Token::Text(_)
                ),
                _ => false,
            },
            _ => false,
        })
        .map(|position| position.saturating_add(1))
        .or_else(|| match tokens {
            [.., Token::If | Token::And | Token::Or, Token::Valued] => {
                Some(tokens.len().saturating_sub(1))
            }
            _ => None,
        })
}

/// The condition grammar, reading an operand-less `IN` or `NOT IN` as a
/// check of `source` when there is one.
fn condition<'a>(source: Option<Operand>) -> impl Parser<'a, &'a [Token], Expr, Extra<'a>> {
    let operand = select! {
        Token::Field(operand) => operand,
        Token::Component(operand) => operand,
    }
    .or(
        select! { Token::Segment(id) => id }.try_map(|id: String, span| {
            hl7v2_types::segment::find(&id)
                .map(|_| Operand::Segment(id))
                .ok_or_else(|| Simple::new(None, span))
        }),
    );
    let connective = just(Token::And).to(true).or(just(Token::Or).to(false));
    let test = operand
        .then(connective.then(operand).repeated().collect::<Vec<_>>())
        .then(check())
        .map(|((first, rest), check)| {
            let mut expr = Expr::Test(first, check.clone());
            for (conjunction, operand) in rest {
                let next = Box::new(Expr::Test(operand, check.clone()));
                expr = if conjunction {
                    Expr::And(Box::new(expr), next)
                } else {
                    Expr::Or(Box::new(expr), next)
                };
            }
            expr
        });
    let quoted = select! { Token::Text(quoted) => quoted };
    let list = quoted
        .separated_by(just(Token::Comma))
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Open), just(Token::Close));
    let is = just(Token::Is).or_not();
    // NOTE: `mapping_guidelines.md` §Conditions lists `IF IN (...)` and `IF NOT IN (...)` with no
    // operand, reading the row's own source; the guide lists only those, and reading `VALUED` and
    // `NOT VALUED` the same way is our own reading.
    let implicit = choice((
        is.clone()
            .then(just(Token::Not))
            .then(just(Token::In))
            .ignore_then(list.clone())
            .map(Check::NotIn),
        is.clone()
            .then(just(Token::In))
            .ignore_then(list)
            .map(Check::In),
        is.clone()
            .then(just(Token::Not))
            .then(just(Token::Valued))
            .to(Check::NotValued { error: false }),
        is.then(just(Token::Valued)).to(Check::Valued),
    ))
    .try_map(move |check, span| {
        source
            .clone()
            .map(|operand| Expr::Test(operand, check))
            .ok_or_else(|| Simple::new(None, span))
    });
    let expr = recursive(|expr| {
        let group = just(Token::Open)
            .ignore_then(just(Token::If).or_not())
            .ignore_then(expr)
            .then_ignore(just(Token::Close));
        let unary = choice((
            just(Token::Not)
                .ignore_then(group.clone())
                .map(|inner| Expr::Not(Box::new(inner))),
            group,
            test,
            implicit,
        ));
        let and = unary.clone().foldl(
            just(Token::And)
                .then(just(Token::If).or_not())
                .ignore_then(unary)
                .repeated(),
            |left, right| Expr::And(Box::new(left), Box::new(right)),
        );
        and.clone().foldl(
            just(Token::Or)
                .then(just(Token::If).or_not())
                .ignore_then(and)
                .repeated(),
            |left, right| Expr::Or(Box::new(left), Box::new(right)),
        )
    });
    just(Token::If).ignore_then(expr).then_ignore(end())
}

/// The check that follows an operand.
fn check<'a>() -> impl Parser<'a, &'a [Token], Check, Extra<'a>> + Clone {
    let literal = select! { Token::Text(text) => text };
    let number = select! { Token::Number(number) => number };
    let list = literal
        .separated_by(just(Token::Comma))
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Open), just(Token::Close));
    let or_equals = just(Token::Or).then(just(Token::Equals)).or_not();
    let compare = choice((
        just(Token::Equals).to(Compare::Equal),
        just(Token::EqualSign).to(Compare::Equal),
        just(Token::Not)
            .then(just(Token::Equals))
            .to(Compare::NotEqual),
        just(Token::Greater)
            .then(just(Token::Than))
            .ignore_then(or_equals.clone())
            .map(|or_equal| {
                if or_equal.is_some() {
                    Compare::GreaterOrEqual
                } else {
                    Compare::Greater
                }
            }),
        just(Token::Less)
            .then(just(Token::Than))
            .ignore_then(or_equals)
            .map(|or_equal| {
                if or_equal.is_some() {
                    Compare::LessOrEqual
                } else {
                    Compare::Less
                }
            }),
    ));
    let is = just(Token::Is).or_not();
    choice((
        just(Token::Count)
            .ignore_then(compare.clone())
            .then(number)
            .map(|(compare, number)| Check::Count(compare, number)),
        just(Token::Length)
            .ignore_then(compare)
            .then(number)
            .map(|(compare, number)| Check::Length(compare, number)),
        just(Token::Does)
            .then(just(Token::Not))
            .then(just(Token::Exist))
            .to(Check::NotValued { error: false }),
        just(Token::Is)
            .then(just(Token::Empty))
            .to(Check::NotValued { error: false }),
        is.clone().then(just(Token::Valued)).to(Check::Valued),
        is.clone()
            .then(just(Token::Not))
            .then(just(Token::Valued))
            .ignore_then(just(Token::Error).or_not())
            .map(|error| Check::NotValued {
                error: error.is_some(),
            }),
        is.clone()
            .then(just(Token::Not))
            .then(just(Token::In))
            .ignore_then(list.clone())
            .map(Check::NotIn),
        is.then(just(Token::In)).ignore_then(list).map(Check::In),
        just(Token::Not)
            .then(just(Token::Equals))
            .ignore_then(literal)
            .map(Check::NotEquals),
        just(Token::Is)
            .then(just(Token::Not))
            .ignore_then(literal)
            .map(Check::NotEquals),
        choice((just(Token::Equals), just(Token::EqualSign), just(Token::Is)))
            .ignore_then(literal)
            .map(Check::Equals),
    ))
}

/// What an operand reads in the message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Probe {
    /// Whether it holds a value.
    pub valued: bool,
    /// The first value, when it holds one.
    pub text: Option<String>,
    /// How many valued repetitions the field holds.
    pub count: u64,
}

/// Why a parsed condition could not be evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unevaluable {
    /// An operand names nothing the scope can read.
    Operand(String),
    /// `NOT VALUED ERROR` held: the mapper stops.
    Stop(String),
}

/// Evaluates `expr`, reading each operand through `probe`.
///
/// # Errors
///
/// Returns [`Unevaluable::Operand`] when `probe` cannot read an operand, and
/// [`Unevaluable::Stop`] when a `NOT VALUED ERROR` check holds.
pub fn evaluate(
    expr: &Expr,
    probe: &mut impl FnMut(&Operand) -> Option<Probe>,
) -> Result<bool, Unevaluable> {
    match expr {
        Expr::And(left, right) => Ok(evaluate(left, probe)? && evaluate(right, probe)?),
        Expr::Or(left, right) => Ok(evaluate(left, probe)? || evaluate(right, probe)?),
        Expr::Not(inner) => Ok(!evaluate(inner, probe)?),
        Expr::Test(operand, check) => {
            let read = probe(operand).ok_or_else(|| Unevaluable::Operand(operand.to_string()))?;
            let text = read.text.as_deref();
            let member =
                |items: &[Literal]| text.is_some_and(|text| items.iter().any(|item| item == text));
            Ok(match check {
                Check::Valued => read.valued,
                Check::NotValued { error } => {
                    if *error && !read.valued {
                        return Err(Unevaluable::Stop(operand.to_string()));
                    }
                    !read.valued
                }
                Check::Equals(literal) => text == Some(literal.as_str()),
                Check::NotEquals(literal) => text != Some(literal.as_str()),
                Check::In(items) => member(items),
                Check::NotIn(items) => !member(items),
                Check::Count(compare, number) => compare.holds(read.count, *number),
                Check::Length(compare, number) => {
                    let length = text.map_or(0, |text| text.chars().count());
                    compare.holds(u64::try_from(length).unwrap_or(u64::MAX), *number)
                }
            })
        }
    }
}

/// Whether `expr` holds only where `operand` is not valued: a `NOT VALUED`
/// check of `operand`, alone or as a conjunct.
///
/// This is how a row states that it maps the absence of its own source, as
/// the `segment-msh-to-messageheader` MSH-24 rows do with
/// `IF MSH-24 NOT VALUED AND MSH-3 NOT VALUED`.
#[must_use]
pub fn requires_absent(expr: &Expr, operand: &Operand) -> bool {
    match expr {
        Expr::And(left, right) => requires_absent(left, operand) || requires_absent(right, operand),
        Expr::Test(tested, Check::NotValued { .. }) => tested == operand,
        Expr::Or(..) | Expr::Not(_) | Expr::Test(..) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AssignmentError, Check, Compare, ConditionError, Expr, Operand, Part, Probe, Unevaluable,
        assignment, evaluate, parse, parse_for, requires_absent, source_operand,
    };

    fn component(datatype: &str, path: &[usize]) -> Operand {
        Operand::Component {
            datatype: String::from(datatype),
            path: path.to_vec(),
        }
    }

    fn field(segment: &str, path: &[usize]) -> Operand {
        Operand::Field {
            segment: String::from(segment),
            path: path.to_vec(),
        }
    }

    #[test]
    fn the_guideline_forms_parse() {
        for text in [
            "IF OBX-2 EQUALS \"NM\"",
            "IF PID-29 NOT VALUED",
            "IF PRT-4.1 EQUALS \"PP\" AND PRT-4.3 EQUALS \"HL70443\"",
            "IF OBX-2 IN (\"ST\", \"FT\", \"TX\")",
            "IF (OBX-5 LST.count LESS THAN OR EQUALS 1 OR OBX-2 IS \"NA\") AND OBX-29 NOT IN (\"QST\", \"SCI\")",
            "IF PID-7 LENGTH GREATER THAN 8",
            "IF XCN.19 DOES NOT EXIST AND IF XCN.20 DOES NOT EXIST",
            "IF RXO-2 IS VALUED AND (IF RXO-4.1 IS VALUED OR RXO-4.3 IS VALUED)",
            "IF NOT (PID-3 VALUED OR PID-4 VALUED)",
            "IF OBX-2=\"NM\"",
            "IF ORC VALUED",
            "If CWE.2 IS NOT VALUED",
            "IF HD-3 = \"ISO\"",
            "IF HD.2 NOT VALUED AND (HD-3 NOT IN (\"ISO\", \"UUID\", \"DNS\", \"URI\"))",
            "IF HD.1 NOT VALUED AND IF HD-3 NOT IN (\"ISO\", \"UUID\")",
        ] {
            assert!(parse(text).is_ok(), "{text}: {:?}", parse(text));
        }
    }

    #[test]
    fn corpus_forms_outside_the_list_are_refused() {
        for text in [
            "IF PV1-20 VALUE",
            "IF OBX-33 COUNT>1",
            "IF NOT VALUED",
            "IF PRT-5 AND PRT-6 AREA NOT VALUED",
            "IF (OBX-2 EQUALS \"SN\" AND OBX-5.1 EQUALS \"<>\"",
            "IF CX.4 IN http://hl7.org/implement/standards/fhir/identifier-registry.html",
            "IF IN1-17 IS 'patient'",
            "IF XYZ-3 = \"ISO\"",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_shared_check_applies_to_each_operand() {
        assert_eq!(
            parse("IF PID-33 AND PID-34 VALUED"),
            Ok(Expr::And(
                Box::new(Expr::Test(field("PID", &[33]), Check::Valued)),
                Box::new(Expr::Test(field("PID", &[34]), Check::Valued)),
            ))
        );
    }

    #[test]
    fn and_binds_tighter_than_or() {
        let parsed = parse("IF PID-3 VALUED OR PID-4 VALUED AND PID-5 VALUED");
        let test = |position| Box::new(Expr::Test(field("PID", &[position]), Check::Valued));
        assert_eq!(
            parsed,
            Ok(Expr::Or(test(3), Box::new(Expr::And(test(4), test(5)))))
        );
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn a_count_comparison_reads_the_repetitions() -> Result<(), Unevaluable> {
        let expr = parse("IF OBX-5 LST.COUNT GREATER THAN OR EQUALS 2").expect("a condition");
        assert!(
            matches!(
                expr,
                Expr::Test(_, Check::Count(Compare::GreaterOrEqual, 2))
            ),
            "{expr:?}"
        );
        let mut probe = |_: &Operand| {
            Some(Probe {
                valued: true,
                text: Some(String::from("5.4")),
                count: 2,
            })
        };
        assert!(evaluate(&expr, &mut probe)?);
        Ok(())
    }

    #[test]
    fn not_valued_error_stops_the_mapper_when_it_holds() {
        let expr = parse("IF PID-3 NOT VALUED ERROR").expect("a condition");
        let mut probe = |_: &Operand| Some(Probe::default());
        assert!(matches!(
            evaluate(&expr, &mut probe),
            Err(Unevaluable::Stop(_))
        ));
    }

    #[test]
    fn only_a_conjunct_not_valued_check_requires_the_operand_absent() {
        let own = field("MSH", &[24]);
        let holds = |text: &str| requires_absent(&parse(text).expect("it parses"), &own);
        assert!(holds("IF MSH-24 NOT VALUED AND MSH-3 NOT VALUED"));
        assert!(holds("IF MSH-3 NOT VALUED AND MSH-24 NOT VALUED"));
        assert!(!holds("IF MSH-3 NOT VALUED"));
        assert!(!holds("IF MSH-24.1 NOT VALUED"));
        assert!(!holds("IF MSH-24 NOT VALUED OR MSH-3 NOT VALUED"));
        assert!(!holds("IF MSH-24 NOT IN (\"A\")"));
    }

    #[test]
    fn a_dashed_data_type_name_is_a_component_and_a_dashed_segment_a_field() {
        assert_eq!(
            parse("IF HD-3 = \"ISO\""),
            Ok(Expr::Test(
                component("HD", &[3]),
                Check::Equals(String::from("ISO"))
            ))
        );
        assert_eq!(
            parse("IF XON-10 NOT VALUED"),
            Ok(Expr::Test(
                component("XON", &[10]),
                Check::NotValued { error: false }
            ))
        );
        assert_eq!(
            parse("IF PID-3 VALUED"),
            Ok(Expr::Test(field("PID", &[3]), Check::Valued))
        );
    }

    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "test assertions")]
    fn a_dashed_component_comparison_reads_the_component() -> Result<(), Unevaluable> {
        let expr = parse("IF HD-3 NOT IN (\"ISO\", \"UUID\")").expect("a condition");
        let probe = |text: &'static str| {
            move |operand: &Operand| {
                (operand == &component("HD", &[3])).then(|| Probe {
                    valued: true,
                    text: Some(String::from(text)),
                    count: 1,
                })
            }
        };
        assert!(!evaluate(&expr, &mut probe("UUID"))?);
        assert!(evaluate(&expr, &mut probe("DNS"))?);
        Ok(())
    }

    #[test]
    fn a_check_with_no_operand_is_refused_as_such() {
        for (text, token) in [
            ("IF NOT VALUED", 1),
            (
                "IF NOT VALUED OR NOT IN (\"ISO\", \"UUID\", \"DNS\", \"URI\")",
                1,
            ),
            ("IF NOT VALUED AND RXA-21 NOT EQUALS \"D\"", 1),
            ("IF PID-3 VALUED AND IN (\"A\")", 4),
        ] {
            assert_eq!(
                parse(text),
                Err(ConditionError::NoOperand { token }),
                "{text}"
            );
        }
        assert!(matches!(parse("IF NOT (PID-3 VALUED)"), Ok(Expr::Not(_))));
    }

    #[test]
    fn an_operand_less_in_reads_the_row_source() {
        let own = component("HD", &[3]);
        let list = || vec![String::from("ISO"), String::from("UUID")];
        assert_eq!(
            parse_for("IF IN (\"ISO\", \"UUID\")", Some(&own)),
            Ok(Expr::Test(own.clone(), Check::In(list())))
        );
        assert_eq!(
            parse_for("IF NOT IN (\"ISO\", \"UUID\")", Some(&own)),
            Ok(Expr::Test(own.clone(), Check::NotIn(list())))
        );
        assert_eq!(
            parse_for("IF HD.2 VALUED AND NOT IN (\"ISO\", \"UUID\")", Some(&own)),
            Ok(Expr::And(
                Box::new(Expr::Test(component("HD", &[2]), Check::Valued)),
                Box::new(Expr::Test(own.clone(), Check::NotIn(list())))
            ))
        );
    }

    #[test]
    fn an_operand_less_check_with_no_row_source_or_a_bare_literal_stays_refused() {
        for text in [
            "IF IN (\"ISO\")",
            "IF NOT IN (\"ISO\")",
            "IF VALUED",
            "IF NOT VALUED",
        ] {
            assert_eq!(
                parse_for(text, None),
                Err(ConditionError::NoOperand { token: 1 }),
                "{text}"
            );
        }
        assert_eq!(
            parse_for(
                "IF IAM-15 VALUED AND NOT \"SEL\"",
                Some(&field("IAM", &[15]))
            ),
            Err(ConditionError::NoOperand { token: 4 })
        );
    }

    #[test]
    fn an_operand_less_valued_reads_the_row_source() {
        let own = component("HD", &[3]);
        assert_eq!(
            parse_for("IF VALUED", Some(&own)),
            Ok(Expr::Test(own.clone(), Check::Valued))
        );
        assert_eq!(
            parse_for("IF IS VALUED", Some(&own)),
            Ok(Expr::Test(own.clone(), Check::Valued))
        );
    }

    #[test]
    fn an_operand_less_not_valued_reads_the_row_source() {
        let own = component("HD", &[3]);
        let absent = || Expr::Test(own.clone(), Check::NotValued { error: false });
        assert_eq!(parse_for("IF NOT VALUED", Some(&own)), Ok(absent()));
        assert_eq!(
            parse_for(
                "IF NOT VALUED OR NOT IN (\"ISO\", \"UUID\", \"DNS\", \"URI\")",
                Some(&own)
            ),
            Ok(Expr::Or(
                Box::new(absent()),
                Box::new(Expr::Test(
                    own.clone(),
                    Check::NotIn(["ISO", "UUID", "DNS", "URI"].map(String::from).to_vec())
                ))
            ))
        );
        assert_eq!(
            parse_for("IF NOT VALUED AND RXA-21 NOT EQUALS \"D\"", Some(&own)),
            Ok(Expr::And(
                Box::new(absent()),
                Box::new(Expr::Test(
                    field("RXA", &[21]),
                    Check::NotEquals(String::from("D"))
                ))
            ))
        );
    }

    #[test]
    fn a_source_code_names_its_operand() {
        assert_eq!(source_operand("HD.3"), Some(component("HD", &[3])));
        assert_eq!(source_operand("MSH-24"), Some(field("MSH", &[24])));
        assert_eq!(
            source_operand("PID"),
            Some(Operand::Segment(String::from("PID")))
        );
        assert_eq!(source_operand("ORU_R01.MSH"), None);
        assert_eq!(source_operand("MSG"), None);
    }

    #[test]
    fn a_concatenation_parses_into_its_parts() {
        let literal = |text: &str| Part::Literal(String::from(text));
        assert_eq!(
            assignment("\"urn:oid:\"+HD.2"),
            Ok(vec![
                literal("urn:oid:"),
                Part::Operand(component("HD", &[2]))
            ])
        );
        assert_eq!(
            assignment("HD.1+\" - \"+HD.3+\":\"+HD.2"),
            Ok(vec![
                Part::Operand(component("HD", &[1])),
                literal(" - "),
                Part::Operand(component("HD", &[3])),
                literal(":"),
                Part::Operand(component("HD", &[2])),
            ])
        );
        assert_eq!(
            assignment("OBX-5.1+\"-\"+OBX-5.2"),
            Ok(vec![
                Part::Operand(field("OBX", &[5, 1])),
                literal("-"),
                Part::Operand(field("OBX", &[5, 2])),
            ])
        );
        assert_eq!(
            assignment("NA.1 + \"^\" + NA.2"),
            Ok(vec![
                Part::Operand(component("NA", &[1])),
                literal("^"),
                Part::Operand(component("NA", &[2])),
            ])
        );
        assert_eq!(
            assignment("NA.1"),
            Ok(vec![Part::Operand(component("NA", &[1]))])
        );
    }

    #[test]
    fn a_concatenation_missing_an_operator_is_refused_as_such() {
        assert_eq!(
            assignment("RP.3\"/\"RP.4"),
            Err(AssignmentError::MissingOperator { token: 0 })
        );
        assert_eq!(
            assignment("NA.1 + \"^\" + NA.2 + \"^\" NA.4"),
            Err(AssignmentError::MissingOperator { token: 6 })
        );
    }

    #[test]
    fn an_expression_or_narrative_assignment_is_outside_the_grammar() {
        for text in [
            "/translate number to day/",
            "Appointment.participant.period.start + AIG-11",
            "#placer contact#",
            "\"http://terminology.hl7.org/CodeSystem/v2-0203\"\\\\",
            "HD.1+",
        ] {
            assert!(
                matches!(
                    assignment(text),
                    Err(AssignmentError::Lex { .. } | AssignmentError::Syntax { .. })
                ),
                "{text}: {:?}",
                assignment(text)
            );
        }
    }
}
