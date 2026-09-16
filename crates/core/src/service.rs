use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceName(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid service name: {0}")]
pub struct ServiceNameError(String);

// Size of the kernel's `task_struct.comm` buffer (`include/linux/sched.h`).
const TASK_COMM_LEN: usize = 16;
const MAX_SERVICE_NAME_LEN: usize = TASK_COMM_LEN - 1;

impl ServiceName {
    pub fn new(s: impl Into<String>) -> Result<Self, ServiceNameError> {
        let s = s.into();
        let ok = !s.is_empty() && s.len() <= MAX_SERVICE_NAME_LEN
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
        if ok {
            Ok(Self(s))
        } else {
            Err(ServiceNameError(s))
        }
    }
}

impl TryFrom<String> for ServiceName {
    type Error = ServiceNameError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<ServiceName> for String {
    fn from(s: ServiceName) -> Self {
        s.0
    }
}

#[derive(Serialize, Deserialize)]
pub enum DepKind {
    /// Hard requirement: pull the target in, start it first, fail if it fails.
    /// On shutdown, dependants are stopped before the target. (`sshd` needs `net`)
    Need,
    /// Like [`Need`](Self::Need), but the target's failure is tolerated —
    /// we start anyway. For optional extras such as a metrics exporter.
    Want,
    /// Order after the target only if something else already put it in the plan;
    /// otherwise a no-op. "I use it when it's around" — e.g. `use logger`.
    Use,
    /// Pure ordering: target first if present, no functional claim.
    After,
    /// Mirror of [`After`](Self::After): we start first, and stop last.
    /// Lets a unit insert itself into an ordering it doesn't own (`before net`).
    Before,
}

#[derive(Serialize, Deserialize)]
pub struct Dependency {
    pub kind: DepKind,
    pub target: ServiceName,
}

#[derive(Serialize, Deserialize)]
pub struct Service {
    pub name: ServiceName,
    pub desc: String,
    pub provides: Vec<ServiceName>,
    pub deps: Vec<Dependency>, 
    pub runlevels: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_service_name() {
        let s = "lets_go_rusty";
        assert_eq!(s.to_string(), ServiceName::new(s).unwrap().0);
    }

    #[test]
    fn service_name_len_more_than_fifteen() {
        let s = "letsgorustytogether";
        assert_eq!(format!("invalid service name: {}", s), ServiceName::new(s).unwrap_err().to_string());
    }

    #[test]
    fn service_name_contains_invalid_character() {
        let s = "lets_g#_rusty";
         assert_eq!(format!("invalid service name: {}", s), ServiceName::new(s).unwrap_err().to_string());
    }
}