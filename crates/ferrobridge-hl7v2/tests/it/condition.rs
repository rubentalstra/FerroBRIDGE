// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The condition and assignment grammars of the v2-to-FHIR `ConceptMaps`
//! and the evaluation of a parsed condition.

use ferrobridge_hl7v2::map::condition::{
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
