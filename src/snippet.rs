use std::fmt;

use serde::{
    Deserialize,
    Deserializer,
    de::{self, SeqAccess, Visitor},
};

/// A shell snippet from `dev.yaml`: a string, or a list of strings.
///
/// A string is used as is. A list becomes one script that runs each item in
/// order and stops at the first one that fails, the way CI systems run a
/// list of steps. Shell state like `cd` and vars carries from one item to the
/// next. Items can be multi-line, but not lists themselves:
///
/// ```yaml
/// run:
///   - cargo fmt --check
///   - |
///     cd crates/core
///     cargo test
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Snippet(String);

impl Snippet {
    /// Joins `items` into a script that stops at the first failing item.
    ///
    /// Fails if `items` is empty or any item is blank.
    pub fn from_list(items: Vec<String>) -> Result<Self, String> {
        if items.is_empty() {
            return Err("a list of commands can't be empty".to_string());
        }
        if let Some(i) = items.iter().position(|item| item.trim().is_empty()) {
            return Err(format!("command {} in the list can't be empty", i + 1));
        }
        Ok(Self(format!("set -e\n{}", items.join("\n"))))
    }

    /// The script to run with `sh -c`.
    pub fn into_script(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for Snippet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(SnippetVisitor)
    }
}

struct SnippetVisitor;

impl<'de> Visitor<'de> for SnippetVisitor {
    type Value = Snippet;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a shell snippet, or a list of them")
    }

    fn visit_str<E: de::Error>(self, snippet: &str) -> Result<Snippet, E> {
        Ok(Snippet(snippet.to_string()))
    }

    // `true` and `false` are commands too, and YAML reads them as bools.
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Snippet, E> {
        Ok(Snippet(value.to_string()))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Snippet, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<String>()? {
            items.push(item);
        }
        Snippet::from_list(items).map_err(de::Error::custom)
    }
}
