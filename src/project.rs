use std::{collections::BTreeMap, env, fmt, fs, path::Path};

use color_eyre::eyre::{WrapErr, bail, eyre};
use serde::{
    Deserialize,
    Deserializer,
    de::{self, MapAccess, Visitor},
};

use crate::{
    Result,
    commands::ProjectCommand,
    deps::{
        Dependency,
        brew::Brew,
        mise::Mise,
        rust::{self, Rust},
    },
};

/// Resolves `${VAR}` in env values. Same contract as [`std::env::var`].
pub type Lookup<'a> = dyn Fn(&str) -> std::result::Result<String, env::VarError> + 'a;

/// A project as described by its `dev.yaml`.
///
/// ```yaml
/// name: demo-project
/// env:
///   PATH: "${PWD}/bin:${PATH}"
/// commands:
///   test: cargo test
///   deploy:
///     desc: Deploy an app
///     flags:
///       dry-run: { type: bool }
///     run: echo "dry run: $DRY_RUN"
/// deps:
///   brew:
///     - jq
///     - yq
///   node: lts
///   python: 3.14
/// ```
pub struct Project {
    /// Lowercase words separated by dashes, e.g. `demo-project`.
    pub name: String,
    /// Set on every process `dev` runs for this project, sorted by name.
    /// `${NAME}` in a value is replaced with `NAME` from the environment `dev`
    /// started in.
    pub env: Vec<(String, String)>,
    /// Run by `dev <name>`, by name.
    pub commands: BTreeMap<String, ProjectCommand>,
    /// In the order they appear under `deps`. Keys other than built-in
    /// dependencies (e.g. `brew`) are tools, all installed by one mise
    /// dependency where the first one appears.
    pub deps: Vec<Box<dyn Dependency>>,
}

impl Project {
    /// Reads and parses the `dev.yaml` at `path`, expanding env values from
    /// the process environment.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        Self::load_with(path, &|var| env::var(var))
    }

    /// Like [`Project::load`], but `${VAR}` in env values comes from `lookup`.
    pub fn load_with(path: impl AsRef<Path>, lookup: &Lookup<'_>) -> Result<Self> {
        let path = path.as_ref();
        let yaml = fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Self::from_yaml_with(&yaml, lookup).wrap_err_with(|| format!("invalid project file {}", path.display()))
    }

    /// Parses a `dev.yaml` document, expanding env values from the process
    /// environment.
    ///
    /// Fails on unknown top-level fields, reserved dependency keys, names
    /// that aren't lowercase and dashed, and env values that reference unset
    /// variables.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        Self::from_yaml_with(yaml, &|var| env::var(var))
    }

    /// Like [`Project::from_yaml`], but `${VAR}` in env values comes from
    /// `lookup`.
    pub fn from_yaml_with(yaml: &str, lookup: &Lookup<'_>) -> Result<Self> {
        let raw: RawProject = serde_saphyr::from_str(yaml)?;
        validate_name(&raw.name)?;

        let env = raw
            .env
            .into_iter()
            .map(|(name, value)| {
                let value = expand(&name, &value, lookup)?;
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
    commands: BTreeMap<String, ProjectCommand>,
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
        f.write_str("a map of dependency type to its settings, e.g. `brew: [jq]` or `node: lts`")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Deps, A::Error> {
        let mut deps: Vec<Box<dyn Dependency>> = Vec::new();
        // Every key that isn't a built-in dependency is a mise tool. They're
        // all met by one dependency, placed where the first tool appears.
        let mut tools: Vec<(String, String)> = Vec::new();
        let mut tools_at = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "brew" => {
                    let packages: Vec<String> = map.next_value()?;
                    deps.push(Box::new(Brew::new(packages)));
                }
                "rust" => {
                    let entries: Vec<rust::Entry> = map.next_value()?;
                    deps.push(Box::new(Rust::new(entries)));
                }
                // Reserved for a built-in dependency, so it can't be a tool.
                // Fail while reading the value so the error points at this
                // entry. Failing here directly points at the start of `deps`.
                "custom" => {
                    map.next_value_seed(Unsupported(&key))?;
                }
                _ => {
                    let version: String = map.next_value()?;
                    tools_at.get_or_insert(deps.len());
                    tools.push((key, version));
                }
            }
        }

        if let Some(at) = tools_at {
            deps.insert(at, Box::new(Mise::new(tools)));
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

/// Replaces each `${VAR}` in env entry `name`'s `value` with `VAR` from
/// `lookup`. A bare `$VAR` is left as is.
fn expand(name: &str, value: &str, lookup: &Lookup<'_>) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| eyre!("env `{name}` has a `${{` with no closing `}}`"))?;

        let var = &after[..end];
        let val = lookup(var).map_err(|e| match e {
            env::VarError::NotPresent => eyre!("env `{name}` references `${{{var}}}`, which isn't set"),
            env::VarError::NotUnicode(_) => eyre!("env `{name}` references `${{{var}}}`, which isn't valid UTF-8"),
        })?;
        out.push_str(&val);
        rest = &after[end + 1..];
    }

    out.push_str(rest);
    Ok(out)
}
