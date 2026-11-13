use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use dev::{
    Result,
    new::{Templates, dev_ref},
    project::Project,
};
use tempfile::TempDir;

/// Writes `files` (relative path, contents) under `dir`.
fn write_tree(dir: &Path, files: &[(&str, &[u8])]) -> Result<()> {
    for (path, contents) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().expect("files have a parent"))?;
        fs::write(path, contents)?;
    }
    Ok(())
}

#[test]
fn create_renders_the_name_in_contents_and_paths() -> Result<()> {
    let tmp = TempDir::new()?;
    let user = tmp.path().join("templates");
    write_tree(
        &user.join("t"),
        &[
            ("dev.yaml", b"name: {{name}}\n"),
            ("cmd/{{name}}/main.go", b"// {{name}} and {{name}}\n"),
        ],
    )?;

    let dest = tmp.path().join("my-app");
    Templates::new(&user).find("t")?.create("my-app", &dest)?;

    assert_eq!(fs::read_to_string(dest.join("dev.yaml"))?, "name: my-app\n");
    assert_eq!(
        fs::read_to_string(dest.join("cmd/my-app/main.go"))?,
        "// my-app and my-app\n"
    );
    Ok(())
}

#[test]
fn create_returns_the_rendered_notes_instead_of_copying_them() -> Result<()> {
    let tmp = TempDir::new()?;
    let user = tmp.path().join("templates");
    write_tree(
        &user.join("t"),
        &[
            ("dev.yaml", b"name: {{name}}\n"),
            (".dev-notes.txt", b"Next: cd {{name}}\n"),
            // Only the root's are notes.
            ("docs/.dev-notes.txt", b"{{name}}\n"),
        ],
    )?;

    let dest = tmp.path().join("my-app");
    let notes = Templates::new(&user).find("t")?.create("my-app", &dest)?;

    assert_eq!(notes.as_deref(), Some("Next: cd my-app\n"));
    assert!(!dest.join(".dev-notes.txt").exists());
    assert_eq!(fs::read_to_string(dest.join("docs/.dev-notes.txt"))?, "my-app\n");
    Ok(())
}

#[test]
fn notes_that_arent_utf8_are_rejected() -> Result<()> {
    let tmp = TempDir::new()?;
    let user = tmp.path().join("templates");
    write_tree(&user.join("t"), &[(".dev-notes.txt", &[0xff, 0xfe])])?;

    let err = Templates::new(&user).find("t").err().expect("non-UTF-8 notes");
    assert!(
        err.to_string().ends_with("t/.dev-notes.txt isn't UTF-8"),
        "unexpected error: {err}"
    );
    Ok(())
}

#[test]
fn create_copies_binary_files_as_they_are() -> Result<()> {
    let tmp = TempDir::new()?;
    let user = tmp.path().join("templates");
    // Not UTF-8, with a placeholder inside that must survive.
    let bytes: &[u8] = b"\xff\xfe{{name}}\x00";
    write_tree(&user.join("t"), &[("logo.bin", bytes)])?;

    let dest = tmp.path().join("my-app");
    Templates::new(&user).find("t")?.create("my-app", &dest)?;

    assert_eq!(fs::read(dest.join("logo.bin"))?, bytes);
    Ok(())
}

#[test]
fn create_keeps_user_scripts_executable() -> Result<()> {
    let tmp = TempDir::new()?;
    let user = tmp.path().join("templates");
    write_tree(&user.join("t"), &[("bin/setup", b"#!/bin/sh\n"), ("README", b"hi\n")])?;
    fs::set_permissions(user.join("t/bin/setup"), fs::Permissions::from_mode(0o755))?;

    let dest = tmp.path().join("my-app");
    Templates::new(&user).find("t")?.create("my-app", &dest)?;

    let mode = |p: &str| fs::metadata(dest.join(p)).map(|m| m.permissions().mode() & 0o777);
    assert_eq!(mode("bin/setup")?, 0o755);
    assert_eq!(mode("README")? & 0o111, 0, "README must not be executable");
    Ok(())
}

