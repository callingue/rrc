use std::{fs, path::{Path, PathBuf}};

use anyhow::Ok;

use crate::types::unit::Unit;

pub fn load_units(dir: &Path) -> anyhow::Result<Vec<Unit>> {
    let service_files = find_service_files(dir)?;

    let mut units = vec![];
    for service_file in &service_files {
        let parsed_unit = parse_service_file(service_file)?;
        units.push(parsed_unit);
    }

    Ok(units)
}

fn find_service_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    let entries = fs::read_dir(dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            result.extend(find_service_files(&path)?);
        } else if path.extension().and_then(|e| e.to_str()) == Some("service") {
            result.push(path);
        }
    }

    Ok(result)
}

fn parse_service_file(file_path: &Path) -> anyhow::Result<Unit> {
    let content = fs::read_to_string(file_path)?;
    let unit = toml::from_str(&content)?;
    Ok(unit)
}