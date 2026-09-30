//! Bounded, provider-free checks. Results describe evidence, never payment authority.
mod strict_json;
mod timeline;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
pub use strict_json::parse as parse_json;
pub use timeline::TimeWindows;

pub const VERSION: &str = "broad-verification/v1";
pub const MAX_UPLOAD: usize = 100 * 1024 * 1024;
pub const MAX_EXPANDED: usize = 1024 * 1024 * 1024;
pub const MAX_FILES: usize = 1024;
pub const MAX_STRUCTURED: usize = 8 * 1024 * 1024;
pub const MAX_RECORDS: usize = 100_000;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Invalid(pub String);
fn invalid(message: &str) -> Invalid {
    Invalid(message.into())
}
pub fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}
pub fn value_digest(value: &impl Serialize) -> Result<String, Invalid> {
    Ok(digest(
        &serde_json::to_vec(value).map_err(|_| invalid("invalid_json"))?,
    ))
}
pub fn valid_hash(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|h| {
        h.len() == 64
            && h.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}
pub fn safe_path(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 512
        && !s.starts_with('/')
        && !s.contains(['\\', ':', '\0'])
        && !s.chars().any(char::is_control)
        && s.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
fn safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileRecord {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: String,
    pub files: Vec<FileRecord>,
}
impl Manifest {
    pub fn from_files(files: &BTreeMap<String, Vec<u8>>) -> Result<Self, Invalid> {
        let result = Self {
            schema_version: "artifact-manifest/v1".into(),
            files: files
                .iter()
                .map(|(path, bytes)| FileRecord {
                    path: path.clone(),
                    sha256: digest(bytes),
                    size: bytes.len() as u64,
                })
                .collect(),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), Invalid> {
        if self.schema_version != "artifact-manifest/v1"
            || self.files.is_empty()
            || self.files.len() > MAX_FILES
        {
            return Err(invalid("invalid_manifest"));
        }
        let mut names = BTreeSet::new();
        let mut size = 0u64;
        for file in &self.files {
            if !safe_path(&file.path)
                || !valid_hash(&file.sha256)
                || !names.insert(file.path.as_str())
            {
                return Err(invalid("invalid_or_duplicate_file"));
            }
            size = size
                .checked_add(file.size)
                .ok_or_else(|| invalid("bundle_too_large"))?;
        }
        for path in &names {
            for (index, _) in path.match_indices('/') {
                if names.contains(&path[..index]) {
                    return Err(invalid("file_path_conflicts_with_parent_file"));
                }
            }
        }
        if size > MAX_UPLOAD as u64 {
            return Err(invalid("bundle_too_large"));
        }
        // Sorting is part of the manifest wire contract; there is one canonical digest.
        if !self.files.windows(2).all(|p| p[0].path < p[1].path) {
            return Err(invalid("manifest_not_sorted"));
        }
        Ok(())
    }
    pub fn verify(&self, files: &BTreeMap<String, Vec<u8>>) -> Result<(), Invalid> {
        self.validate()?;
        if Self::from_files(files)? != *self {
            return Err(invalid("artifact_bytes_do_not_match_manifest"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    pub text: String,
    pub review: Option<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Required,
    Diagnostic,
    Ranking,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub criteria: Vec<String>,
    pub purpose: Purpose,
    pub rule: Rule,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema_version: String,
    pub criteria: Vec<Criterion>,
    pub checks: Vec<Check>,
    /// Only new fixed-cutoff contests use these stages. Legacy clocks are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_windows: Option<TimeWindows>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "checker", rename_all = "snake_case", deny_unknown_fields)]
pub enum Rule {
    IntegrityV1,
    ContentsV1 {
        paths: Vec<String>,
        #[serde(default)]
        folders: Vec<String>,
    },
    TextV1 {
        path: String,
        #[serde(default)]
        contains: Vec<String>,
        #[serde(default)]
        excludes: Vec<String>,
    },
    JsonV1 {
        path: String,
        fields: Vec<FieldRule>,
    },
    CsvV1 {
        path: String,
        fields: Vec<FieldRule>,
        min_records: usize,
        max_records: usize,
    },
    ConsistencyV1 {
        left: Source,
        right: Option<Source>,
        relation: Relation,
    },
    PublishedTestsV1 {
        image: String,
        benchmark_sha256: String,
        benchmark_files: BTreeMap<String, String>,
        command: Vec<String>,
        timeout_seconds: u32,
        memory_bytes: u64,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldRule {
    pub field: String,
    pub kind: ValueKind,
    pub equals: Option<Value>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    String,
    Integer,
    Boolean,
    Array,
    Object,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: String,
    pub format: Format,
    pub field: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Json,
    Csv,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Equal,
    Subset,
    Unique,
    SameCount,
}

fn fields_valid(fields: &[FieldRule], csv: bool) -> bool {
    let mut seen = BTreeSet::new();
    fields.len() <= 128
        && fields.iter().all(|r| {
            r.field.len() <= 512
                && seen.insert(&r.field)
                && (if csv {
                    !r.field.is_empty()
                        && matches!(
                            r.kind,
                            ValueKind::String | ValueKind::Integer | ValueKind::Boolean
                        )
                } else {
                    r.field.is_empty() || r.field.starts_with('/')
                })
                && (r.minimum.is_none() && r.maximum.is_none() || r.kind == ValueKind::Integer)
                && r.minimum.zip(r.maximum).is_none_or(|(a, b)| a <= b)
                && r.equals.as_ref().is_none_or(|v| typed(v, r.kind))
        })
}
pub fn validate_plan(plan: &Plan) -> Result<(), Invalid> {
    if let Some(windows) = &plan.time_windows {
        windows.validate(plan)?;
    }
    if plan.schema_version != VERSION
        || plan.criteria.is_empty()
        || plan.criteria.len() > 64
        || plan.checks.len() > 128
    {
        return Err(invalid("invalid_plan_size_or_version"));
    }
    if serde_json::to_vec(plan)
        .map_err(|_| invalid("invalid_plan"))?
        .len()
        > 256 * 1024
    {
        return Err(invalid("plan_too_large"));
    }
    let mut ids = BTreeSet::new();
    for c in &plan.criteria {
        if !safe_id(&c.id)
            || !ids.insert(c.id.as_str())
            || c.text.trim().is_empty()
            || c.text.len() > 4000
            || c.review
                .as_ref()
                .is_some_and(|r| r.trim().is_empty() || r.len() > 2000)
        {
            return Err(invalid("invalid_criterion"));
        }
    }
    let total_test_seconds: u64 = plan
        .checks
        .iter()
        .map(|c| match &c.rule {
            Rule::PublishedTestsV1 {
                timeout_seconds, ..
            } => *timeout_seconds as u64,
            _ => 0,
        })
        .sum();
    if total_test_seconds > 600 {
        return Err(invalid("plan_test_time_exceeds_600_seconds"));
    }
    let mut checks = BTreeSet::new();
    let mut covered = BTreeSet::new();
    for c in &plan.checks {
        if !safe_id(&c.id)
            || !checks.insert(&c.id)
            || c.criteria.is_empty()
            || c.criteria.len() > 64
        {
            return Err(invalid("invalid_check"));
        }
        let mut refs = BTreeSet::new();
        for id in &c.criteria {
            if !ids.contains(id.as_str()) || !refs.insert(id) {
                return Err(invalid("invalid_criterion_reference"));
            }
            if c.purpose == Purpose::Required {
                covered.insert(id.as_str());
            }
        }
        let valid = match &c.rule {
            Rule::IntegrityV1 => true,
            Rule::ContentsV1 { paths, folders } => !paths.is_empty() || !folders.is_empty(),
            Rule::TextV1 {
                contains, excludes, ..
            } => !contains.is_empty() || !excludes.is_empty(),
            Rule::JsonV1 { fields, .. } => fields_valid(fields, false),
            Rule::CsvV1 {
                fields,
                min_records,
                max_records,
                ..
            } => {
                fields_valid(fields, true)
                    && min_records <= max_records
                    && *max_records <= MAX_RECORDS
            }
            Rule::ConsistencyV1 {
                left,
                right,
                relation,
            } => {
                matches!(relation, Relation::Unique) == right.is_none()
                    && valid_source(left)
                    && right.as_ref().is_none_or(valid_source)
            }
            Rule::PublishedTestsV1 {
                image,
                benchmark_sha256,
                benchmark_files,
                command,
                timeout_seconds,
                memory_bytes,
            } => {
                image.split_once("@sha256:").is_some_and(|(name, h)| {
                    !name.is_empty() && !name.starts_with('-') && valid_hash(&format!("sha256:{h}"))
                }) && valid_hash(benchmark_sha256)
                    && !benchmark_files.is_empty()
                    && benchmark_files.keys().all(|p| safe_path(p))
                    && value_digest(benchmark_files).is_ok_and(|hash| &hash == benchmark_sha256)
                    && !command.is_empty()
                    && command.len() <= 32
                    && command.iter().map(String::len).sum::<usize>() <= 16_384
                    && command.first().is_some_and(|v| {
                        !matches!(
                            v.rsplit(['/', '\\'])
                                .next()
                                .unwrap_or_default()
                                .to_ascii_lowercase()
                                .as_str(),
                            "sh" | "bash"
                                | "dash"
                                | "zsh"
                                | "cmd"
                                | "cmd.exe"
                                | "powershell"
                                | "pwsh"
                        )
                    })
                    && command.iter().all(|v| {
                        !v.is_empty() && v.len() <= 2048 && !v.contains(['\0', '\n', '\r'])
                    })
                    && (1..=600).contains(timeout_seconds)
                    && (64 * 1024 * 1024..=4 * 1024 * 1024 * 1024).contains(memory_bytes)
            }
        };
        if !valid {
            return Err(invalid("invalid_checker_parameters"));
        }
        match &c.rule {
            Rule::ContentsV1 { paths, folders } => {
                if paths.len() + folders.len() > MAX_FILES
                    || paths.iter().chain(folders).any(|p| !safe_path(p))
                {
                    return Err(invalid("invalid_required_path"));
                }
            }
            Rule::TextV1 {
                path,
                contains,
                excludes,
            } => {
                if !safe_path(path)
                    || contains.len() + excludes.len() > 128
                    || contains
                        .iter()
                        .chain(excludes)
                        .any(|s| s.is_empty() || s.len() > 4096)
                {
                    return Err(invalid("invalid_text_rule"));
                }
            }
            Rule::JsonV1 { path, .. } | Rule::CsvV1 { path, .. } if !safe_path(path) => {
                return Err(invalid("invalid_path"));
            }
            _ => (),
        }
    }
    for c in &plan.criteria {
        if c.review.is_none() && !covered.contains(c.id.as_str()) {
            return Err(invalid(
                "criterion_has_no_required_check_or_reviewer_question",
            ));
        }
    }
    Ok(())
}
fn valid_source(s: &Source) -> bool {
    safe_path(&s.path)
        && s.field.len() <= 512
        && match s.format {
            Format::Json => s.field.is_empty() || s.field.starts_with('/'),
            Format::Csv => !s.field.is_empty(),
        }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    NeedsReview,
    Unsupported,
    Blocked,
    CheckUnavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckResult {
    pub evidence: Option<Value>,
    pub id: String,
    pub status: Status,
    pub reason: String,
    pub purpose: Purpose,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    pub schema_version: String,
    pub plan_hash: String,
    pub manifest_hash: String,
    pub checks: Vec<CheckResult>,
    pub review_questions: Vec<Criterion>,
    pub required_checks_passed: bool,
    pub payment_authorized: bool,
}
type CheckOutcome = Result<(), (Status, &'static str)>;
fn failure(reason: &'static str) -> (Status, &'static str) {
    (Status::Fail, reason)
}
fn read<'a>(
    files: &'a BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Result<&'a [u8], (Status, &'static str)> {
    let bytes = files.get(path).ok_or(failure("required_file_missing"))?;
    if bytes.len() > MAX_STRUCTURED {
        return Err((
            Status::Unsupported,
            "structured_input_exceeds_supported_limit",
        ));
    }
    Ok(bytes)
}
fn typed(v: &Value, kind: ValueKind) -> bool {
    match kind {
        ValueKind::String => v.is_string(),
        ValueKind::Integer => v.as_i64().is_some(),
        ValueKind::Boolean => v.is_boolean(),
        ValueKind::Array => v.is_array(),
        ValueKind::Object => v.is_object(),
    }
}
fn matches_field(v: &Value, r: &FieldRule) -> bool {
    typed(v, r.kind)
        && r.equals.as_ref().is_none_or(|expected| expected == v)
        && r.minimum.is_none_or(|m| v.as_i64().is_some_and(|n| n >= m))
        && r.maximum.is_none_or(|m| v.as_i64().is_some_and(|n| n <= m))
}
type CsvRows = (Vec<String>, Vec<BTreeMap<String, String>>);
fn csv_rows(bytes: &[u8]) -> Result<CsvRows, (Status, &'static str)> {
    let mut reader = csv::ReaderBuilder::new().flexible(false).from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|_| failure("invalid_csv"))?
        .clone();
    if headers.is_empty()
        || headers.len() > 128
        || headers.iter().any(str::is_empty)
        || headers.iter().collect::<BTreeSet<_>>().len() != headers.len()
    {
        return Err(failure("invalid_or_duplicate_csv_headers"));
    }
    let mut rows = Vec::new();
    for row in reader.records() {
        if rows.len() >= MAX_RECORDS {
            return Err((Status::Unsupported, "record_limit_exceeded"));
        }
        let row = row.map_err(|_| failure("invalid_csv"))?;
        rows.push(
            headers
                .iter()
                .zip(row.iter())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
    }
    Ok((headers.iter().map(str::to_string).collect(), rows))
}
fn csv_value(s: &str, kind: ValueKind) -> Option<Value> {
    match kind {
        ValueKind::String => Some(Value::String(s.into())),
        ValueKind::Integer => s.parse::<i64>().ok().map(Value::from),
        ValueKind::Boolean => match s {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}
fn source_values(
    files: &BTreeMap<String, Vec<u8>>,
    source: &Source,
) -> Result<Vec<Value>, (Status, &'static str)> {
    let bytes = read(files, &source.path)?;
    match source.format {
        Format::Csv => {
            let (headers, rows) = csv_rows(bytes)?;
            if !headers.contains(&source.field) {
                return Err(failure("missing_csv_column"));
            }
            rows.into_iter()
                .map(|row| {
                    row.get(&source.field)
                        .cloned()
                        .map(Value::String)
                        .ok_or(failure("missing_csv_column"))
                })
                .collect()
        }
        Format::Json => {
            let data: Value =
                parse_json(bytes).map_err(|_| failure("invalid_or_ambiguous_json"))?;
            let v = data
                .pointer(&source.field)
                .ok_or(failure("missing_json_field"))?;
            Ok(match v {
                Value::Array(values) if values.len() <= MAX_RECORDS => values.clone(),
                Value::Array(_) => return Err((Status::Unsupported, "record_limit_exceeded")),
                _ => vec![v.clone()],
            })
        }
    }
}
fn evaluate(rule: &Rule, manifest: &Manifest, files: &BTreeMap<String, Vec<u8>>) -> CheckOutcome {
    match rule {
        Rule::IntegrityV1 => manifest
            .verify(files)
            .map_err(|_| failure("artifact_integrity_mismatch")),
        Rule::ContentsV1 { paths, folders } => {
            if paths.iter().all(|p| files.contains_key(p))
                && folders
                    .iter()
                    .all(|f| files.keys().any(|p| p.starts_with(&format!("{f}/"))))
            {
                Ok(())
            } else {
                Err(failure("required_contents_missing"))
            }
        }
        Rule::TextV1 {
            path,
            contains,
            excludes,
        } => {
            let text =
                std::str::from_utf8(read(files, path)?).map_err(|_| failure("invalid_utf8"))?;
            if contains.iter().all(|s| text.contains(s))
                && excludes.iter().all(|s| !text.contains(s))
            {
                Ok(())
            } else {
                Err(failure("text_condition_not_met"))
            }
        }
        Rule::JsonV1 { path, fields } => {
            let data: Value =
                parse_json(read(files, path)?).map_err(|_| failure("invalid_or_ambiguous_json"))?;
            if fields
                .iter()
                .all(|r| data.pointer(&r.field).is_some_and(|v| matches_field(v, r)))
            {
                Ok(())
            } else {
                Err(failure("json_condition_not_met"))
            }
        }
        Rule::CsvV1 {
            path,
            fields,
            min_records,
            max_records,
        } => {
            let (headers, rows) = csv_rows(read(files, path)?)?;
            if fields.iter().any(|r| !headers.contains(&r.field)) {
                return Err(failure("missing_csv_column"));
            }
            if rows.len() < *min_records || rows.len() > *max_records {
                return Err(failure("csv_record_count_not_met"));
            }
            if rows.iter().all(|row| {
                fields.iter().all(|r| {
                    row.get(&r.field)
                        .and_then(|s| csv_value(s, r.kind))
                        .is_some_and(|v| matches_field(&v, r))
                })
            }) {
                Ok(())
            } else {
                Err(failure("csv_condition_not_met"))
            }
        }
        Rule::ConsistencyV1 {
            left,
            right,
            relation,
        } => {
            let a = source_values(files, left)?;
            let b = right
                .as_ref()
                .map(|s| source_values(files, s))
                .transpose()?
                .unwrap_or_default();
            let keys = |v: &[Value]| {
                v.iter()
                    .map(|x| serde_json::to_string(x).expect("JSON value"))
                    .collect::<BTreeSet<_>>()
            };
            let pass = match relation {
                Relation::Equal => a == b,
                Relation::SameCount => a.len() == b.len(),
                Relation::Subset => keys(&a).is_subset(&keys(&b)),
                Relation::Unique => keys(&a).len() == a.len(),
            };
            if pass {
                Ok(())
            } else {
                Err(failure("consistency_condition_not_met"))
            }
        }
        Rule::PublishedTestsV1 { .. } => Err((
            Status::CheckUnavailable,
            "requires_precommitted_bounded_runner",
        )),
    }
}
/// Never accepts supplied test outcomes. External runs must be verified separately.
pub fn run(
    plan: &Plan,
    manifest: &Manifest,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Report, Invalid> {
    validate_plan(plan)?;
    manifest.validate()?;
    let integrity = manifest.verify(files).is_ok();
    let checks: Vec<_> = plan
        .checks
        .iter()
        .map(|c| {
            let result = if integrity {
                evaluate(&c.rule, manifest, files)
            } else {
                Err((Status::Blocked, "artifact_integrity_mismatch"))
            };
            let (status, reason) = match result {
                Ok(()) => (Status::Pass, "stated_condition_passed"),
                Err(e) => e,
            };
            CheckResult {
                evidence: None,
                id: c.id.clone(),
                status,
                reason: reason.into(),
                purpose: c.purpose,
            }
        })
        .collect();
    Ok(Report {
        schema_version: VERSION.into(),
        plan_hash: value_digest(plan)?,
        manifest_hash: value_digest(manifest)?,
        required_checks_passed: integrity
            && checks
                .iter()
                .filter(|c| c.purpose == Purpose::Required)
                .all(|c| c.status == Status::Pass),
        checks,
        review_questions: plan
            .criteria
            .iter()
            .filter(|c| c.review.is_some())
            .cloned()
            .collect(),
        payment_authorized: false,
    })
}

pub fn catalog() -> Value {
    let schema = serde_json::to_value(schemars::schema_for!(Plan)).expect("schema");
    let mut catalog = json!({"schema_version":VERSION,"payment_authorized":false,"limits":{"upload_bytes":MAX_UPLOAD,"expanded_bytes":MAX_EXPANDED,"files":MAX_FILES,"structured_bytes":MAX_STRUCTURED,"records":MAX_RECORDS},"checks":[
        {"id":"integrity_v1","family":"delivery_and_integrity","accepts":["any bytes"],"claim":"Stored bytes match the exact file manifest","excludes":["authorship","quality","content truth"]},
        {"id":"contents_v1","family":"required_contents","accepts":["file manifest"],"parameters":["paths","folders"],"claim":"Named files and nonempty folders exist","excludes":["correctness of their contents"]},
        {"id":"text_v1","family":"structured_rules","accepts":["UTF-8"],"parameters":["path","contains","excludes"],"claim":"Exact required text is present and prohibited text absent","excludes":["meaning","factual accuracy"]},
        {"id":"json_v1","family":"structured_rules","accepts":["JSON"],"parameters":["path","fields: JSON pointer, type, equals, integer bounds"],"claim":"Declared JSON fields satisfy the specified rules","excludes":["truth of self-reported results"]},
        {"id":"csv_v1","family":"structured_rules","accepts":["UTF-8 CSV with unique headers"],"parameters":["path","fields: column, type, equals, integer bounds","min_records","max_records"],"claim":"Records have the declared shape, values and count","excludes":["provenance","real-world accuracy"]},
        {"id":"consistency_v1","family":"consistency","accepts":["JSON","CSV"],"parameters":["left","right","relation: equal, subset, unique, same_count"],"claim":"Selected values satisfy the stated relationship","excludes":["semantic equivalence","truth"]},
        {"id":"published_tests_v1","family":"published_tests","accepts":["pinned source and benchmark"],"parameters":["image digest","benchmark_sha256","benchmark_files: exact UTF-8 test sources","command","timeout_seconds","memory_bytes"],"claim":"The exact published tests ran on the exact inputs","excludes":["test completeness","platform approval of arbitrary tests"],"requires":"bounded runner"}
    ],"assurance":"local or trusted-runner computation; not a cryptographic computation proof","payment_approval":"advisory for existing contracts; no new settlement profile activated","workflow":["find applicable checks","map every criterion","resolve missing parameters","accept plan before funding","run exact committed checks","review remaining questions"],"interpretation_notice":"Structural coverage does not prove the requirements were interpreted correctly."});
    for entry in catalog["checks"].as_array_mut().expect("static catalog") {
        entry["parameters_schema"] = schema["$defs"]["Rule"]["oneOf"]
            .as_array()
            .and_then(|variants| {
                variants
                    .iter()
                    .find(|v| v["properties"]["checker"]["const"] == entry["id"])
            })
            .cloned()
            .unwrap_or(Value::Null);
        // An agent may validate one checker without retaining the whole catalog.
        // Local references must therefore resolve within this schema document.
        entry["parameters_schema"]["$defs"] = schema["$defs"].clone();
        entry["parameters_schema"]["$schema"] = schema["$schema"].clone();
        entry["version"] = json!(1);
        entry["payment_approval_status"] = json!("advisory_only");
        entry["assurance"] = json!("trusted_runner_not_cryptographic_computation_proof");
        entry["execution_limits"] = json!({"input_bytes":MAX_UPLOAD,"structured_bytes":MAX_STRUCTURED,"records":MAX_RECORDS,"plan_bytes":262144,"test_seconds":600,"test_memory_bytes":4294967296u64});
    }
    catalog["plan_schema"] = schema;
    catalog["new_contest_windows"] = json!({"units":"unix_seconds","checks_minimum_hours":24,"review_minimum_hours":48,"recovery_hours":24,"unused_stages":"equal_cutoffs","legacy_clocks":"unchanged","capacity_reserved":false,"funding_enabled":false});
    catalog
}

#[cfg(test)]
mod tests;
