use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use dev::{Result, shell::env::hook};
use tempfile::TempDir;

/// A fake `$HOME` and a shell env that lives in it.
struct Shell {
    home: TempDir,
    env: BTreeMap<String, String>,
}

impl Shell {
    fn new() -> Result<Self> {
        let home = tempfile::tempdir()?;
        let env = BTreeMap::from([
            ("HOME".to_string(), path_str(home.path())),
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
        ]);
        Ok(Self { home, env })
    }

    /// `$HOME/<rel>`.
    fn path(&self, rel: &str) -> PathBuf {
        self.home.path().join(rel)
    }

    /// Writes `yaml` to `$HOME/<rel>/dev.yaml`.
    fn project(&self, rel: &str, yaml: &str) -> Result<()> {
        let dir = self.path(rel);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("dev.yaml"), yaml)?;
        Ok(())
    }

    /// `cd`s to `dir`, then runs the prompt hook like `share/shell` does.
    /// Returns the script it evaluated, if any.
    fn cd(&mut self, dir: &Path) -> Result<Option<String>> {
        fs::create_dir_all(dir)?;
        self.env.insert("PWD".to_string(), path_str(dir));

        let script = hook(&self.env)?;
        if let Some(script) = &script {
            self.env = eval(&self.env, script)?;
            self.env.insert("PWD".to_string(), path_str(dir));
        }
        Ok(script)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.env.get(name).map(String::as_str)
    }
}

fn path_str(path: &Path) -> String {
    path.to_str().expect("temp paths are UTF-8").to_string()
}

/// Evals `script` in a real `sh` with exactly `env`, and returns the env
/// after, minus what `sh` adds itself.
#[allow(clippy::disallowed_methods)]
fn eval(env: &BTreeMap<String, String>, script: &str) -> Result<BTreeMap<String, String>> {
    let output = Command::new("/bin/sh")
        .env_clear()
        .envs(env)
        .arg("-c")
        .arg(format!("{script}\nexec /usr/bin/env -0"))
        .output()?;
    assert!(
        output.status.success(),
        "script failed: {}\n{script}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(name, _)| !matches!(*name, "SHLVL" | "_" | "OLDPWD" | "PWD"))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect())
}

#[test]
fn entering_a_project_exports_its_env_and_leaving_restores_it() -> Result<()> {
    let mut sh = Shell::new()?;
    sh.env.insert("GREETING".to_string(), "before".to_string());
    sh.project("proj", "name: proj\nenv:\n  GREETING: hi\n  NEW: \"${PWD}/x\"\n")?;

    sh.cd(&sh.path("proj"))?;
    assert_eq!(Some("hi"), sh.get("GREETING"));
    let proj = sh.path("proj");
    assert_eq!(Some(format!("{}/x", proj.display()).as_str()), sh.get("NEW"));

    sh.cd(&sh.path("elsewhere"))?;
    assert_eq!(Some("before"), sh.get("GREETING"), "a set var gets its old value back");
    assert_eq!(None, sh.get("NEW"), "a var that was unset is unset again");
    assert_eq!(None, sh.get("__DEV_ENV"), "no state is left behind");
    Ok(())
}

#[test]
fn nested_projects_layer_outermost_first_and_back_out_cleanly() -> Result<()> {
    let mut sh = Shell::new()?;
    sh.project(
        "proj",
        "name: proj\nenv:\n  PATH: \"${PWD}/bin:${PATH}\"\n  WHO: outer\n",
    )?;
    sh.project(
        "proj/sub",
        "name: sub\nenv:\n  PATH: \"${PWD}/bin:${PATH}\"\n  WHO: inner\n",
    )?;
    let (proj, sub) = (sh.path("proj"), sh.path("proj/sub"));

    sh.cd(&sub.join("deeper"))?;
    let both = format!("{}/bin:{}/bin:/usr/bin:/bin", sub.display(), proj.display());
    assert_eq!(Some(both.as_str()), sh.get("PATH"), "inner sees the outer PATH");
    assert_eq!(Some("inner"), sh.get("WHO"), "inner overrides outer");

    sh.cd(&proj)?;
    let outer = format!("{}/bin:/usr/bin:/bin", proj.display());
    assert_eq!(Some(outer.as_str()), sh.get("PATH"));
    assert_eq!(Some("outer"), sh.get("WHO"));

    let home = sh.home.path().to_path_buf();
    sh.cd(&home)?;
    assert_eq!(
        Some("/usr/bin:/bin"),
        sh.get("PATH"),
        "$HOME itself is outside every project"
    );
    assert_eq!(None, sh.get("WHO"));
    Ok(())
}

#[test]
fn reloads_only_when_a_dev_yaml_changes_and_never_grows_path() -> Result<()> {
    let mut sh = Shell::new()?;
    sh.project("proj", "name: proj\nenv:\n  PATH: \"${PWD}/bin:${PATH}\"\n  V: one\n")?;
    let proj = sh.path("proj");

    assert!(sh.cd(&proj)?.is_some(), "entering loads");
    assert_eq!(None, sh.cd(&proj)?, "the next prompt has nothing to do");
    assert_eq!(
        None,
        sh.cd(&proj.join("src"))?,
        "a subdir with no dev.yaml changes nothing"
    );

    sh.project("proj", "name: proj\nenv:\n  PATH: \"${PWD}/bin:${PATH}\"\n  V: two\n")?;
    let file = fs::File::options().write(true).open(proj.join("dev.yaml"))?;
    file.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1))?;

    assert!(sh.cd(&proj)?.is_some(), "an edit reloads");
    assert_eq!(Some("two"), sh.get("V"));
    let path = format!("{}/bin:/usr/bin:/bin", proj.display());
    assert_eq!(
        Some(path.as_str()),
        sh.get("PATH"),
        "expanded from the baseline, not the loaded PATH"
    );
    Ok(())
}

