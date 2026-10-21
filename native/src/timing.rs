use crate::{model::Work, resolve};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct Timings(Option<Arc<Mutex<State>>>);

struct State {
    started: Instant,
    stages: BTreeMap<&'static str, Duration>,
    stack: Vec<Duration>,
    resources: Vec<Resource>,
    resource_ids: HashMap<(String, String), usize>,
}

struct Resource {
    description: Value,
    stages: BTreeMap<&'static str, Duration>,
    tasks: BTreeMap<&'static str, (usize, Duration)>,
}

#[derive(Clone, Default)]
pub(crate) struct ResourceTimer {
    timings: Timings,
    index: Option<usize>,
}

pub(crate) struct Span {
    state: Option<Arc<Mutex<State>>>,
    started: Option<Instant>,
    name: &'static str,
    resource: Option<usize>,
    task: bool,
}

impl Timings {
    pub(crate) fn new(enabled: bool, started: Instant) -> Self {
        Self(enabled.then(|| {
            Arc::new(Mutex::new(State {
                started,
                stages: BTreeMap::new(),
                stack: vec![],
                resources: vec![],
                resource_ids: HashMap::new(),
            }))
        }))
    }

    // Stage spans run on the coordinator thread. Child stages are subtracted
    // from their parent, so stage totals never double-count nested work.
    pub(crate) fn stage(&self, name: &'static str) -> Span {
        self.span(name, None, false)
    }

    pub(crate) fn setup(&self) -> Span {
        let mut span = self.stage("setup");
        if let Some(state) = &self.0 {
            span.started = Some(state.lock().unwrap().started);
        }
        span
    }

    fn span(&self, name: &'static str, resource: Option<usize>, task: bool) -> Span {
        let started = self.0.as_ref().map(|state| {
            let start = Instant::now();
            if !task {
                state.lock().unwrap().stack.push(Duration::ZERO);
            }
            start
        });
        Span {
            state: self.0.clone(),
            started,
            name,
            resource,
            task,
        }
    }

    pub(crate) fn resource(&self, work: &Work) -> ResourceTimer {
        let index = self.0.as_ref().map(|state| {
            let mut state = state.lock().unwrap();
            let key = (work.resource_url.clone(), work.kind.clone());
            if let Some(index) = state.resource_ids.get(&key) {
                return *index;
            }
            let mut description = json!({"kind": work.kind});
            if let Some(inline) = &work.inline {
                let mut parent = json!({"offset": inline.offset});
                if !inline.parent_url.starts_with("memory:") {
                    parent["parentUrl"] = json!(resolve::sanitize_url(&inline.parent_url));
                }
                description["inline"] = parent;
            } else if !work.resource_url.starts_with("memory:") {
                description["url"] = json!(resolve::sanitize_url(&work.resource_url));
            }
            let index = state.resources.len();
            state.resources.push(Resource {
                description,
                stages: BTreeMap::new(),
                tasks: BTreeMap::new(),
            });
            state.resource_ids.insert(key, index);
            index
        });
        ResourceTimer {
            timings: self.clone(),
            index,
        }
    }

    pub(crate) fn finish(&self) -> Option<Value> {
        self.0.as_ref().map(|state| {
            let state = state.lock().unwrap();
            debug_assert!(state.stack.is_empty());
            let total = state.started.elapsed();
            let accounted: Duration = state.stages.values().copied().sum();
            let mut stages = durations(&state.stages);
            stages["other"] = json!(millis(total.saturating_sub(accounted)));
            let resources: Vec<_> = state
                .resources
                .iter()
                .map(|resource| {
                    let mut row = resource.description.clone();
                    row["stages"] = durations(&resource.stages);
                    row["tasks"] = json!(
                        resource
                            .tasks
                            .iter()
                            .map(|(name, (count, elapsed))| {
                                (
                                    *name,
                                    json!({"count": count, "elapsedMs": millis(*elapsed)}),
                                )
                            })
                            .collect::<BTreeMap<_, _>>()
                    );
                    row
                })
                .collect();
            json!({"totalMs": millis(total), "stages": stages, "resources": resources})
        })
    }
}

impl ResourceTimer {
    pub(crate) fn stage(&self, name: &'static str) -> Span {
        self.timings.span(name, self.index, false)
    }

    // Task spans may overlap across worker threads or nest (JS parsing is
    // included in analysis). These durations are diagnostic, not wall totals.
    pub(crate) fn task(&self, name: &'static str) -> Span {
        self.timings.span(name, self.index, true)
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let (Some(state), Some(started)) = (&self.state, self.started) else {
            return;
        };
        let elapsed = started.elapsed();
        let mut state = state.lock().unwrap();
        if self.task {
            if let Some(index) = self.resource {
                let entry = state.resources[index].tasks.entry(self.name).or_default();
                entry.0 += 1;
                entry.1 += elapsed;
            }
        } else {
            let children = state.stack.pop().expect("timing stage stack");
            if let Some(parent) = state.stack.last_mut() {
                *parent += elapsed;
            }
            let exclusive = elapsed.saturating_sub(children);
            *state.stages.entry(self.name).or_default() += exclusive;
            if let Some(index) = self.resource {
                *state.resources[index].stages.entry(self.name).or_default() += exclusive;
            }
        }
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn durations(values: &BTreeMap<&'static str, Duration>) -> Value {
    json!(
        values
            .iter()
            .map(|(key, value)| (*key, millis(*value)))
            .collect::<BTreeMap<_, _>>()
    )
}
