use crate::model::text;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Report-local references replace internal hashes. Detailed access records remain
/// available in the full report; the manifest retains every mutation and finding.
pub fn compact(report: &Value) -> Value {
    let rows = |key: &str| report[key].as_array().unwrap().as_slice();
    let resources: HashMap<_, _> = rows("resources")
        .iter()
        .map(|r| (text(&r["resourceId"]), r))
        .collect();
    let ids: HashMap<_, _> = rows("analysisInstances")
        .iter()
        .enumerate()
        .map(|(i, a)| (text(&a["analysisId"]), format!("s{}", i + 1)))
        .collect();
    let inline_urls: HashSet<_> = resources
        .values()
        .filter(|resource| resource.get("inline").is_some())
        .map(|resource| text(&resource["finalUrl"]))
        // Inline dependencies have content instead of a specifier. Include
        // pending children that never became resources because a budget stopped them.
        .chain(
            rows("references")
                .iter()
                .filter(|edge| edge.get("specifier").is_none())
                .filter_map(|edge| edge["resolvedUrl"].as_str()),
        )
        .collect();
    let mut scopes = HashMap::new();
    for a in rows("analysisInstances") {
        let next = scopes.len() + 1;
        scopes
            .entry(text(&a["context"]["scopeKey"]))
            .or_insert(next);
    }
    let mut sources = Vec::new();
    for a in rows("analysisInstances") {
        let r = resources[text(&a["resourceId"])];
        let mut source = json!({
            "id": ids[text(&a["analysisId"])],
            "kind": r["kind"],
            "status": a["scanStatus"],
        });
        // Anonymous content is not a filesystem path or a document base URL.
        if !text(&r["finalUrl"]).starts_with("memory:") {
            source["url"] = r["finalUrl"].clone();
        }
        if r.get("inline").is_some() {
            // The inline identity hash is not useful to a manifest consumer.
            source.as_object_mut().unwrap().remove("url");
            source["inline"] = r["inline"].clone();
            let parent = text(&r["inline"]["parentUrl"]);
            if let Some(edge) = rows("references").iter().find(|edge| {
                edge["toAnalysisIds"]
                    .as_array()
                    .unwrap()
                    .contains(&a["analysisId"])
                    && rows("analysisInstances").iter().any(|other| {
                        other["analysisId"] == edge["fromAnalysisId"]
                            && resources[text(&other["resourceId"])]["finalUrl"] == parent
                    })
            }) {
                source["inline"]
                    .as_object_mut()
                    .unwrap()
                    .remove("parentUrl");
                source["inline"]["parent"] = json!(ids[text(&edge["fromAnalysisId"])]);
            }
        }
        if r["kind"] == "js" {
            source["scriptMode"] = a["context"]["scriptMode"].clone();
            source["environment"] = a["context"]["executionEnvironment"].clone();
        }
        if scopes.len() > 1 {
            source["scope"] = json!(scopes[text(&a["context"]["scopeKey"])]);
        }
        if a["sourceMapStatus"] != "absent" {
            source["sourceMapStatus"] = a["sourceMapStatus"].clone();
        }
        sources.push(source);
    }
    let owner = |record: &Value, field: &str| ids.get(text(&record[field])).map(|id| json!(id));
    let dependencies: Vec<_> = rows("references")
        .iter()
        .map(|r| {
            let mut edge = json!({
                "from": owner(r, "fromAnalysisId"),
                "kind": r["kind"],
                "status": r["resolution"],
                "location": location(&r["location"]),
            });
            let to: Vec<_> = r["toAnalysisIds"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|id| ids.get(text(id)))
                .collect();
            if !to.is_empty() {
                edge["to"] = json!(to);
            }
            copy(r, &mut edge, &["specifier", "reason", "condition"]);
            if let Some(url) = r["resolvedUrl"].as_str() {
                if !url.starts_with("memory:")
                    && !inline_urls.contains(url)
                    && r["specifier"] != url
                {
                    edge["url"] = json!(url);
                }
            }
            edge
        })
        .collect();
    let targets: HashMap<_, _> = rows("globalTargets")
        .iter()
        .map(|t| (text(&t["targetId"]), t))
        .collect();
    let mut globals = Vec::new();
    let mut read_count = 0;
    for access in rows("globalAccesses") {
        if access["operation"] == "read" {
            read_count += 1;
            continue;
        }
        let names: Vec<_> = access["targetRefs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| {
                let t = targets[text(id)];
                let mut name = json!({"namespace": t["namespace"], "path": t["pathSegments"]});
                if t["keyKind"] != "string" {
                    name["keyKind"] = t["keyKind"].clone();
                }
                name
            })
            .collect();
        let mut item = json!({
            "source": owner(access, "analysisId"),
            "targets": names,
            "operation": access["operation"],
            "location": location(&access["location"]),
        });
        if access["valueType"] != "unknown" {
            item["valueType"] = access["valueType"].clone();
        }
        globals.push(item);
    }
    let findings: Vec<_> = rows("findings")
        .iter()
        // Global findings describe the same operations already present above.
        .filter(|f| {
            !(f["kind"] == "global"
                && ["global-mutation", "prototype-mutation"].contains(&text(&f["ruleId"])))
        })
        .map(|f| {
            let mut details = f["details"].clone();
            // Static-candidate certainty applies to the entire manifest.
            if details["certainty"] == "static-candidate" {
                details.as_object_mut().unwrap().remove("certainty");
            }
            let mut item = json!({
                "source": owner(f, "analysisId"),
                "rule": f["ruleId"],
                "location": location(&f["location"]),
            });
            if details.as_object().is_some_and(|d| !d.is_empty()) {
                item["details"] = details;
            }
            item
        })
        .collect();
    let mut groups: HashMap<String, usize> = HashMap::new();
    let mut unresolved: Vec<Value> = Vec::new();
    for gap in rows("unresolved") {
        let key = json!([
            gap["analysisId"],
            gap["reason"],
            gap["affectedCapabilities"],
            gap["detail"]
        ])
        .to_string();
        if let Some(index) = groups.get(&key) {
            let count = unresolved[*index]["count"].as_u64().unwrap();
            unresolved[*index]["count"] = json!(count + 1);
        } else {
            let mut item = json!({"reason": gap["reason"], "count": 1, "capabilities": gap["affectedCapabilities"]});
            if let Some(source) = owner(gap, "analysisId") {
                item["source"] = source;
            }
            copy(gap, &mut item, &["detail"]);
            if gap.get("location").is_some() {
                item["exampleLocation"] = location(&gap["location"]);
            }
            groups.insert(key, unresolved.len());
            unresolved.push(item);
        }
    }
    let mut output = json!({
        "format": "compact",
        "schemaVersion": 1,
        "scanStatus": report["scanStatus"],
        "runStatus": report["runStatus"],
        "certainty": "static-candidate",
        "summary": {
            "resources": resources.len(),
            "analyses": sources.len(),
            "dependencies": dependencies.len(),
            "globalMutations": globals.len(),
            "findings": findings.len(),
            "unresolved": rows("unresolved").len(),
            "omittedReadAccesses": read_count,
            "bytes": report["coverage"]["bytes"],
            "elapsedMs": report["coverage"]["elapsedMs"],
            "dependencyMode": report["coverage"]["dependencyMode"],
        },
        "entries": rows("entryAnalysisIds").iter().filter_map(|id| ids.get(text(id))).collect::<Vec<_>>(),
        "sources": sources,
        "dependencies": dependencies,
        "globals": globals,
        "findings": findings,
        "unresolved": unresolved,
    });
    if let Some(timings) = report.get("timings") {
        output["timings"] = timings.clone();
    }
    output
}

fn copy(from: &Value, to: &mut Value, keys: &[&str]) {
    for key in keys {
        if let Some(value) = from.get(key) {
            to[*key] = value.clone();
        }
    }
}

fn location(value: &Value) -> Value {
    let mut out = json!({"line": value["line"], "column": value["column"]});
    if value["endLine"] != value["line"] {
        out["endLine"] = value["endLine"].clone();
    }
    out["endColumn"] = value["endColumn"].clone();
    copy(value, &mut out, &["original"]);
    // Inline source records already identify their containing document.
    if let Some(container) = value.get("container") {
        out["container"] = json!({"line": container["line"], "column": container["column"]});
        copy(container, &mut out["container"], &["approximate"]);
    }
    out
}
