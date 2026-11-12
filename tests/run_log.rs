use std::{fs, io::Write};

use dev::{Result, run_log::RunLog};
use tempfile::tempdir;

#[test]
fn create_makes_the_log_a_gitignore_and_latest() -> Result<()> {
    let root = tempdir()?;
    let mut log = RunLog::create(root.path(), "up", 5)?;
    writeln!(log.file, "hello")?;

    let name = log.path.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(root.path().join(".dev/log"), log.path.parent().unwrap());
    assert!(
        name.ends_with(&format!("-{}-up.log", std::process::id())),
        "name: {name}"
    );
    assert_eq!("*\n", fs::read_to_string(root.path().join(".dev/.gitignore"))?);
    assert_eq!(
        name,
        fs::read_link(root.path().join(".dev/log/latest"))?.to_string_lossy()
    );
    assert_eq!("hello\n", fs::read_to_string(root.path().join(".dev/log/latest"))?);
    Ok(())
}

#[test]
fn create_keeps_only_the_newest_logs_and_nothing_else_is_touched() -> Result<()> {
    let root = tempdir()?;
    let dir = root.path().join(".dev/log");
    fs::create_dir_all(&dir)?;
    // Older runs, named the way `create` names them.
    for n in 1..=6 {
        fs::write(dir.join(format!("{n:013}-1-up.log")), "")?;
    }
    fs::write(dir.join("notes.txt"), "mine")?;
    fs::write(root.path().join(".dev/.gitignore"), "custom\n")?;

    let log = RunLog::create(root.path(), "down", 5)?;

    let mut logs: Vec<String> = fs::read_dir(&dir)?
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".log"))
        .collect();
    logs.sort();
    let newest = log.path.file_name().unwrap().to_string_lossy().into_owned();
    let expected: Vec<String> = (3..=6).map(|n| format!("{n:013}-1-up.log")).chain([newest]).collect();
    assert_eq!(expected, logs);
    assert_eq!("mine", fs::read_to_string(dir.join("notes.txt"))?);
    assert_eq!("custom\n", fs::read_to_string(root.path().join(".dev/.gitignore"))?);
    Ok(())
}

#[test]
fn create_moves_latest_to_the_new_log() -> Result<()> {
    let root = tempdir()?;
    RunLog::create(root.path(), "up", 5)?;
    // Names are per millisecond; one process only makes one log per run.
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second = RunLog::create(root.path(), "up", 5)?;

    assert_eq!(
        second.path.file_name().unwrap(),
        fs::read_link(root.path().join(".dev/log/latest"))?
    );
    Ok(())
}
