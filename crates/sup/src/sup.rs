use std::collections::HashMap;

use rrc_cfg::types::execspec::ExecSpec;
use rrc_core::{service::ServiceName, state::State};

pub struct Supervisor {
    execs: HashMap<ServiceName, ExecSpec>,
    states: HashMap<ServiceName, State>,
}

impl Supervisor {
    pub fn new() {
        
    }
}