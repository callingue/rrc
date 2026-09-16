use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceName(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid service name: {0}")]
pub struct ServiceNameError(String);

// Long enough for real-world names such as `systemd-resolved`, short enough to keep
// runtime paths like `/run/rrc/<name>` manageable. Deliberately unrelated to the
// kernel's 15-character `task_struct.comm` limit: that one caps thread names, which
// is a separate concern from what a service may be called.
const MAX_SERVICE_NAME_LEN: usize = 64;

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

impl std::fmt::Display for ServiceName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
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
    fn service_name_longer_than_limit_is_rejected() {
        let s = "a".repeat(MAX_SERVICE_NAME_LEN + 1);
        assert_eq!(
            format!("invalid service name: {}", s),
            ServiceName::new(s.as_str()).unwrap_err().to_string()
        );
    }

    #[test]
    fn service_name_at_limit_is_accepted() {
        let s = "a".repeat(MAX_SERVICE_NAME_LEN);
        assert!(ServiceName::new(s.as_str()).is_ok());
    }

    #[test]
    fn service_name_contains_invalid_character() {
        let s = "lets_g#_rusty";
         assert_eq!(format!("invalid service name: {}", s), ServiceName::new(s).unwrap_err().to_string());
    }
}