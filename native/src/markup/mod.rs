use crate::{
    model::{Context, gap, push, raw},
    resolve,
};
use cssparser::{Parser, Token as CssToken};
use html5gum::{DefaultEmitter, Token, Tokenizer};
use serde_json::{Value, json};
use std::collections::BTreeMap;

mod css;
mod html;
pub use css::css;
pub use html::html;

#[cfg(test)]
#[path = "../tests/markup.rs"]
mod tests;
