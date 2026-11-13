//! Project templates for `dev new`.
//!
//! Built-in templates live in `share/templates/<name>/` and are compiled
//! into the binary, so a new release brings new templates. User templates
//! live in `$XDG_CONFIG_HOME/dev/templates/<name>/`
//! (`~/.config/dev/templates`) and win over a built-in with the same name.
//!
//! Creating a project copies the template's files, replacing `{{name}}`
//! with the project's name and `{{dev_version}}` with the ref to pin the dev
//! GitHub action to (see [`dev_ref`]), in every path and in every file
//! that's UTF-8.

use std::{
    borrow::Cow,
    collections::BTreeSet,
    fs,
    io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{WrapErr, bail};
use include_dir::{Dir, include_dir};

use crate::{Context, Result, process, project::validate_name, shell::config_home};

/// The placeholder replaced with the project's name.
const PLACEHOLDER: &str = "{{name}}";

/// The placeholder replaced with the ref to pin `pseudomuto/dev` to: see
/// [`dev_ref`].
const DEV_VERSION_PLACEHOLDER: &str = "{{dev_version}}";

/// The ref to use `pseudomuto/dev` (the GitHub action) at, for dev `version`.
/// Release builds know their commit (`DEV_RELEASE_SHA`, set by the release
/// workflow) and pin to it, with the version as a comment. Other builds use
/// the version's tag: Cargo.toml only changes at release, so it exists.
pub fn dev_ref(version: &str, sha: Option<&str>) -> String {
    match sha {
        Some(sha) => format!("{sha} # v{version}"),
        None => format!("v{version}"),
    }
}

static BUILT_IN: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/share/templates");

/// Where templates are looked up: user templates first, then built-in ones.
pub struct Templates {
    user_dir: PathBuf,
}

impl Templates {
    /// Uses `user_dir` for user templates, one directory per template.
    pub fn new(user_dir: impl Into<PathBuf>) -> Self {
        Self {
            user_dir: user_dir.into(),
        }
    }

    /// `$XDG_CONFIG_HOME/dev/templates`, or `~/.config/dev/templates`.
    pub fn user_default() -> Result<Self> {
        Ok(Self::new(config_home()?.join("dev/templates")))
    }

    /// Every template's name, user and built-in, sorted.
    pub fn names(&self) -> Result<Vec<String>> {
        let mut names: BTreeSet<String> = BUILT_IN
            .dirs()
            .filter_map(|d| d.path().file_name()?.to_str().map(String::from))
            .collect();

        match fs::read_dir(&self.user_dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.wrap_err_with(|| format!("failed to read {}", self.user_dir.display()))?;
                    if entry.path().is_dir()
                        && let Some(name) = entry.file_name().to_str()
                    {
                        names.insert(name.to_string());
                    }
                }
            }
            // No user templates is fine.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).wrap_err_with(|| format!("failed to read {}", self.user_dir.display())),
        }
        Ok(names.into_iter().collect())
    }

    /// The template called `name`. A user template wins over a built-in one.
    pub fn find(&self, name: &str) -> Result<Template> {
        let user = self.user_dir.join(name);
        if user.is_dir() {
            return Template::from_dir(&user);
        }
        if let Some(dir) = BUILT_IN.get_dir(name) {
            return Template::built_in(dir);
        }
        bail!("no template named `{name}`. Available: {}", self.names()?.join(", "));
    }
}

/// The file at a template's root that's printed after `dev new` instead of
/// copied, e.g. to say what to change before the first commit.
const NOTES_FILE: &str = ".dev-notes.txt";

/// A template's files, ready to write.
pub struct Template {
    files: Vec<TemplateFile>,
    /// [`NOTES_FILE`]'s contents, not yet rendered.
    notes: Option<String>,
}

struct TemplateFile {
    /// Relative to the template's root.
    path: PathBuf,
    contents: Cow<'static, [u8]>,
    executable: bool,
}

