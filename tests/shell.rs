use std::fs;

use dev::{Result, shell::ShellDir};
use tempfile::tempdir;

#[test]
fn write_creates_the_dir_file_and_stamp() -> Result<()> {
    let tmp = tempdir()?;
    let dir = ShellDir::new(tmp.path().join("dev/shell"));

    dir.write("zerobrew", "export A=1\n")?;
    assert_eq!("export A=1\n", fs::read_to_string(dir.file("zerobrew"))?);
    assert!(!dir.stamp()?.is_empty(), "a stamp should be written");
    Ok(())
}

#[test]
fn write_replaces_the_file_and_changes_the_stamp() -> Result<()> {
    let tmp = tempdir()?;
    let dir = ShellDir::new(tmp.path());

    dir.write("zerobrew", "old\n")?;
    let first = dir.stamp()?;
    dir.write("zerobrew", "new\n")?;

    assert_eq!("new\n", fs::read_to_string(dir.file("zerobrew"))?);
    assert_ne!(first, dir.stamp()?, "the stamp should change on every write");
    Ok(())
}

#[test]
fn write_leaves_no_temp_files_behind() -> Result<()> {
    let tmp = tempdir()?;
    let dir = ShellDir::new(tmp.path());

    dir.write("zerobrew", "x\n")?;
    let mut names: Vec<String> = fs::read_dir(tmp.path())?
        .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_>>()?;
    names.sort();

    assert_eq!(vec![".stamp", "zerobrew"], names);
    Ok(())
}
