use std::{collections::BTreeMap, path::PathBuf};

use dev::{
    Result,
    layout::{Host, Layout},
};

const LINUX: Host = Host {
    macos: false,
    opt_zerobrew: false,
};

/// An env with `HOME=/home/me` and `vars` on top.
fn env(vars: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([("HOME".to_string(), "/home/me".to_string())]);
    env.extend(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    env
}

/// A path in a [`Layout`].
type Field = fn(&Layout) -> &PathBuf;

fn layout(vars: &[(&str, &str)], host: Host) -> Result<Layout> {
    Layout::from_env(&env(vars), host)
}

#[test]
fn defaults_follow_home() -> Result<()> {
    let l = layout(&[], LINUX)?;
    assert_eq!(PathBuf::from("/home/me/.local/share/dev/shell"), l.shell_dir);
    assert_eq!(PathBuf::from("/home/me/.local/bin/mise"), l.mise_bin);
    assert_eq!(PathBuf::from("/home/me/.local/share/mise"), l.mise_data);
    assert_eq!(PathBuf::from("/home/me/.cargo"), l.cargo_home);
    assert_eq!(PathBuf::from("/home/me/.rustup"), l.rustup_home);
    assert_eq!(PathBuf::from("/home/me/.local/bin"), l.zb_bin);
    assert_eq!(PathBuf::from("/home/me/.local/share/zerobrew"), l.zerobrew_root);
    Ok(())
}

#[test]
fn each_override_moves_its_path() -> Result<()> {
    let cases: [(&str, Field, &str); 7] = [
        ("XDG_DATA_HOME", |l| &l.shell_dir, "/x/dev/shell"),
        ("MISE_INSTALL_PATH", |l| &l.mise_bin, "/x"),
        ("MISE_DATA_DIR", |l| &l.mise_data, "/x"),
        ("CARGO_HOME", |l| &l.cargo_home, "/x"),
        ("RUSTUP_HOME", |l| &l.rustup_home, "/x"),
        ("ZEROBREW_BIN", |l| &l.zb_bin, "/x"),
        ("ZEROBREW_ROOT", |l| &l.zerobrew_root, "/x"),
    ];
    for (var, field, want) in cases {
        let l = layout(&[(var, "/x")], LINUX)?;
        assert_eq!(&PathBuf::from(want), field(&l), "{var}");

        let unset = layout(&[(var, "")], LINUX)?;
        let default = layout(&[], LINUX)?;
        assert_eq!(field(&default), field(&unset), "an empty {var} counts as unset");
    }
    Ok(())
}

#[test]
fn xdg_data_home_also_moves_mise_and_zerobrew_data() -> Result<()> {
    let l = layout(&[("XDG_DATA_HOME", "/data")], LINUX)?;
    assert_eq!(PathBuf::from("/data/mise"), l.mise_data);
    assert_eq!(PathBuf::from("/data/zerobrew"), l.zerobrew_root);
    Ok(())
}

#[test]
fn zerobrew_uses_opt_on_macos_or_when_it_exists() -> Result<()> {
    let opt = PathBuf::from("/opt/zerobrew");
    let macos = Host {
        macos: true,
        opt_zerobrew: false,
    };
    let linux_with_opt = Host {
        macos: false,
        opt_zerobrew: true,
    };
    assert_eq!(opt, layout(&[], macos)?.zerobrew_root);
    assert_eq!(opt, layout(&[], linux_with_opt)?.zerobrew_root);
    assert_eq!(
        PathBuf::from("/x"),
        layout(&[("ZEROBREW_ROOT", "/x")], macos)?.zerobrew_root,
        "ZEROBREW_ROOT wins everywhere"
    );
    Ok(())
}

#[test]
fn home_is_needed_for_the_defaults() {
    let err = Layout::from_env(&BTreeMap::new(), LINUX).expect_err("no HOME, no defaults");
    assert_eq!("HOME is not set", err.to_string());
}
