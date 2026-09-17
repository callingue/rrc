use std::{collections::HashMap, path::Path};

use anyhow::{Context, bail};
use rrc_cfg::{
    config::load_units,
    types::execspec::{ExecSpec, Kind},
};
use rrc_core::{
    service::ServiceName,
    state::{Flags, State, Status},
};
use tokio::process::Command;

pub struct Supervisor {
    execs: HashMap<ServiceName, ExecSpec>,
    statuses: HashMap<ServiceName, Status>,
}

impl Supervisor {
    pub fn new(execs: HashMap<ServiceName, ExecSpec>) -> Self {
        let statuses = execs
            .keys()
            .map(|name| (name.clone(), Status::default()))
            .collect();
        Self { execs, statuses }
    }

    /// Builds a supervisor from every `.service` file under `dir`.
    ///
    /// The graph half of each unit (`deps`, `provides`, `runlevels`) is dropped here:
    /// nothing consumes it until the dependency registry exists, and keeping a copy
    /// that no one reads would just be a second source of truth.
    pub fn from_dir(dir: &Path) -> anyhow::Result<Self> {
        let units = load_units(dir)?;
        let execs = units
            .into_iter()
            .map(|unit| (unit.service.name, unit.exec))
            .collect();

        Ok(Self::new(execs))
    }

    pub fn status(&self, name: &ServiceName) -> Option<Status> {
        self.statuses.get(name).copied()
    }

    /// Starts every known service.
    ///
    /// Ordered by name: there is no dependency graph yet, so this is a placeholder
    /// for the topological order a plan will provide later. A failure is reported and
    /// the run continues — nothing knows yet who depended on the failed service.
    pub async fn start_all(&mut self) {
        let mut order: Vec<ServiceName> = self.execs.keys().cloned().collect();
        order.sort();

        for name in order {
            if let Err(e) = self.start_one(&name).await {
                println!("{name:<16} !! {e:#}");
            }
        }
    }

    pub async fn start_one(&mut self, name: &ServiceName) -> anyhow::Result<()> {
        let spec = self
            .execs
            .get(name)
            .with_context(|| format!("unknown service {name}"))?;

        if spec.kind != Kind::Oneshot {
            bail!("service kind {:?} is not supported yet", spec.kind);
        }

        // Copy the command out before the first `set`: `spec` borrows `self.execs`,
        // and `set` needs `&mut self`.
        let (program, args) = spec.start.split();
        let program = program.clone();
        let args = args.to_vec();

        self.clear_failure(name);
        self.set(name, State::Starting);

        let exit = match Command::new(&program).args(&args).status().await {
            Ok(exit) => exit,
            // Without this arm a service with a bad command would sit in Starting forever.
            Err(e) => {
                self.fail(name);
                return Err(e).with_context(|| format!("spawning {program}"));
            }
        };

        if exit.success() {
            // A oneshot is Started with no process left alive: the work is done.
            self.set(name, State::Started);
            Ok(())
        } else {
            self.fail(name);
            bail!("exited with {exit}")
        }
    }

    /// The only place that writes a state, so every change is checked by the FSM.
    fn set(&mut self, name: &ServiceName, next: State) {
        let Some(status) = self.statuses.get_mut(name) else {
            println!("{name:<16} !! state update for an unknown service");
            return;
        };

        match status.state.transition(next) {
            Ok(state) => {
                status.state = state;
                println!("{name:<16} -> {state:?}");
            }
            // Reaching an illegal transition means the supervisor itself is wrong,
            // so fail loudly in tests and stay on the last valid state in release.
            Err(e) => {
                debug_assert!(false, "{e}");
                println!("{name:<16} !! {e}");
            }
        }
    }

    /// A start that never came up: back to Stopped, and remember why.
    ///
    /// The two writes belong together — the state alone cannot express failure,
    /// since `Stopped` is also the ordinary resting state.
    fn fail(&mut self, name: &ServiceName) {
        self.set(name, State::Stopped);
        if let Some(status) = self.statuses.get_mut(name) {
            status.flags.insert(Flags::FAILED);
        }
    }

