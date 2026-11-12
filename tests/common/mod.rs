use std::{
    cell::RefCell,
    fmt::Display,
    io::{self, Write},
    rc::Rc,
    sync::{Arc, Mutex},
};

use color_eyre::eyre::bail;
use dev::{
    Context,
    Result,
    deps::{Dependency, Task, meet_tasks},
    project::Project,
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

/// A context whose `out` and `err` both write to buffers the test can read.
// Each tests/*.rs file compiles this module separately, and not all of them use it.
#[allow(dead_code)]
pub fn capturing_ctx() -> (Context, Buffer, Buffer) {
    let (out, err) = (Buffer::default(), Buffer::default());
    let ctx = Context {
        out: Box::new(out.clone()),
        err: Box::new(err.clone()),
        ..Context::default()
    };
    (ctx, out, err)
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
