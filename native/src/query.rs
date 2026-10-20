use crate::{
    engine,
    model::{hash, text},
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
pub struct Index {
    pub report: Value,
    by_target: HashMap<String, Vec<usize>>,
    by_analysis: HashMap<String, Vec<usize>>,
    reverse: HashMap<String, Vec<usize>>,
}
impl Index {
    pub fn new(report: Value) -> Result<Self, String> {
        engine::validate(&report)?;
        let mut index = Self {
            report,
            by_target: HashMap::new(),
            by_analysis: HashMap::new(),
            reverse: HashMap::new(),
        };
        for (i, a) in index.report["globalAccesses"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            for target in a["targetRefs"].as_array().unwrap() {
                index
                    .by_target
                    .entry(text(target).into())
                    .or_default()
                    .push(i);
            }
            index
                .by_analysis
                .entry(text(&a["analysisId"]).into())
                .or_default()
                .push(i);
        }
        for (i, r) in index.report["references"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            for target in r["toAnalysisIds"].as_array().unwrap() {
                index
                    .reverse
                    .entry(text(target).into())
                    .or_default()
                    .push(i);
            }
        }
        Ok(index)
    }
    fn access(&self, index: usize) -> &Value {
        &self.report["globalAccesses"][index]
    }
    fn sources(&self, id: &str) -> Value {
        json!(
            self.by_target
                .get(id)
                .into_iter()
                .flatten()
                .map(|i| {
                    let access = self.access(*i);
                    let analysis = self.report["analysisInstances"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|a| a["analysisId"] == access["analysisId"])
                        .unwrap();
                    let resource = self.report["resources"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|r| r["resourceId"] == analysis["resourceId"])
                        .unwrap();
                    json!({"access":access,"analysis":analysis,"resource":resource})
                })
                .collect::<Vec<_>>()
        )
    }
    pub fn invoke(&self, operation: &str, args: &Value) -> Result<Value, String> {
        let id = text(&args["id"]);
        Ok(match operation{
        "findTargets"=>json!(self.report["globalTargets"].as_array().unwrap().iter().filter(|t|t["pathSegments"]==args["pathSegments"]&&args.get("namespace").is_none_or(|v|v==&t["namespace"])&&args.get("scopeKey").is_none_or(|v|v==&t["scopeKey"])).collect::<Vec<_>>()),
        "sources"=>self.sources(id),
        "accesses"=>json!(self.by_analysis.get(id).into_iter().flatten().map(|i|self.access(*i)).collect::<Vec<_>>()),
        "resourceAccesses"=>json!(self.report["analysisInstances"].as_array().unwrap().iter().filter(|a|a["resourceId"]==id).flat_map(|a|self.by_analysis.get(text(&a["analysisId"])).into_iter().flatten().map(|i|self.access(*i))).collect::<Vec<_>>()),
        "conflicts"=>json!(self.by_target.iter().filter_map(|(id,indices)|{let writes:Vec<_>=indices.iter().map(|i|self.access(*i)).filter(|a|["write","declare","define","delete"].contains(&text(&a["operation"]))).collect();if writes.len()<2{return None;}let analyses:HashSet<_>=writes.iter().map(|a|a["analysisId"].clone()).collect();let types:HashSet<_>=writes.iter().map(|a|a["valueType"].clone()).collect();Some(json!({"targetId":id,"accessIds":writes.iter().map(|a|&a["accessId"]).collect::<Vec<_>>(),"analysisIds":analyses,"valueTypes":types,"certainty":"static-candidate"}))}).collect::<Vec<_>>()),
        "resolveUnbound"=>json!(self.report["globalTargets"].as_array().unwrap().iter().filter(|t|t["namespace"]=="unbound-reference").map(|t|json!({"targetId":t["targetId"],"candidates":self.report["globalTargets"].as_array().unwrap().iter().filter(|other|other["scopeKey"]==t["scopeKey"]&&other["pathSegments"]==t["pathSegments"]&&["global-object","global-lexical"].contains(&text(&other["namespace"]))).map(|t|&t["targetId"]).collect::<Vec<_>>(),"certainty":"static-candidate"})).collect::<Vec<_>>()),
        "entryPaths"=>self.paths(id,args),
        "indexArtifact"=>{let mut v=self.report.clone();v.as_object_mut().unwrap().retain(|k,_|["entryAnalysisIds","resources","analysisInstances","references","globalTargets","globalAccesses","findings","unresolved","scopeKey","indexSchemaVersion"].contains(&k.as_str()));v["reportId"]=args.get("reportId").cloned().unwrap_or(self.report["scanId"].clone());let schema:Value=serde_json::from_str(include_str!("../../schemas/index.schema.json")).unwrap();jsonschema::validator_for(&schema).map_err(|e|e.to_string())?.validate(&v).map_err(|e|e.to_string())?;v},
        "adaptReport"=>{let kind=text(&args["type"]);if !["build","page"].contains(&kind){return Err("invalid-report-type".into());}json!({"schemaVersion":1,"reportType":kind,"reportId":self.report["scanId"],"scanStatus":self.report["scanStatus"],"runStatus":self.report["runStatus"],"capturedAt":self.report["finishedAt"],"scope":args.get("metadata").unwrap_or(&json!({})),"versions":self.report["configuration"]["context"]["versions"],"findings":self.report["findings"],"references":self.report["references"],"unresolved":self.report["unresolved"],"coverage":self.report["coverage"],"resourceResults":self.report["analysisInstances"].as_array().unwrap().iter().map(|a|json!({"analysisId":a["analysisId"],"scanStatus":a["scanStatus"],"resource":self.report["resources"].as_array().unwrap().iter().find(|r|r["resourceId"]==a["resourceId"]),"findings":self.report["findings"].as_array().unwrap().iter().filter(|f|f["analysisId"]==a["analysisId"]).collect::<Vec<_>>(),"unresolved":self.report["unresolved"].as_array().unwrap().iter().filter(|u|u["analysisId"]==a["analysisId"]).collect::<Vec<_>>()})).collect::<Vec<_>>(),"index":{"indexSchemaVersion":1,"reportId":self.report["scanId"],"pointer":"/globalTargets"}})},
        "diffReports"=>{engine::validate(&args["after"])?;diff(&self.report,&args["after"])},
        "runtimeCandidates"=>json!(self.report["globalAccesses"].as_array().unwrap().iter().filter(|access|{let analysis=self.report["analysisInstances"].as_array().unwrap().iter().find(|a|a["analysisId"]==access["analysisId"]).unwrap();let resource=self.report["resources"].as_array().unwrap().iter().find(|r|r["resourceId"]==analysis["resourceId"]).unwrap();args.get("resourceUrl").is_none_or(|v|v==&resource["finalUrl"])&&args.get("contentHash").is_none_or(|v|v==&resource["contentHash"])&&args.get("line").is_none_or(|v|v==&access["location"]["line"])&&args.get("column").is_none_or(|v|v==&access["location"]["column"])&&args.get("pathSegments").is_none_or(|path|access["targetRefs"].as_array().unwrap().iter().any(|id|self.report["globalTargets"].as_array().unwrap().iter().any(|t|&t["targetId"]==id&&&t["pathSegments"]==path)))}).map(|a|json!({"accessId":a["accessId"],"attribution":"static_candidate"})).collect::<Vec<_>>()),
        _=>return Err("unknown-index-operation".into())
    })
    }
    fn paths(&self, id: &str, args: &Value) -> Value {
        let limit = args["limit"].as_u64().unwrap_or(100) as usize;
        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        let max = args["maxVisits"].as_u64().unwrap_or(10_000) as usize;
        let starts: HashSet<_> = self
            .by_target
            .get(id)
            .into_iter()
            .flatten()
            .map(|i| text(&self.access(*i)["analysisId"]).to_string())
            .collect();
        let mut stack: Vec<_> = starts
            .into_iter()
            .map(|id| {
                (
                    id,
                    Vec::<String>::new(),
                    Vec::<String>::new(),
                    HashSet::<String>::new(),
                )
            })
            .collect();
        let mut paths = Vec::new();
        let mut visits = 0;
        let mut skipped = 0;
        while let Some((id, mut analyses, refs, mut seen)) = stack.pop() {
            if visits >= max || paths.len() >= limit {
                stack.push((id, analyses, refs, seen));
                break;
            }
            visits += 1;
            if !seen.insert(id.clone()) {
                continue;
            }
            analyses.insert(0, id.clone());
            if self.report["entryAnalysisIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x == &id)
            {
                if skipped >= offset {
                    paths.push(
                        json!({"entryAnalysisId":id,"analysisIds":analyses,"referenceIds":refs}),
                    );
                }
                skipped += 1;
                continue;
            }
            for index in self.reverse.get(&id).into_iter().flatten() {
                let r = &self.report["references"][*index];
                let mut next = refs.clone();
                next.insert(0, text(&r["referenceId"]).into());
                stack.push((
                    text(&r["fromAnalysisId"]).into(),
                    analyses.clone(),
                    next,
                    seen.clone(),
                ));
            }
        }
        json!({"truncated":!stack.is_empty(),"nextOffset":offset+paths.len(),"paths":paths})
    }
}
fn diff(before: &Value, after: &Value) -> Value {
    fn groups(report: &Value) -> HashMap<String, Vec<Value>> {
        let mut map: HashMap<String, Vec<Value>> = HashMap::new();
        for a in report["globalAccesses"].as_array().into_iter().flatten() {
            let Some(instance) = report["analysisInstances"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["analysisId"] == a["analysisId"])
            else {
                continue;
            };
            let Some(resource) = report["resources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["resourceId"] == instance["resourceId"])
            else {
                continue;
            };
            let targets: Vec<_> = a["targetRefs"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|id| {
                    report["globalTargets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|t| &t["targetId"] == id)
                })
                .map(|t| {
                    json!([
                        t["scopeKey"],
                        t["namespace"],
                        t["pathSegments"],
                        t["keyKind"]
                    ])
                })
                .collect();
            let key = hash(&json!([
                resource["finalUrl"],
                instance["context"]["scriptMode"],
                instance["context"]["executionEnvironment"],
                targets,
                a["operation"],
                a["evidence"]
            ]));
            map.entry(key).or_default().push(a.clone());
        }
        map
    }
    let a = groups(before);
    let b = groups(after);
    let keys: HashSet<_> = a.keys().chain(b.keys()).collect();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for key in keys {
        let left = a.get(key).cloned().unwrap_or_default();
        let right = b.get(key).cloned().unwrap_or_default();
        if left.len() == 1 && right.len() == 1 {
            if left[0]["fingerprint"] != right[0]["fingerprint"] {
                changed.push(json!({"before":left[0],"after":right[0]}));
            }
        } else {
            removed.extend(left);
            added.extend(right);
        }
    }
    json!({"added":added,"removed":removed,"changed":changed})
}
