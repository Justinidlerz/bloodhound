use super::*;

pub(crate) const COLLECTIONS: [&str; 7] = [
    "resources",
    "analysisInstances",
    "references",
    "globalTargets",
    "globalAccesses",
    "findings",
    "unresolved",
];
const SCHEMA: &str = include_str!("../../../schemas/report.schema.json");
static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
pub fn validate(report: &Value) -> Result<(), String> {
    let v = VALIDATOR.get_or_init(|| {
        jsonschema::validator_for(&serde_json::from_str::<Value>(SCHEMA).unwrap()).unwrap()
    });
    if let Err(e) = v.validate(report) {
        return Err(format!("invalid-report: {e}"));
    }
    let ids = |key: &str, id: &str| {
        report[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| text(&v[id]).to_string())
            .collect::<HashSet<_>>()
    };
    let resources = ids("resources", "resourceId");
    let analyses = ids("analysisInstances", "analysisId");
    let targets = ids("globalTargets", "targetId");
    let accesses = ids("globalAccesses", "accessId");
    for a in report["analysisInstances"].as_array().unwrap() {
        if !resources.contains(text(&a["resourceId"])) {
            return Err("dangling-resource".into());
        }
    }
    for r in report["references"].as_array().unwrap() {
        if !analyses.contains(text(&r["fromAnalysisId"]))
            || r["toAnalysisIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| !analyses.contains(text(id)))
        {
            return Err("dangling-reference".into());
        }
    }
    for a in report["globalAccesses"].as_array().unwrap() {
        if !analyses.contains(text(&a["analysisId"]))
            || a["targetRefs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| !targets.contains(text(id)))
        {
            return Err("dangling-access".into());
        }
    }
    for f in report["findings"].as_array().unwrap() {
        if !analyses.contains(text(&f["analysisId"]))
            || f["accessRefs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| !accesses.contains(text(id)))
        {
            return Err("dangling-finding".into());
        }
    }
    Ok(())
}
pub(super) fn date() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}
pub fn write_json(path: &str, value: &Value) -> Result<(), String> {
    let target = Path::new(path);
    let temp = target.with_extension(format!("{}.tmp", now()));
    let mut file = BufWriter::new(File::create(&temp).map_err(|e| e.to_string())?);
    serde_json::to_writer_pretty(&mut file, value).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    file.get_ref().sync_all().map_err(|e| e.to_string())?;
    fs::rename(&temp, target).map_err(|e| e.to_string())
}
