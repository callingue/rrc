use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecSpec {
    #[serde(default)]
    pub kind: Kind,
    pub start: Argv,
    pub stop: Option<Argv>,
    pub reload: Option<Argv>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Runs to completion; exit code 0 means success.
    Oneshot,
    /// Stays in the foreground as our child.
    #[default]
    Simple,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Argv(Vec<String>);

impl Argv {
    pub fn split(&self) -> Option<(&String, &[String])> {
        self.0.split_first()
    }
}
