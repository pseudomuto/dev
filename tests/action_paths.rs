use std::{collections::BTreeMap, path::Path};

use dev::{
    Result,
    action::{self, PathEntry},
    layout::{Host, Layout},
};

const LINUX: Host = Host {
    macos: false,
    opt_zerobrew: false,
};
const MACOS: Host = Host {
    macos: true,
    opt_zerobrew: false,
};

fn paths(host: Host, project: &Path) -> Result<action::Paths> {
    let env = BTreeMap::from([("HOME".to_string(), "/h".to_string())]);
    Ok(action::paths(
        &Layout::from_env(&env, host)?,
        host,
        Path::new("/h/.local/bin"),
        project,
    ))
}

#[test]
fn path_puts_mise_shims_first_then_setups_in_reverse_load_order_then_dev() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let p = paths(LINUX, dir.path())?;
    assert_eq!(
        vec![
            PathEntry::Dir("/h/.local/share/mise/shims".into()),
            PathEntry::Setup("/h/.local/share/dev/shell/rustup".into()),
            PathEntry::Setup("/h/.local/share/dev/shell/zerobrew".into()),
            PathEntry::Dir("/h/.local/bin".into()),
        ],
        p.path
    );
    assert_eq!(Path::new("/h/.local/bin/mise"), p.mise);
    Ok(())
}

#[test]
fn caches_what_dev_up_installs_with_its_exclusions() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let p = paths(LINUX, dir.path())?;
    assert_eq!(
        vec![
            "/h/.rustup/toolchains",
            "!/h/.rustup/toolchains/stable-*",
            "/h/.rustup/update-hashes",
            "/h/.local/bin/mise",
            "/h/.local/share/mise/installs",
            "/h/.local/share/mise/shims",
            "/h/.local/share/dev/shell",
            "!/h/.local/share/dev/shell/zerobrew",
        ],
        p.cache
    );
    assert_eq!(
        vec![
            "/h/.local/bin/zb",
            "/h/.local/bin/zb-brew",
            "/h/.local/bin/zbx",
            "/h/.local/share/zerobrew",
            "/h/.local/share/dev/shell/zerobrew",
        ],
        p.brew_cache
    );
    assert_eq!(Path::new("/h/.local/share/zerobrew"), p.brew_root);
    Ok(())
}

#[test]
fn macos_has_no_brew_cache() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let p = paths(MACOS, dir.path())?;
    assert!(p.brew_cache.is_empty(), "{:?}", p.brew_cache);
    assert_eq!(Path::new("/opt/zerobrew"), p.brew_root);
    Ok(())
}

#[test]
fn env_points_mise_at_the_project_config_only_when_there_is_one() -> Result<()> {
    let dir = tempfile::tempdir()?;
    assert!(paths(LINUX, dir.path())?.env.is_empty());

    std::fs::create_dir_all(dir.path().join(".dev"))?;
    std::fs::write(dir.path().join(".dev/mise.toml"), "")?;
    assert_eq!(
        BTreeMap::from([("MISE_DEFAULT_CONFIG_FILENAME".to_string(), ".dev/mise.toml".to_string())]),
        paths(LINUX, dir.path())?.env
    );
    Ok(())
}
