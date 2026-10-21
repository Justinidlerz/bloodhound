use super::*;
use crate::test_support::{has_reason, rules};
fn run(code: &str, extra: Json) -> Json {
    let mut options = json!({"scriptMode":"module","executionEnvironment":"window","maxSteps":200000,"maxCandidates":64});
    for (k, v) in extra.as_object().unwrap() {
        options[k] = v.clone();
    }
    serde_json::from_str(&analyze(code.into(), options.to_string()).unwrap()).unwrap()
}
fn parse(code: &str) -> Json {
    run(code, json!({}))
}
fn accesses<'a>(out: &'a Json, ns: &str, path: &[&str], op: &str) -> Vec<&'a Json> {
    out["accesses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| {
            a["operation"] == op
                && a["targets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["namespace"] == ns && t["pathSegments"] == json!(path))
        })
        .collect()
}
#[test]
fn aliases_normalize_window_globalthis_and_self() {
    let out = parse("const g=window; g.shared={}; globalThis.shared; self.shared='x';");
    assert_eq!(
        accesses(&out, "global-object", &["shared"], "write").len(),
        2
    );
    assert_eq!(
        accesses(&out, "global-object", &["shared"], "read").len(),
        1
    );
}
#[test]
fn local_shadowing_and_reassigned_aliases_do_not_create_global_writes() {
    for code in [
        "function f(window){window.x=1}",
        "const window={};window.x=1",
        "let g=window;g={};g.x=1",
        "const {window}=obj;window.x=1",
    ] {
        assert!(
            accesses(&parse(code), "global-object", &["x"], "write").is_empty(),
            "{code}"
        );
    }
}
#[test]
fn classic_declarations_separate_object_and_lexical_namespaces() {
    let out = run(
        "var objectName=1; function fn(){} let lexicalName=2; const fixedName=3; class Klass {}",
        json!({"scriptMode":"classic"}),
    );
    for name in ["objectName", "fn"] {
        assert!(
            !accesses(&out, "global-object", &[name], "declare").is_empty(),
            "{name}"
        );
    }
    for name in ["lexicalName", "fixedName", "Klass"] {
        assert!(
            !accesses(&out, "global-lexical", &[name], "declare").is_empty(),
            "{name}"
        );
    }
    let module = parse("var objectName=1; let lexicalName=2;");
    assert!(module["accesses"].as_array().unwrap().is_empty());
}
#[test]
fn unbound_writes_stay_candidates_and_module_this_is_not_window() {
    let out = parse("missing=1; this.x=2;");
    assert_eq!(
        accesses(&out, "unbound-reference", &["missing"], "write").len(),
        1
    );
    assert!(accesses(&out, "global-object", &["missing"], "write").is_empty());
    assert!(accesses(&out, "global-object", &["x"], "write").is_empty());
    assert_eq!(
        accesses(
            &run("this.x=2", json!({"scriptMode":"classic"})),
            "global-object",
            &["x"],
            "write"
        )
        .len(),
        1
    );
}
#[test]
fn compound_updates_and_delete_preserve_operation_types() {
    let out = parse("window.x+=1; window.y++; delete window.z;");
    for name in ["x", "y"] {
        assert!(!accesses(&out, "global-object", &[name], "read").is_empty());
        assert!(!accesses(&out, "global-object", &[name], "write").is_empty());
    }
    assert_eq!(accesses(&out, "global-object", &["z"], "delete").len(), 1);
}
#[test]
fn finite_keys_use_templates_tables_arrays_and_branch_merges() {
    let out = parse(
        "const prefix='s'; const table={key:'hared'}; const a=['shared']; window[prefix+table.key]=1; window[`${a[0]}`]=2; let k;if(flag){k='left'}else{k='right'} window[k]=3;",
    );
    assert_eq!(
        accesses(&out, "global-object", &["shared"], "write").len(),
        2
    );
    for name in ["left", "right"] {
        assert!(!accesses(&out, "global-object", &[name], "write").is_empty());
    }
}
#[test]
fn unknown_keys_keep_accesses_without_a_shared_fake_target() {
    let out = parse("window[input]=1; window[other]=2;");
    let writes: Vec<_> = out["accesses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["operation"] == "write")
        .collect();
    assert_eq!(writes.len(), 2);
    assert!(writes.iter().all(|a| a["targets"] == json!([])));
    assert!(has_reason(&out, "dynamic-property-key"));
}
#[test]
fn symbol_keys_keep_local_registered_and_string_identities_separate() {
    let out = parse(
        "const a=Symbol('key'); const b=Symbol('key'); window[a]=1; window[b]=2; window[Symbol.for('key')]=3; window.key=4;",
    );
    let targets: Vec<_> = out["accesses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["operation"] == "write")
        .map(|a| a["targets"][0].clone())
        .collect();
    assert_eq!(targets.len(), 4);
    assert_ne!(targets[0], targets[1]);
    assert_eq!(targets[0]["keyKind"], "symbol-local");
    assert_eq!(targets[2]["keyKind"], "symbol-for");
    assert_eq!(targets[3]["keyKind"], "string");
}
#[test]
fn known_object_operations_record_writes_definitions_and_deletes() {
    let out = parse(
        "Object.assign(window,{a:1,b:'s'}); Object.defineProperty(window,'c',{value:true}); Object.defineProperties(window,{d:{value:[]}}); Reflect.set(window,'e',1); Reflect.deleteProperty(window,'f'); Reflect.defineProperty(window,'g',{value:null});",
    );
    for (name, operation) in [
        ("a", "write"),
        ("b", "write"),
        ("c", "define"),
        ("d", "define"),
        ("e", "write"),
        ("f", "delete"),
        ("g", "define"),
    ] {
        assert_eq!(
            accesses(&out, "global-object", &[name], operation).len(),
            1,
            "{name}"
        );
    }
    assert!(has_reason(
        &parse("Object.assign(window,unknown)"),
        "unknown-object-operation-keys"
    ));
}
#[test]
fn shadowed_native_apis_and_getters_are_not_evaluated_as_builtins() {
    let out = parse(
        "const Object={assign(){}};Object.assign(window,{x:1}); const obj={get key(){throw Error('must not execute')}};window[obj.key]=1;",
    );
    assert!(accesses(&out, "global-object", &["x"], "write").is_empty());
    assert!(has_reason(&out, "dynamic-property-key"));
}
#[test]
fn iife_and_umd_parameter_bindings_preserve_global_targets() {
    for code in [
        "(function(root){root.x=1})(window)",
        "(function(root,factory){factory(root)})(globalThis,function(g){g.x=1})",
    ] {
        assert!(
            !accesses(&parse(code), "global-object", &["x"], "write").is_empty(),
            "{code}"
        );
    }
}
#[test]
fn unknown_calls_and_complex_control_flow_report_limits() {
    for (code, reason) in [
        (
            "let g=window; unknown(); g.x=1",
            "mutable-alias-after-unknown-call",
        ),
        (
            "let g=window; function f(){g.x=1}",
            "mutable-captured-alias",
        ),
        ("switch(x){case 1:window.x=1}", "switch-flow-not-propagated"),
        ("try{window.x=1}catch(e){}", "exception-flow-not-propagated"),
        ("eval('window.x=1')", "dynamic-code-execution"),
    ] {
        assert!(has_reason(&parse(code), reason), "{reason}");
    }
    assert!(has_reason(
        &run("with(obj){x=1}", json!({"scriptMode":"classic"})),
        "with-dynamic-scope"
    ));
}
#[test]
fn missing_context_parse_errors_and_semantic_errors_are_distinct() {
    let out = run(
        "window.x=1",
        json!({"scriptMode":"unknown","executionEnvironment":"unknown"}),
    );
    assert!(has_reason(&out, "missing-script-mode"));
    assert!(has_reason(&out, "missing-execution-environment"));
    assert!(has_reason(&parse("const = ;"), "parse-error"));
    let invalid = parse("let x;let x;");
    assert!(has_reason(&invalid, "semantic-error") || has_reason(&invalid, "parse-error"));
    assert!(analyze("".into(), "{}".into()).is_err());
}
#[test]
fn budgets_bound_candidates_ast_visits_and_elapsed_analysis() {
    assert!(has_reason(
        &run("window[flag?'a':'b']=1", json!({"maxCandidates":1})),
        "candidate-budget"
    ));
    let out = run("window.a=1;window.b=2;window.c=3;", json!({"maxSteps":1}));
    assert!(has_reason(&out, "ast-visit-budget"));
    let out = run(&"window.a=1;".repeat(200), json!({"deadlineUnixMs":1}));
    assert!(has_reason(&out, "analysis-time-budget"));
}
#[test]
fn esm_references_and_finite_dynamic_imports_preserve_kinds() {
    let out = parse(
        "import {x} from './one.js'; export {y} from './two.js'; export * from './three.js'; import(flag?'./left.js':'./right.js'); import(unknown);",
    );
    let deps = out["dependencies"].as_array().unwrap();
    assert!(
        deps.iter()
            .any(|d| d["kind"] == "import" && d["specifiers"] == json!(["./one.js"]))
    );
    assert!(
        deps.iter()
            .any(|d| d["kind"] == "export" && d["specifiers"] == json!(["./two.js"]))
    );
    assert!(
        deps.iter().any(|d| d["kind"] == "dynamic-import"
            && d["specifiers"] == json!(["./left.js", "./right.js"]))
    );
    assert!(has_reason(&out, "non-finite-dependency"));
}
#[test]
fn module_summaries_export_only_supported_immutable_values() {
    let out = parse(
        "export const name='shared';export const count=2;export const yes=true;export const empty=null;export let mutable=1;export const object={x:1};",
    );
    assert_eq!(
        out["exports"],
        json!({"name":"shared","count":2.0,"yes":true,"empty":null})
    );
    let out = run(
        "import {name} from './config.js';window[name]=1",
        json!({"importedConstants":{"./config.js#name":"shared"}}),
    );
    assert_eq!(
        accesses(&out, "global-object", &["shared"], "write").len(),
        1
    );
}
#[test]
fn worker_urls_distinguish_document_and_importer_bases() {
    let out = run(
        "new Worker('./worker.js',{type:'module'}); new SharedWorker(new URL('./shared.js',import.meta.url));",
        json!({"baseUrl":"https://example.test/page/","resourceUrl":"https://example.test/modules/main.js"}),
    );
    assert_eq!(out["dependencies"][0]["scriptMode"], "module");
    assert_eq!(
        out["dependencies"][0]["baseUrl"],
        "https://example.test/page/"
    );
    assert_eq!(
        out["dependencies"][1]["baseUrl"],
        "https://example.test/modules/main.js"
    );
    assert!(has_reason(
        &parse("new Worker('./x.js')"),
        "worker-document-base-unknown"
    ));
    assert_eq!(
        run(
            "importScripts('a.js','b.js')",
            json!({"executionEnvironment":"worker","scriptMode":"classic"})
        )["dependencies"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn dom_operations_and_dynamic_resources_remain_static_candidates() {
    let out = parse(
        "document.body.appendChild(node);document.body.innerHTML='x'; const s=document.createElement('script');s.src='/entry.js';s.text='window.inline=1';const f=document.createElement('iframe');f.srcdoc='<p>x</p>';",
    );
    assert!(rules(&out).contains(&"dom-operation"));
    assert!(rules(&out).contains(&"dom-property-write"));
    assert!(
        out["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["specifiers"] == json!(["/entry.js"]))
    );
    assert!(
        out["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["content"] == "window.inline=1" && d["sourceLocationApproximate"] == true)
    );
}
#[test]
fn webpack_chunk_tables_resolve_without_matching_variable_names() {
    let out = parse(
        "var runtime={};runtime.p='/assets/';runtime.u=id=>({1:'one',2:'two'}[id])+'.js';runtime.load=function(id){const script=document.createElement('script');script.src=runtime.p+runtime.u(id)};",
    );
    let dep = out["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "webpack-chunk")
        .expect("finite webpack chunks");
    assert_eq!(
        dep["specifiers"],
        json!(["/assets/one.js", "/assets/two.js"])
    );
}
#[test]
fn vite_preload_recognizes_function_shape_and_rejects_unknown_arrays() {
    let helper = "const renamed=(load,deps)=>{document.createElement('link');return load()};";
    let out = parse(&format!(
        "{helper} renamed(()=>import('./entry.js'),['./dep.js','./style.css']);"
    ));
    assert_eq!(
        out["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["kind"] == "preload")
            .count(),
        2
    );
    assert!(has_reason(
        &parse(&format!(
            "{helper} renamed(()=>import('./entry.js'),unknown);"
        )),
        "non-finite-preload-array"
    ));
    assert!(
        !parse("other(()=>import('./entry.js'),['./dep.js'])")["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["kind"] == "preload")
    );
}
#[test]
fn json_conversion_refuses_unknown_and_ambiguous_values() {
    let input = json!({"array":[1.0,true,null,"s"],"object":{"a":2.0}});
    assert_eq!(to_json(&json_value(&input)).unwrap(), input);
    assert!(to_json(&Value::Unknown).is_none());
    assert!(to_json(&Value::Strings(vec!["a".into(), "b".into()])).is_none());
}

#[test]
fn destructuring_keeps_known_values_and_drops_unknown_rest() {
    let out = parse(
        "const {key, ...rest}={key:'object',extra:'rest'};const [first,,third]=['array','skip','third'];window[key]=1;window[first]=1;window[third]=1;window[rest.extra]=1;",
    );
    for name in ["object", "array", "third"] {
        assert_eq!(accesses(&out, "global-object", &[name], "write").len(), 1);
    }
    assert!(accesses(&out, "global-object", &["rest"], "write").is_empty());
    assert!(has_reason(&out, "dynamic-property-key"));
}
#[test]
fn loops_invalidate_mutable_aliases_but_keep_constant_roots() {
    for body in [
        "for(let i=0;i<2;i++){g.x=1;fixed.y=1}",
        "for(const key in input){g.x=1;fixed.y=1}",
        "for(const value of input){g.x=1;fixed.y=1}",
        "while(flag){g.x=1;fixed.y=1}",
        "do{g.x=1;fixed.y=1}while(flag)",
    ] {
        let out = parse(&format!("let g=window;const fixed=window;{body}"));
        assert!(
            accesses(&out, "global-object", &["x"], "write").is_empty(),
            "{body}"
        );
        assert_eq!(
            accesses(&out, "global-object", &["y"], "write").len(),
            1,
            "{body}"
        );
    }
}
#[test]
fn default_namespace_and_reexport_summaries_keep_export_names() {
    let constants = json!({"./config.js#default":"defaultKey","./config.js#key":"namedKey"});
    let out = run(
        "import key from './config.js';import * as ns from './config.js';window[key]=1;window[ns.key]=1;export {key as renamed} from './config.js';export * from './config.js';export * as all from './config.js';",
        json!({"importedConstants":constants}),
    );
    for name in ["defaultKey", "namedKey"] {
        assert_eq!(accesses(&out, "global-object", &[name], "write").len(), 1);
    }
    assert_eq!(out["exports"]["renamed"], "namedKey");
    assert_eq!(out["exports"]["key"], "namedKey");
    assert!(out["exports"].get("default").is_none());
    let out = parse("const value='name';export {value as alias};export default 'defaultKey';");
    assert_eq!(out["exports"]["alias"], "name");
    assert_eq!(out["exports"]["default"], "defaultKey");
}
#[test]
fn finite_primitive_expressions_resolve_property_keys() {
    let out = parse(
        "window[1+2]=1;window[true]=1;window[null]=1;window[!false?'yes':'no']=1;window[typeof window==='object'?'root':'wrong']=1;window['x'!=='y'?'different':'equal']=1;const object={['key']:'computed'};window[object.key]=1;",
    );
    for name in ["3", "true", "null", "yes", "root", "different", "computed"] {
        assert_eq!(
            accesses(&out, "global-object", &[name], "write").len(),
            1,
            "{name}"
        );
    }
    assert!(accesses(&out, "global-object", &["wrong"], "write").is_empty());
}
#[test]
fn pure_return_functions_resolve_values_without_executing_code() {
    for definition in [
        "const key=x=>{return x+'Key'}",
        "const key=function(x){return x+'Key'}",
    ] {
        let out = parse(&format!("{definition};window[key('shared')]=1"));
        assert_eq!(
            accesses(&out, "global-object", &["sharedKey"], "write").len(),
            1
        );
    }
    let code = "const key=x=>key(x);window[key('never')]=1";
    assert!(has_reason(&parse(code), "evaluation-depth-budget"));
}
#[test]
fn worker_environment_does_not_treat_window_as_worker_global() {
    let out = run(
        "self.x=1;globalThis.y=1;window.z=1;",
        json!({"executionEnvironment":"worker"}),
    );
    for name in ["x", "y"] {
        assert_eq!(accesses(&out, "global-object", &[name], "write").len(), 1);
    }
    assert!(accesses(&out, "global-object", &["z"], "write").is_empty());
    assert_eq!(
        accesses(&out, "unbound-reference", &["window", "z"], "write").len(),
        1
    );
}
#[test]
fn assigned_aliases_nested_properties_and_updates_preserve_identity() {
    let out = run(
        "var count=0;count++;missing++;const g=(window.store={});g.child=1;const obj={key:'before'};obj['key']='after';window[obj.key]=1;",
        json!({"scriptMode":"classic"}),
    );
    assert_eq!(
        accesses(&out, "global-object", &["count"], "write").len(),
        1
    );
    assert_eq!(
        accesses(&out, "unbound-reference", &["missing"], "write").len(),
        1
    );
    assert_eq!(
        accesses(&out, "global-object", &["store", "child"], "write").len(),
        1
    );
    assert_eq!(
        accesses(&out, "global-object", &["after"], "write").len(),
        1
    );
}
#[test]
fn conditional_aliases_keep_finite_targets_and_mark_mixed_values() {
    let out = parse(
        "const g=flag?window.left:window.right;g.x=1;const mixed=flag?window:{};mixed.y=1;const k=flag?'a':'b';const p=other?'c':'d';window[k+p]=1;",
    );
    for name in ["left", "right"] {
        assert_eq!(
            accesses(&out, "global-object", &[name, "x"], "write").len(),
            1
        );
    }
    assert!(has_reason(&out, "mixed-branch-target"));
    for name in ["ac", "ad", "bc", "bd"] {
        assert_eq!(accesses(&out, "global-object", &[name], "write").len(), 1);
    }
    assert!(has_reason(
        &run(
            "const k=flag?'a':'b';const p=other?'c':'d';window[k+p]=1",
            json!({"maxCandidates":2})
        ),
        "candidate-budget"
    ));
}
