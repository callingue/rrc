use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecSpec {
    #[serde(default)]
    pub kind: Kind,
    pub start: Argv,
    pub stop: Option<Argv>,
    pub reload: Option<Argv>,

    pub stop_timeout: Option<Duration>,
    pub kill_timeout: Option<Duration>,
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

/// A command as an argv vector: `argv[0]` is the program, the rest are its arguments.
///
/// Both invariants are enforced at parse time, so a broken command is a config error
/// rather than a failure at boot: the vector is never empty, and the program is an
/// absolute path. Resolving through `PATH` is unreliable early in boot, where the
/// environment may be empty and `/usr` may not be mounted yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct Argv(Vec<String>);

#[derive(Debug, thiserror::Error)]
pub enum ArgvError {
    #[error("command is empty")]
    Empty,
    #[error("program path must be absolute: {0}")]
    NotAbsolute(String),
}

impl TryFrom<Vec<String>> for Argv {
    type Error = ArgvError;

    fn try_from(argv: Vec<String>) -> Result<Self, Self::Error> {
        let program = argv.first().ok_or(ArgvError::Empty)?;
        if !Path::new(program).is_absolute() {
            return Err(ArgvError::NotAbsolute(program.clone()));
        }
        Ok(Self(argv))
    }
}

impl Argv {
    /// Program and its arguments. Infallible: the vector cannot be empty.
    pub fn split(&self) -> (&String, &[String]) {
        self.0.split_first().expect("Argv is never empty")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(start: &str) -> Result<ExecSpec, toml::de::Error> {
        toml::from_str(&format!("kind = \"oneshot\"\nstart = {start}\n"))
    }

    #[test]
    fn absolute_command_parses() {
        let spec = parse("[\"/bin/echo\", \"hi\"]").unwrap();
        let (program, args) = spec.start.split();
        assert_eq!(program, "/bin/echo");
        assert_eq!(args, ["hi"]);
    }

    #[test]
    fn empty_command_is_rejected() {
        let err = parse("[]").unwrap_err().to_string();
        assert!(err.contains("command is empty"), "{err}");
    }

    #[test]
    fn relative_program_is_rejected() {
        let err = parse("[\"echo\"]").unwrap_err().to_string();
        assert!(err.contains("must be absolute"), "{err}");
    }
}