impl Template {
    fn built_in(dir: &'static Dir<'static>) -> Result<Self> {
        let mut files = Vec::new();
        collect_built_in(dir, dir.path(), &mut files);
        Self::from_files(files, dir.path())
    }

    fn from_dir(root: &Path) -> Result<Self> {
        let mut files = Vec::new();
        collect_dir(root, root, &mut files)?;
        Self::from_files(files, root)
    }

    /// Takes the notes out of `files`, which came from the template at `root`.
    fn from_files(mut files: Vec<TemplateFile>, root: &Path) -> Result<Self> {
        let notes = match files.iter().position(|f| f.path == Path::new(NOTES_FILE)) {
            Some(i) => {
                let contents = files.remove(i).contents.into_owned();
                let text = String::from_utf8(contents)
                    .wrap_err_with(|| format!("{} isn't UTF-8", root.join(NOTES_FILE).display()))?;
                Some(text)
            }
            None => None,
        };
        Ok(Self { files, notes })
    }

    /// Creates `dest` and writes the template into it for project `name`,
    /// returning its notes (see [`NOTES_FILE`]), rendered, if it has any.
    /// Fails without writing anything if `name` isn't a valid project name
    /// or `dest` already exists.
    pub fn create(&self, name: &str, dest: &Path) -> Result<Option<String>> {
        validate_name(name)?;
        fs::create_dir(dest).wrap_err_with(|| format!("failed to create {}", dest.display()))?;

        let dev = dev_ref(env!("CARGO_PKG_VERSION"), option_env!("DEV_RELEASE_SHA"));
        let render = |text: &str| text.replace(PLACEHOLDER, name).replace(DEV_VERSION_PLACEHOLDER, &dev);

        for file in &self.files {
            let rel = file.path.to_str().map(render);
            let path = match rel {
                Some(rel) => dest.join(rel),
                None => dest.join(&file.path),
            };
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).wrap_err_with(|| format!("failed to create {}", parent.display()))?;
            }

            let contents = match std::str::from_utf8(&file.contents) {
                Ok(text) => Cow::Owned(render(text).into_bytes()),
                Err(_) => Cow::Borrowed(file.contents.as_ref()),
            };
            fs::write(&path, &contents).wrap_err_with(|| format!("failed to write {}", path.display()))?;

            if file.executable {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                    .wrap_err_with(|| format!("failed to make {} executable", path.display()))?;
            }
        }
        Ok(self.notes.as_deref().map(render))
    }
}

fn collect_built_in(dir: &'static Dir<'static>, root: &Path, files: &mut Vec<TemplateFile>) {
    for file in dir.files() {
        files.push(TemplateFile {
            path: file
                .path()
                .strip_prefix(root)
                .expect("entries are under their dir")
                .into(),
            contents: Cow::Borrowed(file.contents()),
            executable: false,
        });
    }
    for sub in dir.dirs() {
        collect_built_in(sub, root, files);
    }
}

fn collect_dir(dir: &Path, root: &Path, files: &mut Vec<TemplateFile>) -> Result<()> {
    let entries = fs::read_dir(dir).wrap_err_with(|| format!("failed to read {}", dir.display()))?;
    for entry in entries {
        let path = entry
            .wrap_err_with(|| format!("failed to read {}", dir.display()))?
            .path();
        let meta = fs::metadata(&path).wrap_err_with(|| format!("failed to read {}", path.display()))?;

        if meta.is_dir() {
            collect_dir(&path, root, files)?;
        } else {
            let contents = fs::read(&path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
            files.push(TemplateFile {
                path: path.strip_prefix(root).expect("entries are under the root").into(),
                contents: Cow::Owned(contents),
                executable: meta.permissions().mode() & 0o111 != 0,
            });
        }
    }
    Ok(())
}

/// Runs `git init` in `dir`.
pub fn git_init(ctx: &Context, dir: &Path) -> Result<()> {
    process::output(
        ctx.command("git").args(["init", "--quiet"]).current_dir(dir),
        "git init",
    )?;
    Ok(())
}
