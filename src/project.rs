use std::{collections::BTreeMap, env, fmt, fs, path::Path};

use color_eyre::eyre::{WrapErr, bail, eyre};
use serde::{
    Deserialize,
    Deserializer,
    de::{self, MapAccess, Visitor},
};
use serde_saphyr::{Location, Spanned};

use crate::{
    Result,
    commands::ProjectCommand,
    deps::{
        Dependency,
        brew::Brew,
        mise::{self, Mise, ToolSpec},
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
    /// Lowercase words separated by dashes, e.g. `demo-project`. Optional in
    /// the global file, where it defaults to `global`.
    pub name: String,
    /// Set on every process `dev` runs for this project, sorted by name.
    /// `${NAME}` in a value is replaced with `NAME` from the environment `dev`
    /// started in.
    ///
    /// A project with tools also gets [`mise::CONFIG_FILENAME_VAR`], so mise
    /// loads the project's tools in it.
    pub env: Vec<(String, String)>,
    /// Run by `dev <name>`, by name.
    pub commands: BTreeMap<String, ProjectCommand>,
    /// In the order they appear under `deps`. Keys other than built-in
    /// dependencies (e.g. `brew`) are tools, all installed by one mise
    /// dependency where the first one appears. A project's tools go to its
    /// own mise config, the global file's to dev's global one.
    ///
    /// When loaded from a file, the project's packages are installed last,
    /// for each ecosystem found next to it, unless `packages` says otherwise.
    pub deps: Vec<Box<dyn Dependency>>,
    /// The tools under `deps`, as the mise dependency in [`Project::deps`]
    /// gets them.
    pub tools: Vec<ToolSpec>,
}

/// Which kind of `dev.yaml` is being read.
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    /// A project's, in `root`.
    Project,
    /// The user's own, which applies everywhere.
    Global,
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

    /// Reads and parses the user's global `dev.yaml` at `path`, expanding env
    /// values from the process environment.
    ///
    /// Unlike a project's, its `name` is optional, packages are never
    /// detected next to it, and its tools go to dev's global mise config.
    pub fn load_global(path: impl AsRef<Path>) -> Result<Self> {
        Self::load_global_with(path, &|var| env::var(var))
    }

    /// Like [`Project::load_global`], but `${VAR}` in env values comes from
    /// `lookup`.
    pub fn load_global_with(path: impl AsRef<Path>, lookup: &Lookup<'_>) -> Result<Self> {
        let path = path.as_ref();
        let yaml = fs::read_to_string(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Self::parse(&yaml, None, Scope::Global, lookup)
            .wrap_err_with(|| format!("invalid project file {}", path.display()))
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
        Self::parse(yaml, None, Scope::Project, lookup)
    }

    /// Like [`Project::from_yaml_with`], for a `dev.yaml` in `root`, whose
    /// packages are installed as `packages` says.
    pub fn from_yaml_at(yaml: &str, root: impl AsRef<Path>, lookup: &Lookup<'_>) -> Result<Self> {
        Self::parse(yaml, Some(root.as_ref()), Scope::Project, lookup)
    }

    /// `root` is the project dir, `None` when there's none to look in. A
    /// project's tools are still rooted at `.` then, as for `dev.yaml`.
    fn parse(yaml: &str, root: Option<&Path>, scope: Scope, lookup: &Lookup<'_>) -> Result<Self> {
        let raw: RawProject = serde_saphyr::from_str(yaml)?;
        let name = match (raw.name, scope) {
            (Some(name), _) => name,
            (None, Scope::Global) => "global".to_string(),
            (None, Scope::Project) => bail!("`name` is required, e.g. `name: demo-project`"),
        };
        validate_name(&name)?;
        if raw.env.contains_key(mise::CONFIG_FILENAME_VAR) {
            bail!(
                "env `{}` is set by dev, so mise finds a project's tools; remove it from dev.yaml",
                mise::CONFIG_FILENAME_VAR
            );
        }

        let mut env: Vec<(String, String)> = raw
            .env
            .into_iter()
            .map(|(name, value)| {
                let value = expand(&name, &value, lookup)?;
                Ok((name, value))
            })
            .collect::<Result<_>>()?;

        let mut deps = raw.deps.deps;
        let mut tools = Vec::new();
        if let Some((at, raw_tools)) = raw.deps.tools {
            tools = raw_tools
                .into_iter()
                .map(|tool| tool.resolve(yaml))
                .collect::<Result<Vec<_>>>()?;
            let mise = match scope {
                Scope::Global => Mise::global(tools.clone()),
                Scope::Project => {
                    let at = mise::PROJECT_CONFIG.to_string();
                    let i = env.partition_point(|(name, _)| name.as_str() < mise::CONFIG_FILENAME_VAR);
                    env.insert(i, (mise::CONFIG_FILENAME_VAR.to_string(), at));
                    Mise::project(root.unwrap_or(Path::new(".")), tools.clone())
                }
            };
            deps.insert(at, Box::new(mise));
        }
        if let (Some(root), Scope::Project) = (root, scope) {
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
            name,
            env,
            commands: raw.commands,
            deps,
            tools,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProject {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    commands: BTreeMap<String, ProjectCommand>,
    #[serde(default)]
    deps: Deps,
}

/// The `deps` map, turned into dependencies by key as it's read. Packages
/// and tools depend on where the file is, so only their settings are kept
/// here.
#[derive(Default)]
struct Deps {
    deps: Vec<Box<dyn Dependency>>,
    /// The tools, and the index in `deps` their one dependency goes at.
    tools: Option<(usize, Vec<RawTool>)>,
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
        let mut tools: Vec<RawTool> = Vec::new();
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
                    let value: Spanned<ToolValue> = map.next_value()?;
                    tools_at.get_or_insert(deps.len());
                    tools.push(RawTool::new(key, value).map_err(de::Error::custom)?);
                }
            }
        }

        // The tools dep goes in first, at `tools_at`, moving a later
        // `packages` along.
        if tools_at.is_some()
            && packages_after_tools
            && let PackagesSetting::Only { at: packages_at, .. } = &mut packages
        {
            *packages_at += 1;
        }

        let tools = tools_at.map(|at| (at, tools));
        Ok(Deps { deps, tools, packages })
    }
}

