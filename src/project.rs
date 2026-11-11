use std::{collections::BTreeMap, env, fmt, fs, path::Path};

use color_eyre::eyre::{WrapErr, bail, eyre};
use serde::{
    Deserialize,
    Deserializer,
    de::{self, MapAccess, Visitor},
};

use crate::{
    Result,
    deps::{Dependency, brew::Brew},
};

/// A project as described by its `dev.yaml`.
///
/// ```yaml
/// name: demo-project
/// env:
///   PATH: "${PWD}/bin:${PATH}"
/// commands:
///   test: cargo test
/// deps:
///   brew:
///     - jq
///     - yq
/// ```
pub struct Project {
    /// Lowercase words separated by dashes, e.g. `demo-project`.
    pub name: String,
    /// Set on every process `dev` runs for this project, sorted by name.
    /// `${NAME}` in a value is replaced with `NAME` from the environment `dev`
    /// started in.
    pub env: Vec<(String, String)>,
    /// Shell snippets run by `dev <name>`, by name.
    pub commands: BTreeMap<String, String>,
    /// In the order they appear under `deps`.
    pub deps: Vec<Box<dyn Dependency>>,
}

impl Project {
    /// Reads and parses the `dev.yaml` at `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let yaml = fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Self::from_yaml(&yaml).wrap_err_with(|| format!("invalid project file {}", path.display()))
    }

    /// Parses a `dev.yaml` document.
    ///
    /// Fails on unknown top-level fields, unsupported dependency keys, names
    /// that aren't lowercase and dashed, and env values that reference unset
    /// variables.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let raw: RawProject = serde_saphyr::from_str(yaml)?;
        validate_name(&raw.name)?;

        let env = raw
            .env
            .into_iter()
            .map(|(name, value)| {
                let value = expand(&name, &value)?;
                Ok((name, value))
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            name: raw.name,
            env,
            commands: raw.commands,
            deps: raw.deps.0,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
    name: String,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    commands: BTreeMap<String, String>,
    #[serde(default)]
    deps: Deps,
}

/// The `deps` map, turned into dependencies by key as it's read.
#[derive(Default)]
struct Deps(Vec<Box<dyn Dependency>>);

impl<'de> Deserialize<'de> for Deps {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_map(DepsVisitor)
    }
}

struct DepsVisitor;

impl<'de> Visitor<'de> for DepsVisitor {
    type Value = Deps;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a map of dependency type to its settings, e.g. `brew: [jq]`")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Deps, A::Error> {
        let mut deps: Vec<Box<dyn Dependency>> = Vec::new();

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "brew" => {
                    let packages: Vec<String> = map.next_value()?;
                    deps.push(Box::new(Brew::new(packages)));
                }
                // Fail while reading the value so the error points at this
                // entry. Failing here directly points at the start of `deps`.
                other => {
                    map.next_value_seed(Unsupported(other))?;
                }
            }
        }

        Ok(Deps(deps))
    }
}

/// Rejects the value of an unsupported dependency key.
struct Unsupported<'a>(&'a str);

impl<'de> de::DeserializeSeed<'de> for Unsupported<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, _deserializer: D) -> std::result::Result<(), D::Error> {
        Err(de::Error::custom(format!("unsupported dependency `{}`", self.0)))
    }
}

/// Requires lowercase letters and digits in words separated by single dashes.
fn validate_name(name: &str) -> Result<()> {
    let valid = name
        .split('-')
        .all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));

    if !valid {
        bail!("project name `{name}` must be lowercase words separated by dashes, e.g. `demo-project`");
    }
    Ok(())
}

/// Replaces each `${VAR}` in env entry `name`'s `value` with `VAR` from the
/// environment. A bare `$VAR` is left as is.
fn expand(name: &str, value: &str) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| eyre!("env `{name}` has a `${{` with no closing `}}`"))?;

        let var = &after[..end];
        let val = env::var(var).map_err(|e| match e {
            env::VarError::NotPresent => eyre!("env `{name}` references `${{{var}}}`, which isn't set"),
            env::VarError::NotUnicode(_) => eyre!("env `{name}` references `${{{var}}}`, which isn't valid UTF-8"),
        })?;
        out.push_str(&val);
        rest = &after[end + 1..];
    }

    out.push_str(rest);
    Ok(out)
}
