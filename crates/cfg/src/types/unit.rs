use crate::types::execspec::ExecSpec;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub service: rrc_core::service::Service,
    pub exec: ExecSpec,
}
