use crate::errors::*;
use crate::format::Pkg;

#[derive(Debug, serde::Serialize)]
pub struct Json {
    name: String,
    cargo_lock_version: String,
    repository: Option<String>,
    license: Option<String>,
    debian: Option<DebianJson>,
    depth: usize,
}

#[derive(Debug, serde::Serialize)]
pub struct DebianJson {
    version: String,
    compatible: bool,
    exact_match: bool,
    in_new: bool,
    in_unstable: bool,
    outdated: bool,
}

impl Json {
    pub fn new(pkg: &Pkg, depth: usize) -> Self {
        let debian = pkg.debinfo.as_ref().map(|deb| DebianJson {
            version: deb.version.clone(),
            compatible: deb.compatible,
            exact_match: deb.exact_match,
            in_new: deb.in_new,
            in_unstable: deb.in_unstable,
            outdated: deb.outdated,
        });

        Json {
            name: pkg.name.clone(),
            cargo_lock_version: pkg.version.to_string(),
            repository: pkg.repository.clone(),
            license: pkg.license.clone(),
            debian,
            depth,
        }
    }
}

pub fn display(package: &Pkg, depth: usize) -> Result<String, Error> {
    let json = serde_json::to_string(&Json::new(package, depth))?;
    Ok(json)
}