/// A tool as written under `deps`, before plain scalars are read back from
/// the source. See [`Scalar`].
struct RawTool {
    name: String,
    version: RawScalar,
    options: Vec<(String, RawScalar)>,
}

/// A tool's version or option value: as text, or where it is in the source.
enum RawScalar {
    Text(String),
    /// A plain number or bool: its byte offset and length in the source.
    At(usize, usize),
}

impl RawTool {
    fn new(name: String, value: Spanned<ToolValue>) -> std::result::Result<Self, String> {
        let Spanned { value, defined, .. } = value;
        let (version, options) = match value {
            ToolValue::Version(scalar) => (raw(&name, None, scalar, &defined)?, Vec::new()),
            ToolValue::Options { version, options } => {
                let version = version.ok_or_else(|| {
                    format!(
                        "tool `{name}` needs a `version` next to its options, e.g. `{name}: {{ version: latest, ... }}`"
                    )
                })?;
                let version = raw(&name, None, version.value, &version.defined)?;
                let options = options
                    .into_iter()
                    .map(|(key, value)| {
                        let scalar = raw(&name, Some(&key), value.value, &value.defined)?;
                        Ok((key, scalar))
                    })
                    .collect::<std::result::Result<_, String>>()?;
                (version, options)
            }
        };
        Ok(Self { name, version, options })
    }

