use std::collections::HashMap;

use rrc_cfg::types::execspec::ExecSpec;
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
}
