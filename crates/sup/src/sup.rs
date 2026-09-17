use std::{collections::HashMap, path::Path};

use rrc_cfg::{config::load_units, types::execspec::ExecSpec};
use rrc_core::{service::ServiceName, state::Status};

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use rrc_core::state::State;

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
}