#[test]
fn the_global_dev_yaml_is_the_outermost_layer_everywhere() -> Result<()> {
    let mut sh = Shell::new()?;
    sh.project(
        ".config",
        "name: me\nenv:\n  EDITOR: vim\n  WHO: global\n  AT: \"${PWD}\"\n",
    )?;
    sh.project("proj", "name: proj\nenv:\n  WHO: \"proj over ${WHO}\"\n")?;
    let home = sh.home.path().to_path_buf();

    sh.cd(&sh.path("proj"))?;
    assert_eq!(Some("vim"), sh.get("EDITOR"));
    assert_eq!(Some("proj over global"), sh.get("WHO"));
    assert_eq!(
        Some(path_str(&home).as_str()),
        sh.get("AT"),
        "${{PWD}} is $HOME in the global file"
    );

    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("dev.yaml"), "name: out\nenv:\n  WHO: outside\n")?;
    sh.cd(&outside.path().join("sub"))?;
    assert_eq!(Some("global"), sh.get("WHO"), "outside $HOME, parents aren't checked");

    sh.cd(outside.path())?;
    assert_eq!(
        Some("outside"),
        sh.get("WHO"),
        "outside $HOME, $PWD/dev.yaml still counts"
    );
    assert_eq!(Some("vim"), sh.get("EDITOR"));
    Ok(())
}

#[test]
fn xdg_config_home_moves_the_global_dev_yaml() -> Result<()> {
    let mut sh = Shell::new()?;
    sh.project(".config", "name: default\nenv:\n  WHO: default\n")?;
    sh.project("xdg", "name: xdg\nenv:\n  WHO: xdg\n")?;
    sh.env.insert("XDG_CONFIG_HOME".to_string(), path_str(&sh.path("xdg")));

    sh.cd(&sh.path("anywhere"))?;
    assert_eq!(Some("xdg"), sh.get("WHO"));
    Ok(())
}

#[test]
fn values_reach_the_shell_exactly_as_written() -> Result<()> {
    let mut sh = Shell::new()?;
    let tricky = "it's $HOME `whoami` \\ \"q\"\nline two";
    sh.env.insert("TRICKY".to_string(), tricky.to_string());
    sh.project("proj", "name: proj\nenv:\n  COPY: \"${TRICKY}\"\n")?;

    sh.cd(&sh.path("proj"))?;
    assert_eq!(Some(tricky), sh.get("COPY"));
    Ok(())
}

#[test]
fn names_that_sh_cant_export_fail() -> Result<()> {
    let mut sh = Shell::new()?;
    sh.project("proj", "name: proj\nenv:\n  \"BAD-NAME\": x\n")?;

    let err = sh.cd(&sh.path("proj")).expect_err("a bad name should fail");
    assert!(
        err.to_string().contains("env `BAD-NAME` in") && err.to_string().contains("isn't a valid shell variable name"),
        "{err}"
    );
    Ok(())
}

/// Sources `share/shell` in a real `sh` with the built `dev` on `PATH`.
#[test]
#[allow(clippy::disallowed_methods)]
fn share_shell_loads_and_unloads_project_env() -> Result<()> {
    let sh = Shell::new()?;
    sh.project(
        "proj",
        "name: proj\nenv:\n  GREETING: hi\n  PATH: \"${PWD}/bin:${PATH}\"\ncommands:\n  path: 'echo \"$PATH\"'\n",
    )?;
    let bin = Path::new(env!("CARGO_BIN_EXE_dev")).parent().expect("dev is in a dir");
    let share = Path::new(env!("CARGO_MANIFEST_DIR")).join("share/shell");

    let script = r#"
        . "$1"
        cd proj && __dev_hook
        echo "in: $GREETING"
        dev path
        cd .. && __dev_hook
        echo "out: ${GREETING-unset}"
    "#;
    let output = Command::new("/bin/sh")
        .env_clear()
        .env("HOME", sh.home.path())
        .env("PWD", sh.home.path())
        .env("XDG_DATA_HOME", sh.path("data"))
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .current_dir(sh.home.path())
        .args(["-c", script, "sh"])
        .arg(&share)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stderr: {stderr}");

    let proj = sh.path("proj");
    let expected = format!(
        "in: hi\n{}/bin:{}:/usr/bin:/bin\nout: unset\n",
        proj.display(),
        bin.display()
    );
    assert_eq!(expected, String::from_utf8_lossy(&output.stdout), "stderr: {stderr}");
    Ok(())
}
