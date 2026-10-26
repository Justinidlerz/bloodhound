//! Manifest construction.
//!
//! Turns the coordinator's resource graph into the `Manifest` consumed by
//! platforms and the bundler plugin. This is where byte spans become
//! line/column locations, where inline resources get positions inside their
//! parent document, where entry chains are computed, and where gaps are
//! grouped and counted.

use crate::lines::LineIndex;
use crate::model::*;
use crate::resolve;
use crate::scheduler::Graph;
use std::collections::{BTreeMap, VecDeque};

/// Maximum characters in a finding snippet.
const SNIPPET_MAX: usize = 160;

/// Builds the manifest for a finished scan.
pub fn build(graph: Graph, dependency_mode: DependencyMode) -> Manifest {
    let ids: Vec<String> = (0..graph.resources.len())
        .map(|i| format!("s{}", i + 1))
        .collect();
    let chains = chains(&graph);
    let mut sources = Vec::with_capacity(graph.resources.len());
    let mut findings = Vec::new();
    let mut dependencies = Vec::new();
    let mut gap_groups: BTreeMap<(Option<usize>, &str, Option<String>), (usize, Option<Location>)> =
        BTreeMap::new();

    // Line indexes are built lazily per resource and reused for every span.
    let indexes: Vec<Option<LineIndex<'_>>> = graph
        .resources
        .iter()
        .map(|r| r.loaded.as_ref().map(|l| LineIndex::new(&l.content)))
        .collect();

    for (i, r) in graph.resources.iter().enumerate() {
        let id = &ids[i];
        let index = indexes[i].as_ref();
        let locate = |span: Span| -> Location {
            let mut loc = index.map(|ix| ix.location(span)).unwrap_or_default();
            if let (Some(map), Some(ix)) = (&r.source_map, index) {
                let (line, column) = ix.position(span.start);
                if let Some(token) = map.lookup_token(line - 1, column) {
                    if let Some(source) = token.get_source() {
                        loc.original = Some(OriginalLocation {
                            source: source.to_string(),
                            line: token.get_src_line() + 1,
                            column: token.get_src_col(),
                            name: token.get_name().map(String::from),
                        });
                    }
                }
            }
            loc
        };

        // ---- source record
        let inline = r.inline.map(|(parent, offset)| {
            let (line, column) = indexes[parent]
                .as_ref()
                .map(|ix| ix.position(offset))
                .unwrap_or((0, 0));
            InlineOrigin {
                parent: ids[parent].clone(),
                line,
                column,
            }
        });
        let status = if r.loaded.is_none() {
            SourceStatus::Failed
        } else {
            r.status
        };
        sources.push(ManifestSource {
            id: id.clone(),
            kind: r.key_kind,
            url: if r.inline.is_some() || r.url.starts_with("memory:") {
                None
            } else {
                Some(resolve::sanitize(&r.url))
            },
            inline,
            script_mode: r.mode,
            status,
            bytes: r.loaded.as_ref().map(|l| l.bytes).unwrap_or(0),
            content_hash: r
                .loaded
                .as_ref()
                .map(|l| content_hash(&l.content))
                .unwrap_or_default(),
            referrers: r.referrers.iter().map(|&p| ids[p].clone()).collect(),
            chain: chains[i].iter().map(|&p| ids[p].clone()).collect(),
        });

        // ---- load failure becomes a gap on the resource
        if let Some(reason) = &r.failure {
            let key = (Some(i), "load-failed", Some(reason.clone()));
            gap_groups.entry(key).or_insert((0, None)).0 += 1;
        }

        let Some(analysis) = &r.analysis else {
            continue;
        };

        // ---- findings
        for f in &analysis.findings {
            findings.push(ManifestFinding {
                source: id.clone(),
                rule: f.rule,
                severity: f.rule.severity(),
                target: f.target.clone(),
                operation: f.operation.clone(),
                confidence: f.confidence,
                location: locate(f.span),
                snippet: index
                    .map(|ix| ix.snippet(f.span, SNIPPET_MAX))
                    .unwrap_or_default(),
                detail: f.detail.clone(),
            });
        }

        // ---- dependency edges
        for edge in &r.edges {
            let dep = &analysis.dependencies[edge.dependency];
            let to = edge.to.map(|t| ids[t].clone());
            // A child that was never loaded (budget stop) is unresolved even
            // though the edge resolved.
            let (status, reason) = match edge.to {
                Some(t) if graph.resources[t].status == SourceStatus::Failed => (
                    DependencyStatus::Unresolved,
                    Some(graph.resources[t].failure.clone().unwrap_or_else(|| {
                        if graph.resources[t].loaded.is_some() {
                            "parse-error".into()
                        } else {
                            "not-loaded".into()
                        }
                    })),
                ),
                _ => (edge.status, edge.reason.clone()),
            };
            dependencies.push(ManifestDependency {
                from: id.clone(),
                to,
                kind: dep.kind,
                specifier: dep.specifiers.first().map(|s| resolve::sanitize(s)),
                url: edge.url.as_deref().map(resolve::sanitize),
                status,
                reason,
                location: locate(dep.span),
            });
        }

        // ---- gaps, grouped per (resource, reason, detail)
        for g in &analysis.gaps {
            let key = (Some(i), g.reason.as_str(), g.detail.clone());
            let entry = gap_groups.entry(key).or_insert((0, None));
            entry.0 += 1;
            if entry.1.is_none() {
                entry.1 = Some(locate(g.span));
            }
        }
    }

    for (owner, g) in &graph.gaps {
        let key = (*owner, g.reason.as_str(), g.detail.clone());
        gap_groups.entry(key).or_insert((0, None)).0 += 1;
    }

    let gaps: Vec<ManifestGap> = gap_groups
        .into_iter()
        .map(|((owner, reason, detail), (count, location))| ManifestGap {
            source: owner.map(|o| ids[o].clone()),
            reason: reason.to_string(),
            count,
            detail,
            location,
        })
        .collect();

    let analyzed = graph
        .resources
        .iter()
        .filter(|r| r.status != SourceStatus::Failed)
        .count();
    let unresolved = dependencies
        .iter()
        .filter(|d| d.status == DependencyStatus::Unresolved)
        .count();
    let status = if graph.cancelled {
        ScanStatus::Cancelled
    } else if analyzed == 0 {
        ScanStatus::Failed
    } else if unresolved > 0
        || !gaps.is_empty()
        || graph
            .resources
            .iter()
            .any(|r| r.status != SourceStatus::Completed)
    {
        ScanStatus::Partial
    } else {
        ScanStatus::Completed
    };

    let summary = Summary {
        sources: sources.len(),
        findings: findings.len(),
        errors: findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count(),
        warnings: findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count(),
        dependencies: dependencies.len(),
        unresolved,
        gaps: gaps.iter().map(|g| g.count).sum(),
        bytes: graph.bytes,
        elapsed_ms: graph.elapsed_ms,
    };

    Manifest {
        schema_version: 2,
        scanner_version: env!("CARGO_PKG_VERSION").to_string(),
        status,
        dependency_mode,
        summary,
        entries: graph.entries.iter().map(|&e| ids[e].clone()).collect(),
        sources,
        findings,
        dependencies,
        gaps,
    }
}

/// Shortest referrer chain from an entry to every resource (BFS over
/// referrer edges from the entries).
fn chains(graph: &Graph) -> Vec<Vec<usize>> {
    let n = graph.resources.len();
    let mut children: Vec<Vec<usize>> = vec![vec![]; n];
    for (i, r) in graph.resources.iter().enumerate() {
        for &p in &r.referrers {
            children[p].push(i);
        }
    }
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut visited = vec![false; n];
    let mut queue = VecDeque::new();
    for &e in &graph.entries {
        visited[e] = true;
        queue.push_back(e);
    }
    while let Some(i) = queue.pop_front() {
        for &c in &children[i] {
            if !visited[c] {
                visited[c] = true;
                parent[c] = Some(i);
                queue.push_back(c);
            }
        }
    }
    (0..n)
        .map(|i| {
            let mut chain = vec![i];
            let mut cur = i;
            let mut guard = 0;
            while let Some(p) = parent[cur] {
                chain.push(p);
                cur = p;
                guard += 1;
                if guard > n {
                    break;
                }
            }
            chain.reverse();
            chain
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/report.rs"]
mod tests;
