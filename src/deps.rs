use crate::{Context, Result};

pub mod brew;

pub trait Dependency {
    fn name(&self) -> &str;
    fn desc(&self) -> &str;
    fn examples(&self) -> Vec<Example> {
        Vec::new()
    }

    /// The steps that make up this dependency, in order. Empty means it's met
    /// by calling [`Dependency::meet`] directly.
    fn tasks(&self) -> &[Box<dyn Task>] {
        &[]
    }

    fn meet(&self, ctx: &mut Context) -> Result<()>;
    fn is_met(&self, ctx: &mut Context) -> Result<bool>;
    fn down(&self, ctx: &mut Context) -> Result<()>;
}

pub trait Task {
    fn name(&self) -> &str;
    fn apply(&self, ctx: &mut Context) -> Result<()>;
    fn is_applied(&self, ctx: &mut Context) -> Result<bool>;
}

pub struct Example {
    pub name: String,
    pub yaml: String,
}

impl Example {
    pub fn new<T: Into<String>>(name: T, yaml: T) -> Self {
        Self {
            name: name.into(),
            yaml: yaml.into(),
        }
    }
}