#[test]
fn create_fails_when_dest_exists() -> Result<()> {
    let tmp = TempDir::new()?;
    let user = tmp.path().join("templates");
    write_tree(&user.join("t"), &[("dev.yaml", b"name: {{name}}\n")])?;
    let dest = tmp.path().join("my-app");
    write_tree(&dest, &[("dev.yaml", b"mine\n")])?;

    let err = Templates::new(&user).find("t")?.create("my-app", &dest).unwrap_err();

    assert!(format!("{err:#}").contains("failed to create"), "{err:#}");
    assert_eq!(fs::read_to_string(dest.join("dev.yaml"))?, "mine\n");
    Ok(())
}

#[test]
fn create_rejects_invalid_names_before_writing() -> Result<()> {
    let tmp = TempDir::new()?;
    let user = tmp.path().join("templates");
    write_tree(&user.join("t"), &[("dev.yaml", b"name: {{name}}\n")])?;
    let dest = tmp.path().join("My_App");

    let err = Templates::new(&user).find("t")?.create("My_App", &dest).unwrap_err();

    assert!(
        format!("{err:#}").contains("lowercase words separated by dashes"),
        "{err:#}"
    );
    assert!(!dest.exists());
    Ok(())
}

#[test]
fn user_templates_override_built_ins_and_are_listed() -> Result<()> {
    let tmp = TempDir::new()?;
    let user = tmp.path().join("templates");
    write_tree(&user.join("go"), &[("custom", b"mine\n")])?;
    write_tree(&user.join("zig"), &[("build.zig", b"\n")])?;
    let templates = Templates::new(&user);

    assert_eq!(templates.names()?, ["go", "node", "python", "ruby", "rust", "zig"]);

    let dest = tmp.path().join("my-app");
    templates.find("go")?.create("my-app", &dest)?;
    assert!(dest.join("custom").exists());
    assert!(!dest.join("main.go").exists(), "the built-in go template was used");
    Ok(())
}

#[test]
fn unknown_template_lists_the_available_ones() -> Result<()> {
    let tmp = TempDir::new()?;
    // No user templates dir at all.
    let templates = Templates::new(tmp.path().join("missing"));

    let err = templates.find("cobol").err().expect("cobol isn't a template");

    assert_eq!(
        err.to_string(),
        "no template named `cobol`. Available: go, node, python, ruby, rust"
    );
    Ok(())
}

#[test]
fn dev_ref_pins_a_release_build_to_its_commit() {
    assert_eq!(dev_ref("1.2.3", Some("abc123")), "abc123 # v1.2.3");
}

#[test]
fn dev_ref_uses_the_tag_without_a_release_commit() {
    assert_eq!(dev_ref("1.2.3", None), "v1.2.3");
}

#[test]
fn built_in_templates_render_valid_projects() -> Result<()> {
    let tmp = TempDir::new()?;
    let templates = Templates::new(tmp.path().join("missing"));

    for name in templates.names()? {
        let dest = tmp.path().join(&name);
        let notes = templates.find(&name)?.create("my-app", &dest)?.unwrap_or_default();
        assert!(
            !notes.contains("{{"),
            "template {name}'s notes left a placeholder:\n{notes}"
        );

        let project = Project::load(dest.join("dev.yaml"))?;
        assert_eq!(project.name, "my-app", "template {name}");
        assert!(dest.join(".gitignore").exists(), "template {name} has no .gitignore");
        assert!(
            dest.join(".github/workflows/ci.yaml").exists(),
            "template {name} has no CI workflow"
        );

        let leftover = walk(&dest)?.into_iter().find(|(path, text)| {
            ["{{name}}", "{{dev_version}}"]
                .iter()
                .any(|p| path.to_string_lossy().contains(p) || text.contains(p))
        });
        assert_eq!(leftover, None, "template {name} left a placeholder");

        // The same build of dev rendered it, so the same ref.
        let ci = fs::read_to_string(dest.join(".github/workflows/ci.yaml"))?;
        let uses = format!(
            "uses: pseudomuto/dev@{}\n",
            dev_ref(env!("CARGO_PKG_VERSION"), option_env!("DEV_RELEASE_SHA"))
        );
        assert!(ci.contains(&uses), "template {name}'s CI doesn't have `{uses}`:\n{ci}");
    }
    Ok(())
}

/// Every file under `dir`, with its contents as (lossy) text.
fn walk(dir: &Path) -> Result<Vec<(std::path::PathBuf, String)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            files.extend(walk(&path)?);
        } else {
            let text = String::from_utf8_lossy(&fs::read(&path)?).into_owned();
            files.push((path, text));
        }
    }
    Ok(files)
}
