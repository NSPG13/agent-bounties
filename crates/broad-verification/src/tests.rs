use super::*;
#[test]
fn each_checker_schema_can_be_used_independently() {
    fn check_references(value: &Value, root: &Value) {
        match value {
            Value::Object(fields) => {
                if let Some(reference) = fields.get("$ref").and_then(Value::as_str) {
                    let pointer = reference.strip_prefix('#').expect("local schema reference");
                    assert!(root.pointer(pointer).is_some(), "unresolved {reference}");
                }
                for child in fields.values() {
                    check_references(child, root);
                }
            }
            Value::Array(children) => {
                for child in children {
                    check_references(child, root);
                }
            }
            _ => {}
        }
    }
    for checker in catalog()["checks"].as_array().unwrap() {
        let schema = &checker["parameters_schema"];
        assert_eq!(schema["properties"]["checker"]["const"], checker["id"]);
        check_references(schema, schema);
    }
}
#[test]
fn new_contests_have_separate_stages_and_exact_funding_boundaries() {
    let mut p = plan(Rule::IntegrityV1);
    p.criteria[0].review = Some("Does the work meet the brief?".into());
    let windows = TimeWindows::recommended(&p, 100, 200).unwrap();
    assert_eq!(windows.checks_cutoff, 200 + 86_400);
    assert_eq!(windows.review_cutoff, 200 + 3 * 86_400);
    assert_eq!(windows.final_cutoff, 200 + 4 * 86_400);
    p.time_windows = Some(windows.clone());
    validate_plan(&p).unwrap();
    windows.validate_before_funding(&p, 99).unwrap();
    assert!(windows.validate_before_funding(&p, 100).is_err());
    // Self-checking after closure remains possible; it grants no late admission.
    validate_plan(&p).unwrap();
    let original_hash = value_digest(&p).unwrap();
    p.time_windows.as_mut().unwrap().submission_cutoff += 1;
    assert_ne!(original_hash, value_digest(&p).unwrap());
    assert!(validate_plan(&p).is_err());
}
#[test]
fn missing_stages_are_omitted_and_impossible_deadlines_are_rejected() {
    let mut p = plan(Rule::IntegrityV1);
    let automatic = TimeWindows::recommended(&p, 100, 200).unwrap();
    assert_eq!(automatic.review_cutoff, automatic.checks_cutoff);
    assert_eq!(automatic.final_cutoff, automatic.review_cutoff);
    automatic.validate(&p).unwrap();
    let mut extra = automatic.clone();
    extra.final_cutoff += 86_400;
    assert!(extra.validate(&p).is_err());
    p.checks.clear();
    p.criteria[0].review = Some("Review the brief".into());
    let manual = TimeWindows::recommended(&p, 100, 200).unwrap();
    assert_eq!(manual.checks_cutoff, manual.submission_cutoff);
    manual.validate(&p).unwrap();
    let mut short = manual.clone();
    short.review_cutoff -= 1;
    assert!(short.validate(&p).is_err());
    let mut reversed = manual.clone();
    reversed.checks_cutoff = 1;
    assert!(reversed.validate(&p).is_err());
    let mut overflow = manual;
    overflow.review_cutoff = u64::MAX;
    assert!(overflow.validate(&p).is_err());
    assert!(TimeWindows::recommended(&p, 200, 200).is_err());
    assert!(TimeWindows::recommended(&p, 100, u64::MAX).is_err());
}
#[test]
fn optional_deadlines_preserve_existing_plan_hashes() {
    let old = json!({"schema_version":VERSION,"criteria":[{"id":"delivery","text":"Deliver","review":null}],"checks":[]});
    let parsed: Plan = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), old);
}
fn plan(rule: Rule) -> Plan {
    Plan {
        schema_version: VERSION.into(),
        time_windows: None,
        criteria: vec![Criterion {
            id: "delivery".into(),
            text: "Deliver the agreed work".into(),
            review: None,
        }],
        checks: vec![Check {
            id: "check".into(),
            criteria: vec!["delivery".into()],
            purpose: Purpose::Required,
            rule,
        }],
    }
}
fn result(rule: Rule, files: BTreeMap<String, Vec<u8>>) -> Report {
    run(&plan(rule), &Manifest::from_files(&files).unwrap(), &files).unwrap()
}
#[test]
fn altered_or_extra_files_block_even_a_passing_rule() {
    let mut files = BTreeMap::from([("data.json".into(), b"{\"ok\":true}".to_vec())]);
    let manifest = Manifest::from_files(&files).unwrap();
    files.insert("data.json".into(), b"{\"ok\":false}".to_vec());
    let report = run(&plan(Rule::IntegrityV1), &manifest, &files).unwrap();
    assert_eq!(report.checks[0].status, Status::Blocked);
    assert!(!report.required_checks_passed);
}
#[test]
fn opaque_designs_and_documents_are_valid_deliveries_without_quality_claims() {
    let files = BTreeMap::from([
        ("design.bin".into(), vec![255, 0, 99]),
        ("report.pdf".into(), b"opaque document".to_vec()),
    ]);
    let report = result(
        Rule::ContentsV1 {
            paths: vec!["design.bin".into(), "report.pdf".into()],
            folders: vec![],
        },
        files,
    );
    assert!(report.required_checks_passed);
    assert!(!report.payment_authorized);
}
#[test]
fn reported_pass_does_not_run_published_tests() {
    let report = result(
        Rule::PublishedTestsV1 {
            image: format!("runner@sha256:{}", "a".repeat(64)),
            benchmark_sha256: value_digest(&BTreeMap::from([(
                "test.py".to_string(),
                "assert True".to_string(),
            )]))
            .unwrap(),
            benchmark_files: BTreeMap::from([("test.py".into(), "assert True".into())]),
            command: vec!["/tests/check".into()],
            timeout_seconds: 60,
            memory_bytes: 64 * 1024 * 1024,
        },
        BTreeMap::from([("result.json".into(), b"{\"passed\":true}".to_vec())]),
    );
    assert_eq!(report.checks[0].status, Status::CheckUnavailable);
    assert!(!report.required_checks_passed);
}
#[test]
fn unrelated_and_diagnostic_checks_cannot_cover_required_work() {
    let mut p = plan(Rule::IntegrityV1);
    p.checks[0].purpose = Purpose::Diagnostic;
    assert!(validate_plan(&p).is_err());
    p.criteria[0].review = Some("Judge the design against the brief".into());
    assert!(validate_plan(&p).is_ok());
    p.checks[0].criteria = vec!["other".into()];
    assert!(validate_plan(&p).is_err());
}
#[test]
fn unsafe_and_duplicate_paths_rejected() {
    for path in [
        "../secret",
        "/tmp/file",
        "a/../b",
        "a\\b",
        "a//b",
        "a/./b",
        "C:foo",
    ] {
        assert!(!safe_path(path));
    }
    let file = FileRecord {
        path: "a".into(),
        sha256: digest(b"x"),
        size: 1,
    };
    assert!(Manifest {
        schema_version: "artifact-manifest/v1".into(),
        files: vec![file.clone(), file]
    }
    .validate()
    .is_err());
}
#[test]
fn integers_are_compared_without_float_rounding() {
    let rule = Rule::JsonV1 {
        path: "data.json".into(),
        fields: vec![FieldRule {
            field: "/amount".into(),
            kind: ValueKind::Integer,
            equals: Some(json!(9007199254740993i64)),
            minimum: None,
            maximum: None,
        }],
    };
    assert!(
        result(
            rule.clone(),
            BTreeMap::from([(
                "data.json".into(),
                b"{\"amount\":9007199254740993}".to_vec()
            )])
        )
        .required_checks_passed
    );
    assert!(
        !result(
            rule,
            BTreeMap::from([(
                "data.json".into(),
                b"{\"amount\":9007199254740992}".to_vec()
            )])
        )
        .required_checks_passed
    );
}
#[test]
fn csv_checks_actual_rows_and_rejects_duplicate_headers() {
    let rule = Rule::CsvV1 {
        path: "data.csv".into(),
        fields: vec![FieldRule {
            field: "count".into(),
            kind: ValueKind::Integer,
            equals: None,
            minimum: Some(0),
            maximum: Some(5),
        }],
        min_records: 1,
        max_records: 10,
    };
    for (bytes, pass) in [
        ("id,count\na,3\n", true),
        ("id,count\na,-1\n", false),
        ("count,count\n1,2\n", false),
        ("id,count\na\n", false),
    ] {
        assert_eq!(
            result(
                rule.clone(),
                BTreeMap::from([("data.csv".into(), bytes.as_bytes().to_vec())])
            )
            .required_checks_passed,
            pass
        );
    }
}
#[test]
fn reference_checks_find_missing_identifiers_across_files() {
    let rule = Rule::ConsistencyV1 {
        left: Source {
            path: "refs.json".into(),
            format: Format::Json,
            field: "".into(),
        },
        right: Some(Source {
            path: "items.csv".into(),
            format: Format::Csv,
            field: "id".into(),
        }),
        relation: Relation::Subset,
    };
    assert!(
        result(
            rule.clone(),
            BTreeMap::from([
                ("refs.json".into(), br#"["a"]"#.to_vec()),
                ("items.csv".into(), b"id\na\nb\n".to_vec())
            ])
        )
        .required_checks_passed
    );
    assert!(
        !result(
            rule,
            BTreeMap::from([
                ("refs.json".into(), br#"["missing"]"#.to_vec()),
                ("items.csv".into(), b"id\na\n".to_vec())
            ])
        )
        .required_checks_passed
    );
}
#[test]
fn plan_digest_changes_when_rules_or_review_questions_change() {
    let mut p = plan(Rule::IntegrityV1);
    let before = value_digest(&p).unwrap();
    p.criteria[0].review = Some("Check the reasoning".into());
    assert_ne!(before, value_digest(&p).unwrap());
}

#[test]
fn empty_csv_cannot_hide_missing_required_columns() {
    let rule = Rule::CsvV1 {
        path: "data.csv".into(),
        fields: vec![FieldRule {
            field: "required".into(),
            kind: ValueKind::String,
            equals: None,
            minimum: None,
            maximum: None,
        }],
        min_records: 0,
        max_records: 10,
    };
    assert!(
        !result(
            rule,
            BTreeMap::from([("data.csv".into(), b"other\n".to_vec())])
        )
        .required_checks_passed
    );
}

#[test]
fn nested_duplicate_json_fields_are_not_valid_evidence() {
    for bytes in [
        br#"{"answer":false,"answer":true}"#.as_slice(),
        br#"{"outer":{"answer":0,"answer":1}}"#.as_slice(),
    ] {
        assert!(parse_json(bytes).is_err());
        let report = result(
            Rule::JsonV1 {
                path: "data.json".into(),
                fields: vec![],
            },
            BTreeMap::from([("data.json".into(), bytes.to_vec())]),
        );
        assert_eq!(report.checks[0].status, Status::Fail);
    }
    assert_eq!(
        parse_json(br#"{"value":9223372036854775807}"#).unwrap()["value"].as_i64(),
        Some(i64::MAX)
    );
}
#[test]
fn parent_files_cannot_shadow_subdirectories() {
    assert!(Manifest::from_files(&BTreeMap::from([
        ("a".into(), vec![]),
        ("a/b".into(), vec![])
    ]))
    .is_err());
}
