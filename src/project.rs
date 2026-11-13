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
        packages::{Eco, Mode, Packages},
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
    ///
    /// When loaded from a file, the project's packages are installed last,
    /// for each ecosystem found next to it, unless `packages` says otherwise.
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
        // `dev.yaml` has an empty parent; it means the current dir.
        let root = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        Self::from_yaml_at(&yaml, root, lookup).wrap_err_with(|| format!("invalid project file {}", path.display()))
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
    ///
    /// With no project dir to look in, `packages` is checked but adds
    /// nothing; see [`Project::from_yaml_at`].
    pub fn from_yaml_with(yaml: &str, lookup: &Lookup<'_>) -> Result<Self> {
        Self::parse(yaml, None, lookup)
    }

    /// Like [`Project::from_yaml_with`], for a `dev.yaml` in `root`, whose
    /// packages are installed as `packages` says.
    pub fn from_yaml_at(yaml: &str, root: impl AsRef<Path>, lookup: &Lookup<'_>) -> Result<Self> {
        Self::parse(yaml, Some(root.as_ref()), lookup)
    }

    fn parse(yaml: &str, root: Option<&Path>, lookup: &Lookup<'_>) -> Result<Self> {
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

        let mut deps = raw.deps.deps;
        if let Some(root) = root {
            match raw.deps.packages {
                PackagesSetting::Auto => {
                    let packages = Packages::detect(root, Mode::from_env());
                    // Nothing found, so nothing to show.
                    if !packages.tasks().is_empty() {
                        deps.push(Box::new(packages));
                    }
                }
                PackagesSetting::Off => {}
                PackagesSetting::Only { ecos, at } => {
                    deps.insert(at, Box::new(Packages::only(root, &ecos, Mode::from_env())?))
                }
            }
        }

        Ok(Self {
            name: raw.name,
            env,
            commands: raw.commands,
            deps,
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

/// The `deps` map, turned into dependencies by key as it's read. Packages
/// need the project dir, so only their setting is kept here.
#[derive(Default)]
struct Deps {
    deps: Vec<Box<dyn Dependency>>,
    packages: PackagesSetting,
}

/// What `packages` under `deps` says.
#[derive(Default)]
enum PackagesSetting {
    /// Not set: every ecosystem found, last.
    #[default]
    Auto,
    /// `packages: false`.
    Off,
    /// `packages: [node, ruby]`: those, at index `at` of the deps.
    Only { ecos: Vec<Eco>, at: usize },
}

/// The value of `packages`: `false`, or a list of ecosystems.
enum PackagesValue {
    Off,
    Only(Vec<Eco>),
}

impl<'de> Deserialize<'de> for PackagesValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(PackagesVisitor)
    }
}

struct PackagesVisitor;

impl<'de> Visitor<'de> for PackagesVisitor {
    type Value = PackagesValue;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("`false`, or a list of ecosystems like `[node, ruby]` (node, ruby, go, rust, python)")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<PackagesValue, E> {
        if v {
            return Err(E::custom(
                "`packages: true` is the default; leave it out, or list ecosystems like `[node, ruby]`",
            ));
        }
        Ok(PackagesValue::Off)
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<PackagesValue, A::Error> {
        let mut ecos = Vec::new();
        while let Some(eco) = seq.next_element::<Eco>()? {
            ecos.push(eco);
        }
        Ok(PackagesValue::Only(ecos))
    }
}

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
        let mut packages = PackagesSetting::Auto;
        // Whether `packages` came after the first tool, so it moves along
        // when the tools dep is inserted before it.
        let mut packages_after_tools = false;

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
                "packages" => {
                    packages = match map.next_value()? {
                        PackagesValue::Off => PackagesSetting::Off,
                        PackagesValue::Only(ecos) => PackagesSetting::Only { ecos, at: deps.len() },
                    };
                    packages_after_tools = tools_at.is_some();
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
            if let PackagesSetting::Only { at: packages_at, .. } = &mut packages
                && packages_after_tools
            {
                *packages_at += 1;
            }
        }

        Ok(Deps { deps, packages })
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
pub fn validate_name(name: &str) -> Result<()> {
    let valid = name
        .split('-')
        .all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));

    if !valid {
        bail!("project name `{name}` must be lowercase words separated by dashes, e.g. `demo-project`");
    }
    Ok(())
}

/// Replaces each `${VAR}` in env entry `name`'s `value` with `VAR` from
/// `lookup`, and each `${VAR:-default}` with `VAR` if it's set and not empty,
/// or `default` otherwise. A bare `$VAR` is left as is.
fn expand(name: &str, value: &str, lookup: &Lookup<'_>) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| eyre!("env `{name}` has a `${{` with no closing `}}`"))?;

        let expr = &after[..end];
        let val = match expr.split_once(":-") {
            Some((var, default)) => {
                if default.contains("${") {
                    bail!("env `{name}` has a `${{` inside the default for `${{{var}}}`, which isn't supported");
                }
                match lookup(var) {
                    Ok(val) if !val.is_empty() => val,
                    Ok(_) | Err(env::VarError::NotPresent) => default.to_string(),
                    Err(env::VarError::NotUnicode(_)) => {
                        bail!("env `{name}` references `${{{var}}}`, which isn't valid UTF-8")
                    }
                }
            }
            None => lookup(expr).map_err(|e| match e {
                env::VarError::NotPresent => eyre!("env `{name}` references `${{{expr}}}`, which isn't set"),
                env::VarError::NotUnicode(_) => eyre!("env `{name}` references `${{{expr}}}`, which isn't valid UTF-8"),
            })?,
        };
        out.push_str(&val);
        rest = &after[end + 1..];
    }

    out.push_str(rest);
    Ok(out)
}
