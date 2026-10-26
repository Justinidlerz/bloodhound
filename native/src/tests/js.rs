use crate::model::*;
use crate::test_support::{analyze_js, js, mjs, rules, targets};

fn finding_pairs(a: &Analysis) -> Vec<(Rule, String)> {
    a.findings
        .iter()
        .map(|f| (f.rule, f.target.clone()))
        .collect()
}

#[test]
fn smoke() {
    let m = js(r#"
        window.sharedConfig = {};
        globalThis.counter = 1;
        var legacy = 1;
        function helper() {}
        const local = 2;
        let also = 3;
        Array.prototype.flatMap = function () {};
        window.fetch = () => {};
        fetch = () => {};
        history.pushState = function () {};
        document.body.appendChild(document.createElement('div'));
        document.title = 'x';
        document.body.style.overflow = 'hidden';
        var s = document.createElement('style'); document.head.appendChild(s);
        var t = document.createElement('style'); document.head.appendChild(t); t.remove();
        window.addEventListener('resize', () => {});
        window.addEventListener('scroll', () => {});
        window.removeEventListener('scroll', () => {});
        location.href = '/x';
        document.cookie = 'a=b';
        const g = window; g.alias = 1;
        (function (root) { root.umd = 1; })(window);
        (function () { this.viaThis = 1; })();
        Object.defineProperty(window, 'defined', { value: 1 });
        Object.assign(window, { a: 1, b: 2 });
        delete window.gone;
        window.counter++;
        document.write('<p>');
        customElements.define('x-foo', class {});
        var obj = {}; obj.notGlobal = 1;
        function f() { var inner = 1; window.insideFn = 1; }
    "#);
    for f in &m.findings {
        println!(
            "{:?} {} {} {:?} {}:{}",
            f.rule, f.target, f.operation, f.confidence, f.location.line, f.location.column
        );
    }
    println!("gaps: {:?}", m.gaps);
    let t = targets(&m);
    assert!(t.contains(&"window.sharedConfig"));
    assert!(t.contains(&"window.counter"));
    assert!(t.contains(&"window.legacy"));
    assert!(t.contains(&"window.helper"));
    assert!(!t.contains(&"window.local"));
    assert!(!t.contains(&"window.also"));
    assert!(t.contains(&"Array.prototype.flatMap"));
    assert!(t.contains(&"fetch"));
    assert!(t.contains(&"history.pushState"));
    assert!(t.contains(&"document.body"));
    assert!(!t.contains(&"document.title"));
    assert!(t.contains(&"document.body.style.overflow"));
    assert!(!t.contains(&"location.href"));
    assert!(!t.contains(&"document.cookie"));
    assert!(t.contains(&"window.alias"));
    assert!(t.contains(&"window.umd"));
    assert!(t.contains(&"window.viaThis"));
    assert!(t.contains(&"window.defined"));
    assert!(t.contains(&"window.a"));
    assert!(t.contains(&"window.gone"));
    assert!(
        !t.contains(&"window:resize"),
        "anonymous callbacks are outside the listener rule"
    );
    assert!(
        !t.contains(&"window:scroll"),
        "anonymous callbacks are outside the listener rule"
    );
    assert!(t.contains(&"document"));
    assert!(t.contains(&"customElements.x-foo"));
    assert!(!t.iter().any(|x| x.contains("notGlobal")));
    assert!(!t.iter().any(|x| x.contains("inner")));
    assert!(t.contains(&"window.insideFn"));
    let style_injections = m
        .findings
        .iter()
        .filter(|f| f.rule == Rule::StyleInjection)
        .count();
    assert_eq!(style_injections, 1, "removed style must not be reported");
    let _ = (rules, finding_pairs, mjs, analyze_js);
}

#[test]
fn local_slots_do_not_mutate_the_values_they_contain() {
    let a = analyze_js(
        r#"
        const box = { root: window, width: document.documentElement.clientWidth };
        box.width = 10; box.width++; delete box.width;
        box.root.actual = 1;
        box.root = {};
        box.root.local = 1;
        const ds = { value: document.body.dataset }; ds.value.shared = 'yes';
    "#,
        ScriptMode::Module,
    );
    assert_eq!(
        finding_pairs(&a),
        vec![
            (Rule::GlobalWrite, "window.actual".into()),
            (Rule::DocumentWrite, "document.body.dataset.shared".into()),
        ]
    );
}

#[test]
fn cleanup_requires_identity_order_and_paired_removal() {
    let cases = [
        (
            "const s=document.createElement('style'); document.head.appendChild(s); s.remove();",
            0,
        ),
        (
            "const s=document.createElement('style'); s.remove(); document.head.appendChild(s);",
            1,
        ),
        (
            "let s=document.createElement('style'); document.head.appendChild(s); s=document.createElement('style'); s.remove();",
            1,
        ),
        (
            "const s=document.createElement('style'); document.head.appendChild(s); function dispose(){s.remove()}",
            1,
        ),
        (
            "const s=document.createElement('style'); document.head.appendChild(s); if(ok)s.remove();",
            1,
        ),
        (
            "const s=document.createElement('style'); document.head.appendChild(s); ok && s.remove();",
            1,
        ),
        (
            "const s=document.createElement('style'); document.head.appendChild(s); while(ok)s.remove();",
            1,
        ),
        (
            "const s=document.createElement('style'); document.head.appendChild(s); switch(x){case 1:s.remove();}",
            1,
        ),
        (
            "const s=document.createElement('style'); const alias=s; document.head.appendChild(s); alias.remove();",
            0,
        ),
        (
            "const s=document.createElement('style'); document.head.appendChild(s); document.head.removeChild(s);",
            0,
        ),
        (
            "const f=()=>{}; window.addEventListener('x',f); window.removeEventListener('x',f);",
            0,
        ),
        (
            "window.addEventListener('x',()=>{}); window.removeEventListener('x',()=>{});",
            0,
        ),
        (
            "const f=()=>{}; window.addEventListener('x',f,true); window.removeEventListener('x',f,false);",
            0,
        ),
        (
            "const f=()=>{}; window.addEventListener('x',f); function dispose(){window.removeEventListener('x',f)}",
            0,
        ),
        (
            "var onTrap=function(){}; var attach=function(){document.addEventListener('focusin',onTrap)}; var detach=function(){document.removeEventListener('focusin',onTrap)}; attach();",
            0,
        ),
        (
            "var onTrap=function(){}; var detach=function(){document.removeEventListener('focusin',onTrap)}; var attach=function(){document.addEventListener('focusin',onTrap)}; attach();",
            0,
        ),
        (
            "const f=()=>{}; window.removeEventListener('x',f); window.addEventListener('x',f);",
            0,
        ),
        (
            "function on(ev,fn,cap){window.addEventListener(ev,fn,cap)} function off(ev,fn,cap){window.removeEventListener(ev,fn,cap)}",
            1,
        ),
        (
            "function on(ev,fn,cap){window.addEventListener(ev,fn,cap)} function off(ev,fn,other){window.removeEventListener(ev,fn,other)}",
            1,
        ),
        (
            "const f=()=>{}; window.addEventListener('x',f,{once:!0});",
            0,
        ),
        (
            "const f=()=>{}; const c=new AbortController(); window.addEventListener('x',f,{signal:c.signal});",
            0,
        ),
        (
            "const f=()=>{}; const g=f; window.addEventListener('x',f); window.removeEventListener('x',g);",
            0,
        ),
        (
            "class C{mount(){document.addEventListener('keydown',this.h)} unmount(){const t=this; document.removeEventListener('keydown',t.h)}}",
            0,
        ),
        (
            "class C{mount(){window.addEventListener('message',this.h)} unmount(){var Z=getGlobal(); Z.removeEventListener('message',this.h)}}",
            0,
        ),
        (
            "function attach(){window.addEventListener(cfg.evt,this.h)} function detach(){window.removeEventListener(cfg.evt,this.h)}",
            0,
        ),
        (
            "var Wa=function(e){window.addEventListener('pagehide',e,!1)}; var n=()=>{}; Wa(n); window.removeEventListener('pagehide',n);",
            0,
        ),
        (
            "var Wa=function(e){window.addEventListener('pagehide',e,!1)}; var n=()=>{}; var m=()=>{}; Wa(n); window.removeEventListener('pagehide',m);",
            1,
        ),
        (
            "const a=()=>{}; document.addEventListener('visibilitychange',a,!1); (function(l,c,d,h){l.removeEventListener(c,d,h)})(document,'visibilitychange',a,!1);",
            0,
        ),
        (
            "var onTrap=function(){}; var attach=function(){document.addEventListener('focusin',onTrap)}; var detach=function(){if(ok)document.removeEventListener('focusin',onTrap)}; attach();",
            0,
        ),
        (
            "class C{mount(){document.addEventListener('visibilitychange',this.visibilityHandler)} unmount(){document.removeEventListener('visibilitychange',this.visibilityHandler)}}",
            0,
        ),
        (
            "class C{mount(){document.addEventListener('visibilitychange',this.visibilityHandler)} unmount(){document.removeEventListener('visibilitychange',this.otherHandler)}}",
            1,
        ),
        (
            "class C{mount(){if(ok)window.addEventListener('mousemove',this.mouseMoveHandler)} unmount(){window.removeEventListener('mousemove',this.mouseMoveHandler)}}",
            0,
        ),
        (
            "function sub(){const f=()=>{}; window.addEventListener('x',f); return ()=>window.removeEventListener('x',f)}",
            0,
        ),
        (
            "function sub(){var f=function(){}; return window.addEventListener('x',f),function(){window.removeEventListener('x',f)}}",
            0,
        ),
        (
            "function sub(){const f=()=>{}; window.addEventListener('x',f); return ()=>window.removeEventListener('y',f)}",
            1,
        ),
        (
            "function sub(){const f=()=>{}; window.addEventListener('x',f); return ()=>{if(ok)window.removeEventListener('x',f)}}",
            0,
        ),
        (
            "const f=()=>{}; window.addEventListener('x',f,!1); window.removeEventListener('x',f,!1);",
            0,
        ),
        (
            "const f=()=>{}; window.addEventListener('x',f,!0); window.removeEventListener('x',f,!1);",
            0,
        ),
        (
            "function C(){useEffect(()=>{if(visible){const n=e=>{}; return document==null||document.addEventListener('wheel',n,{passive:!1}),()=>{document==null||document.removeEventListener('wheel',n,!1)}}},[visible])}",
            0,
        ),
        (
            "const f=()=>{}; window.addEventListener('x',f); window==null||window.removeEventListener('x',f);",
            0,
        ),
        ("window.addEventListener('x',()=>{});", 0),
        ("window.addEventListener('x',function(){});", 0),
        ("const f=()=>{}; window.addEventListener('x',f);", 1),
        (
            "const f=()=>{}; const g=()=>{}; window.addEventListener('x',f); document.removeEventListener('x',g);",
            1,
        ),
        (
            "const f=()=>{}; window.addEventListener('x',f.bind(window)); window.removeEventListener('x',f);",
            1,
        ),
    ];
    for (code, expected) in cases {
        let a = analyze_js(code, ScriptMode::Module);
        assert!(a.gaps.is_empty(), "{code}: {:?}", a.gaps);
        assert_eq!(a.findings.len(), expected, "{code}: {:?}", a.findings);
    }
}

#[test]
fn function_analysis_does_not_execute_captured_assignments() {
    let a = analyze_js(
        "let g=window; function reset(){g={}} g.shared=1;",
        ScriptMode::Module,
    );
    assert_eq!(
        finding_pairs(&a),
        vec![(Rule::GlobalWrite, "window.shared".into())]
    );
    assert_eq!(a.findings[0].confidence, Confidence::Medium);
}

#[test]
fn classic_globals_strict_functions_and_module_scope() {
    let a = analyze_js(
        "implicit=1; (function(){'use strict'; this.no=1; other=2;})();",
        ScriptMode::Classic,
    );
    assert_eq!(
        finding_pairs(&a),
        vec![(Rule::GlobalWrite, "window.implicit".into())]
    );
    assert!(
        analyze_js(
            "var local=1; function localFn(){}; undeclared=1;",
            ScriptMode::Module
        )
        .findings
        .is_empty()
    );
    assert!(
        analyze_js("const window={}; window.x=1;", ScriptMode::Module)
            .findings
            .is_empty()
    );
}

#[test]
fn dynamic_links_are_stylesheets_only_when_rel_matches() {
    let a = analyze_js(
        r#"
      const icon=document.createElement('link'); icon.href='/icon.png'; icon.rel='icon'; document.head.appendChild(icon);
      const css=document.createElement('link'); css.href='/app.css'; css.rel='stylesheet'; document.head.appendChild(css);
      const js=document.createElement('script'); js.src='/app.js'; js.type='module'; document.head.appendChild(js);
      document.querySelector('body').style.overflow='hidden';
      document.getElementById('mount').appendChild(document.createElement('div'));
    "#,
        ScriptMode::Module,
    );
    assert_eq!(a.dependencies.len(), 2);
    assert_eq!(a.dependencies[0].kind, DependencyKind::DynamicStyle);
    assert_eq!(a.dependencies[1].script_mode, Some(ScriptMode::Module));
    assert_eq!(
        a.findings
            .iter()
            .filter(|f| f.rule == Rule::StyleInjection)
            .count(),
        1
    );
    assert!(
        a.findings
            .iter()
            .any(|f| f.target == "document.body.style.overflow")
    );
    assert!(a.findings.iter().any(|f| f.target == "document.selected"));
}