    /// A new attempt supersedes the result of the previous one.
    fn clear_failure(&mut self, name: &ServiceName) {
        if let Some(status) = self.statuses.get_mut(name) {
            status.flags.remove(Flags::FAILED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rrc_cfg::types::execspec::Argv;

    fn oneshot(start: &[&str]) -> ExecSpec {
        let argv: Vec<String> = start.iter().map(|s| s.to_string()).collect();
        ExecSpec {
            kind: Kind::Oneshot,
            start: Argv::try_from(argv).unwrap(),
            stop: None,
            reload: None,
        }
    }

    fn sup_with(name: &str, start: &[&str]) -> (Supervisor, ServiceName) {
        let name = ServiceName::new(name).unwrap();
        let sup = Supervisor::new(HashMap::from([(name.clone(), oneshot(start))]));
        (sup, name)
    }

    fn unit_toml(name: &str) -> String {
        format!(
            "[service]\nname = \"{name}\"\ndesc = \"\"\nprovides = []\ndeps = []\n\
             runlevels = []\n\n[exec]\nkind = \"oneshot\"\nstart = [\"/bin/echo\"]\n"
        )
    }

    #[test]
    fn from_dir_registers_every_unit_as_stopped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.service"), unit_toml("a")).unwrap();
        std::fs::write(dir.path().join("b.service"), unit_toml("b")).unwrap();

        let sup = Supervisor::from_dir(dir.path()).unwrap();

        for name in ["a", "b"] {
            let name = ServiceName::new(name).unwrap();
            let status = sup.status(&name).expect("unit should be known");
            assert_eq!(status.state, State::Stopped);
            assert!(status.flags.is_empty());
        }
    }

    #[test]
    fn unknown_service_has_no_status() {
        let dir = tempfile::tempdir().unwrap();
        let sup = Supervisor::from_dir(dir.path()).unwrap();

        assert!(sup.status(&ServiceName::new("ghost").unwrap()).is_none());
    }

    #[tokio::test]
    async fn successful_oneshot_ends_started() {
        let (mut sup, name) = sup_with("ok", &["/bin/sh", "-c", "exit 0"]);

        sup.start_one(&name).await.unwrap();

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Started);
        assert!(!status.flags.contains(Flags::FAILED));
    }

    #[tokio::test]
    async fn failing_oneshot_ends_stopped_and_flagged() {
        let (mut sup, name) = sup_with("bad", &["/bin/sh", "-c", "exit 1"]);

        assert!(sup.start_one(&name).await.is_err());

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Stopped);
        assert!(status.flags.contains(Flags::FAILED));
    }

    #[tokio::test]
    async fn missing_binary_does_not_leave_the_service_starting() {
        let (mut sup, name) = sup_with("ghost", &["/nonexistent/binary"]);

        assert!(sup.start_one(&name).await.is_err());

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Stopped);
        assert!(status.flags.contains(Flags::FAILED));
    }

    #[tokio::test]
    async fn start_all_runs_every_service() {
        let mut execs = HashMap::new();
        for name in ["a", "b"] {
            execs.insert(
                ServiceName::new(name).unwrap(),
                oneshot(&["/bin/sh", "-c", "exit 0"]),
            );
        }
        let mut sup = Supervisor::new(execs);

        sup.start_all().await;

        for name in ["a", "b"] {
            let status = sup.status(&ServiceName::new(name).unwrap()).unwrap();
            assert_eq!(status.state, State::Started);
        }
    }

    #[tokio::test]
    async fn unsupported_kind_is_rejected() {
        let name = ServiceName::new("daemon").unwrap();
        let mut spec = oneshot(&["/bin/sh", "-c", "exit 0"]);
        spec.kind = Kind::Simple;
        let mut sup = Supervisor::new(HashMap::from([(name.clone(), spec)]));

        let err = sup.start_one(&name).await.unwrap_err().to_string();
        assert!(err.contains("not supported yet"), "{err}");
        assert_eq!(sup.status(&name).unwrap().state, State::Stopped);
    }
}
