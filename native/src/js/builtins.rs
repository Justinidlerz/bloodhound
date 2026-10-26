//! Knowledge about the browser global environment.
//!
//! These tables decide how an unbound identifier is classified. They are
//! language and host facts, not a business allowlist: anything not listed
//! here is an *unbound reference* and is ignored, because writing to an
//! unknown bare identifier is either a `ReferenceError` in strict mode or a
//! bundler-injected binding we cannot see.

use super::values::Root;

/// Names that evaluate to the global object in a window environment.
pub const GLOBAL_ALIASES: &[&str] = &["window", "globalThis", "self", "frames", "top", "parent"];

/// ECMAScript builtins whose prototypes/statics are shared by every script
/// on the page. Writes to these are `builtin-mutation`.
pub const BUILTINS: &[&str] = &[
    "Object",
    "Function",
    "Array",
    "String",
    "Number",
    "Boolean",
    "Symbol",
    "BigInt",
    "Promise",
    "RegExp",
    "Date",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "WeakRef",
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "EvalError",
    "ReferenceError",
    "URIError",
    "AggregateError",
    "JSON",
    "Math",
    "Reflect",
    "Proxy",
    "Intl",
    "ArrayBuffer",
    "SharedArrayBuffer",
    "DataView",
    "Atomics",
    "Int8Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "Int16Array",
    "Uint16Array",
    "Int32Array",
    "Uint32Array",
    "Float32Array",
    "Float64Array",
    "BigInt64Array",
    "BigUint64Array",
    "Iterator",
    "FinalizationRegistry",
    "globalThis",
];

/// Host APIs whose replacement changes behavior for every script on the
/// page. Writes to these (or their prototypes) are `host-api-override`.
pub const HOST_APIS: &[&str] = &[
    "fetch",
    "XMLHttpRequest",
    "WebSocket",
    "EventSource",
    "Request",
    "Response",
    "Headers",
    "URL",
    "URLSearchParams",
    "history",
    "History",
    "location",
    "Location",
    "navigator",
    "Navigator",
    "localStorage",
    "sessionStorage",
    "Storage",
    "indexedDB",
    "console",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "requestIdleCallback",
    "queueMicrotask",
    "structuredClone",
    "addEventListener",
    "removeEventListener",
    "dispatchEvent",
    "postMessage",
    "alert",
    "confirm",
    "prompt",
    "open",
    "close",
    "Element",
    "HTMLElement",
    "Node",
    "EventTarget",
    "Event",
    "CustomEvent",
    "MutationObserver",
    "IntersectionObserver",
    "ResizeObserver",
    "PerformanceObserver",
    "performance",
    "crypto",
    "Worker",
    "SharedWorker",
    "ServiceWorker",
    "Image",
    "Audio",
    "Option",
    "HTMLImageElement",
    "HTMLScriptElement",
    "HTMLStyleElement",
    "HTMLLinkElement",
    "HTMLIFrameElement",
    "HTMLCanvasElement",
    "CanvasRenderingContext2D",
    "Document",
    "Window",
    "DOMParser",
    "FormData",
    "Blob",
    "File",
    "FileReader",
    "AbortController",
    "TextEncoder",
    "TextDecoder",
    "customElements",
    "CSS",
    "CSSStyleSheet",
    "matchMedia",
    "getComputedStyle",
    "screen",
    "scrollTo",
    "scrollBy",
    "scroll",
    "print",
    "stop",
    "focus",
    "blur",
    "eval",
    "encodeURIComponent",
    "decodeURIComponent",
    "encodeURI",
    "decodeURI",
    "escape",
    "unescape",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "btoa",
    "atob",
    "onerror",
    "onload",
    "onunhandledrejection",
    "onbeforeunload",
    "onunload",
    "onpopstate",
    "onhashchange",
    "onresize",
    "onscroll",
    "onmessage",
];

/// `window.<name>` properties that are routinely assigned by application
/// code without polluting other scripts (they are instance state of the
/// current document, not shared namespace). Writes through these paths are
/// not reported.
pub const BENIGN_WINDOW_WRITES: &[&str] = &[
    "location",
    "name",
    "status",
    "scrollX",
    "scrollY",
    "pageXOffset",
    "pageYOffset",
    "opener",
    "event",
    "innerWidth",
    "innerHeight",
    "outerWidth",
    "outerHeight",
    "devicePixelRatio",
    "returnValue",
];

/// `document.<name>` properties where assignment is the intended API and
/// has only document-local effect. Not reported.
pub const BENIGN_DOCUMENT_WRITES: &[&str] = &["cookie", "title", "domain", "designMode", "dir"];

/// Methods that insert nodes or markup into their receiver.
pub const INSERTION_METHODS: &[&str] = &[
    "appendChild",
    "append",
    "prepend",
    "insertBefore",
    "replaceChild",
    "replaceChildren",
    "insertAdjacentElement",
    "insertAdjacentHTML",
    "insertAdjacentText",
    "before",
    "after",
    "replaceWith",
];

/// `document.*` methods that write markup directly.
pub const DOCUMENT_WRITE_METHODS: &[&str] = &["write", "writeln"];

/// Document-level receivers whose children belong to the page as a whole.
pub const DOCUMENT_CONTAINERS: &[&str] = &["body", "head", "documentElement"];

/// Classifies a free (unbound) identifier.
pub fn classify(name: &str) -> Option<Root> {
    if GLOBAL_ALIASES.contains(&name) {
        return Some(Root::Global);
    }
    if name == "document" {
        return Some(Root::Document);
    }
    if let Some(b) = BUILTINS.iter().find(|b| **b == name) {
        return Some(Root::Builtin(b));
    }
    if let Some(h) = HOST_APIS.iter().find(|h| **h == name) {
        return Some(Root::Host(h));
    }
    None
}

/// Resolves `window.<name>` to the same root as a bare `<name>` would get,
/// so `window.fetch = x` and `fetch = x` are classified identically.
pub fn reclassify_global_child(name: &str) -> Option<Root> {
    match classify(name) {
        Some(Root::Global) => Some(Root::Global),
        other => other,
    }
}

#[cfg(test)]
#[path = "../tests/js_builtins.rs"]
mod tests;