    /// The tool, with plain scalars read back from `yaml`, the document it
    /// was parsed from.
    fn resolve(self, yaml: &str) -> Result<ToolSpec> {
        let text = |scalar: RawScalar| match scalar {
            RawScalar::Text(text) => Ok(text),
            RawScalar::At(start, len) => yaml
                .get(start..start + len)
                .map(str::to_string)
                .ok_or_else(|| eyre!("tool `{}` has a value dev couldn't find in dev.yaml", self.name)),
        };
        let mut tool = ToolSpec::new(self.name.clone(), text(self.version)?);
        for (key, value) in self.options {
            let value = text(value)?;
            // mise takes options as `name[k=v,...]@version` when installing.
            if [&key, &value].iter().any(|s| s.contains(['[', ']', ',', '='])) {
                bail!("options of tool `{}` can't contain `[`, `]`, `,`, or `=`", tool.name);
            }
            tool = tool.with_option(key, value);
        }
        Ok(tool)
    }
}

/// `scalar`, defined at `at`, as a [`RawScalar`]. `key` is the option it's
/// for, or `None` for the version.
fn raw(name: &str, key: Option<&str>, scalar: Scalar, at: &Location) -> std::result::Result<RawScalar, String> {
    match scalar {
        Scalar::Text(text) => Ok(RawScalar::Text(text)),
        Scalar::Plain => {
            let span = at.span();
            match (span.byte_offset(), span.byte_len()) {
                (Some(start), Some(len)) => Ok(RawScalar::At(start as usize, len as usize)),
                _ => Err(format!("tool `{name}` has a value dev couldn't find in dev.yaml")),
            }
        }
        Scalar::Nested => match key {
            Some(key) => Err(format!(
                "option `{key}` of tool `{name}` must be a string, number, or bool"
            )),
            None => Err(format!("the version of tool `{name}` must be a string or number")),
        },
    }
}

/// A tool's value under `deps`: a version, or a map with a version and
/// options, e.g. `aws: { version: "2.37", symlink_bins: true }`.
enum ToolValue {
    Version(Scalar),
    Options {
        version: Option<Spanned<Scalar>>,
        options: Vec<(String, Spanned<Scalar>)>,
    },
}

/// One scalar of a [`ToolValue`].
///
/// serde_saphyr only says what kind of node a value is by converting it, so
/// a plain `3.10` arrives as the number 3.1. Those are marked [`Scalar::Plain`]
/// and read back from the source by their span, so `python: 3.10` means
/// `3.10`.
enum Scalar {
    Text(String),
    Plain,
    /// A list or map, which isn't allowed here.
    Nested,
}

impl<'de> Deserialize<'de> for ToolValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(ToolValueVisitor)
    }
}

struct ToolValueVisitor;

impl<'de> Visitor<'de> for ToolValueVisitor {
    type Value = ToolValue;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a version like `lts`, or a map like `{ version: \"2.37\", symlink_bins: true }`")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<ToolValue, E> {
        Ok(ToolValue::Version(Scalar::Text(v.to_string())))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<ToolValue, E> {
        Ok(ToolValue::Version(Scalar::Plain))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<ToolValue, E> {
        Ok(ToolValue::Version(Scalar::Plain))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<ToolValue, E> {
        Ok(ToolValue::Version(Scalar::Plain))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<ToolValue, E> {
        Ok(ToolValue::Version(Scalar::Plain))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<ToolValue, A::Error> {
        let mut version = None;
        let mut options = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let value: Spanned<Scalar> = map.next_value()?;
            if key == "version" {
                version = Some(value);
            } else {
                options.push((key, value));
            }
        }
        Ok(ToolValue::Options { version, options })
    }
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(ScalarVisitor)
    }
}

struct ScalarVisitor;

impl<'de> Visitor<'de> for ScalarVisitor {
    type Value = Scalar;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a string, number, or bool")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Scalar, E> {
        Ok(Scalar::Text(v.to_string()))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<Scalar, E> {
        Ok(Scalar::Plain)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<Scalar, E> {
        Ok(Scalar::Plain)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<Scalar, E> {
        Ok(Scalar::Plain)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<Scalar, E> {
        Ok(Scalar::Plain)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Scalar, A::Error> {
        while map.next_entry::<de::IgnoredAny, de::IgnoredAny>()?.is_some() {}
        Ok(Scalar::Nested)
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Scalar, A::Error> {
        while seq.next_element::<de::IgnoredAny>()?.is_some() {}
        Ok(Scalar::Nested)
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
