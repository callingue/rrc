use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Ok, bail};
use rrc_core::service::ServiceName;

use crate::types::unit::Unit;

pub fn load_units(dir: &Path) -> anyhow::Result<Vec<Unit>> {
    let service_files = find_service_files(dir)?;

    let mut units = Vec::with_capacity(service_files.len());
    let mut seen: HashMap<ServiceName, PathBuf> = HashMap::new();

    for service_file in &service_files {
        let parsed_unit = parse_service_file(service_file)?;

        // Two files claiming the same name would silently collapse into one entry
        // once units get keyed by name. Reject it here, while both paths are known.
        if let Some(first) = seen.insert(parsed_unit.service.name.clone(), service_file.clone()) {
            bail!(
                "duplicate service name `{}`: {} and {}",
                parsed_unit.service.name,
                first.display(),
                service_file.display()
            );
        }

        units.push(parsed_unit);
    }

    Ok(units)
}

fn find_service_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    let entries =
        fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // `file_type()` does not follow symlinks, so a link pointing at an ancestor
        // directory cannot send us into infinite recursion.
        if entry.file_type()?.is_dir() {
            result.extend(find_service_files(&path)?);
        } else if path.extension().and_then(|e| e.to_str()) == Some("service") {
            result.push(path);
        }
    }

    Ok(result)
}

fn parse_service_file(file_path: &Path) -> anyhow::Result<Unit> {
    let content = fs::read_to_string(file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;
    let unit =
        toml::from_str(&content).with_context(|| format!("parsing {}", file_path.display()))?;
    Ok(unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_toml(name: &str) -> String {
        format!(
            "[service]\nname = \"{name}\"\ndesc = \"\"\nprovides = []\ndeps = []\n\
             runlevels = []\n\n[exec]\nkind = \"oneshot\"\nstart = [\"/bin/echo\"]\n"
        )
    }

    #[test]
    fn duplicate_service_names_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("one.service"), unit_toml("dup")).unwrap();
        fs::write(dir.path().join("two.service"), unit_toml("dup")).unwrap();

        // `.err()` rather than `unwrap_err()`, which would require `Unit: Debug`.
        let err = load_units(dir.path())
            .err()
            .expect("expected the duplicate name to be rejected")
            .to_string();
        assert!(err.contains("duplicate service name `dup`"), "{err}");
    }

    #[test]
    fn parse_error_names_the_offending_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("broken.service"), "this is not toml").unwrap();

        let err = load_units(dir.path())
            .err()
            .expect("expected a parse error")
            .to_string();
        assert!(err.contains("broken.service"), "{err}");
    }

    #[test]
    fn unknown_field_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!("{}stpo = [\"/bin/echo\"]\n", unit_toml("typo"));
        fs::write(dir.path().join("typo.service"), text).unwrap();

        let err = load_units(dir.path())
            .err()
            .expect("expected the misspelled key to be rejected")
            .to_string();
        assert!(err.contains("typo.service"), "{err}");
        // The root cause travels in the error source, not in the top-level context.
        let cause = load_units(dir.path())
            .err()
            .map(|e| format!("{e:#}"))
            .unwrap_or_default();
        assert!(cause.contains("unknown field"), "{cause}");
    }

    #[test]
    fn distinct_service_names_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.service"), unit_toml("a")).unwrap();
        fs::write(dir.path().join("b.service"), unit_toml("b")).unwrap();

        assert_eq!(load_units(dir.path()).unwrap().len(), 2);
    }
}
