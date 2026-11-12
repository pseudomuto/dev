use std::{
    cell::RefCell,
    fmt::Display,
    fs,
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
    rc::Rc,
    sync::{Arc, Mutex},
};

use color_eyre::eyre::bail;
use dev::{
    Context,
    Result,
    deps::{Dependency, Task, meet_tasks},
    project::Project,
    report::{Plain, Reporter, Status},
};

#[derive(Clone, Default)]
pub struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Display for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0.lock().unwrap()))
    }
}

/// A context whose reporter (plain lines) and `log` write to buffers the
/// test can read, in that order. `err` goes to a buffer nobody reads.
// Each tests/*.rs file compiles this module separately, and not all of them use it.
#[allow(dead_code)]
pub fn capturing_ctx() -> (Context, Buffer, Buffer) {
    let (out, log) = (Buffer::default(), Buffer::default());
    let ctx = Context {
        reporter: Box::new(Plain::new(Box::new(out.clone()))),
        err: Box::new(Buffer::default()),
        log: Box::new(log.clone()),
        ..Context::default()
    };
    (ctx, out, log)
}

/// Shared record of calls made to fake tasks, e.g. `["check a", "apply a"]`.
#[allow(dead_code)]
pub type Log = Rc<RefCell<Vec<String>>>;

/// A task that records every call and behaves as configured.
#[allow(dead_code)]
pub struct FakeTask {
    name: String,
    applied: RefCell<bool>,
    fail_apply: bool,
    log: Log,
}

#[allow(dead_code)]
impl FakeTask {
    pub fn new(name: &str, applied: bool, log: &Log) -> Self {
        Self {
            name: name.into(),
            applied: RefCell::new(applied),
            fail_apply: false,
            log: Rc::clone(log),
        }
    }

    pub fn failing_apply(mut self) -> Self {
        self.fail_apply = true;
        self
    }
}

impl Task for FakeTask {
    fn name(&self) -> &str {
        &self.name
    }

    fn apply(&self, _ctx: &mut Context) -> Result<()> {
        self.log.borrow_mut().push(format!("apply {}", self.name));
        if self.fail_apply {
            bail!("boom");
        }
        *self.applied.borrow_mut() = true;
        Ok(())
    }

    fn is_applied(&self, _ctx: &mut Context) -> Result<bool> {
        self.log.borrow_mut().push(format!("check {}", self.name));
        Ok(*self.applied.borrow())
    }
}

/// A dependency that records every call and behaves as configured.
#[allow(dead_code)]
pub struct FakeDep {
    name: String,
    met: bool,
    fail_down: bool,
    tasks: Vec<Box<dyn Task>>,
    log: Log,
}

#[allow(dead_code)]
impl FakeDep {
    pub fn new(name: &str, met: bool, log: &Log) -> Self {
        Self {
            name: name.into(),
            met,
            fail_down: false,
            tasks: Vec::new(),
            log: Rc::clone(log),
        }
    }

    pub fn failing_down(mut self) -> Self {
        self.fail_down = true;
        self
    }

    pub fn with_tasks(mut self, tasks: Vec<FakeTask>) -> Self {
        self.tasks = tasks.into_iter().map(|t| Box::new(t) as Box<dyn Task>).collect();
        self
    }
}

impl Dependency for FakeDep {
    fn name(&self) -> &str {
        &self.name
    }

    fn desc(&self) -> &str {
        "a fake dependency"
    }

    fn tasks(&self) -> &[Box<dyn Task>] {
        &self.tasks
    }

    /// Logs the call, then meets its tasks the way real dependencies do.
    fn meet(&self, ctx: &mut Context) -> Result<()> {
        self.log.borrow_mut().push(format!("meet {}", self.name));
        meet_tasks(&self.tasks, ctx)
    }

    fn is_met(&self, _ctx: &mut Context) -> Result<bool> {
        self.log.borrow_mut().push(format!("check dep {}", self.name));
        Ok(self.met)
    }

    fn down(&self, _ctx: &mut Context) -> Result<()> {
        self.log.borrow_mut().push(format!("down {}", self.name));
        if self.fail_down {
            bail!("boom");
        }
        Ok(())
    }
}

/// A project named `demo` with `deps` and nothing else.
#[allow(dead_code)]
pub fn project(deps: Vec<FakeDep>) -> Project {
    Project {
        name: "demo".into(),
        env: Vec::new(),
        commands: Default::default(),
        deps: deps.into_iter().map(|d| Box::new(d) as Box<dyn Dependency>).collect(),
    }
}

/// Every call a [`Recorder`] got, e.g. `["0 a checking", "output hi"]`.
#[allow(dead_code)]
pub type Events = Arc<Mutex<Vec<String>>>;

/// A reporter that records every call into shared [`Events`].
#[allow(dead_code)]
pub struct Recorder(pub Events);

impl Reporter for Recorder {
    fn start(&mut self, command: &str, project: &str) -> Result<()> {
        self.0.lock().unwrap().push(format!("start {command} {project}"));
        Ok(())
    }

    fn step(&mut self, depth: usize, name: &str, status: Status) -> Result<()> {
        self.0.lock().unwrap().push(format!("{depth} {name} {status}"));
        Ok(())
    }

    fn output(&mut self, line: &str) -> Result<()> {
        self.0.lock().unwrap().push(format!("output {line}"));
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        self.0.lock().unwrap().push("pause".into());
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.0.lock().unwrap().push("resume".into());
        Ok(())
    }

    fn finish(&mut self, ok: bool) -> Result<()> {
        self.0.lock().unwrap().push(format!("finish {ok}"));
        Ok(())
    }

    fn error(&mut self, err: &color_eyre::Report, _log: &std::path::Path) -> Result<bool> {
        self.0.lock().unwrap().push(format!("error {err}"));
        Ok(false)
    }
}

/// A context that records reporter calls into the returned [`Events`].
#[allow(dead_code)]
pub fn recording_ctx() -> (Context, Events) {
    let events = Events::default();
    let ctx = Context {
        reporter: Box::new(Recorder(Arc::clone(&events))),
        err: Box::new(Buffer::default()),
        log: Box::new(Buffer::default()),
        ..Context::default()
    };
    (ctx, events)
}

/// Writes `contents` to an executable file at `path`, creating its dir.
///
/// The file is written by a short-lived `sh`, never by the test process. On
/// Linux, a file that's open for writing can't be run (`ETXTBSY`), and tests
/// run on parallel threads: a child another thread forks while this process
/// has the file open inherits that handle until it execs. Writing it here
/// would make running it fail now and then. Only the `sh` ever has it open.
#[allow(dead_code, clippy::disallowed_methods)]
pub fn write_executable(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut child = Command::new("sh")
        .args(["-c", r#"cat > "$1" && chmod 755 "$1""#, "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()?;
    // Dropped right after, so `cat` sees the end of its input.
    child.stdin.take().expect("stdin is piped").write_all(contents)?;
    if !child.wait()?.success() {
        bail!("failed to write {}", path.display());
    }
    Ok(())
}

/// Copies the executable `from` to `to`. Done by `cp` for the same reason
/// [`write_executable`] uses `sh`.
#[allow(dead_code, clippy::disallowed_methods)]
pub fn copy_executable(from: &Path, to: &Path) -> Result<()> {
    let status = Command::new("sh")
        .args(["-c", r#"cp "$1" "$2" && chmod 755 "$2""#, "sh"])
        .arg(from)
        .arg(to)
        .status()?;
    if !status.success() {
        bail!("failed to copy {} to {}", from.display(), to.display());
    }
    Ok(())
}
