use crate::errors::*;
use crate::format::{Chunk, Pattern, Pkg};
use colored::Colorize;
use std::fmt::Write;

pub fn display(pattern: &Pattern, package: &Pkg) -> Result<String, Error> {
    let mut fmt = String::new();

    for chunk in &pattern.0 {
        match *chunk {
            Chunk::Raw(ref s) => fmt.write_str(s)?,
            Chunk::Package => {
                let pkg = format!("{} v{}", package.name, package.version);
                if let Some(deb) = &package.debinfo {
                    if deb.in_unstable {
                        if deb.compatible {
                            write!(fmt, "{} ({} in debian)", pkg.green(), deb.version.yellow())?;
                        } else if deb.outdated {
                            write!(
                                fmt,
                                "{} (outdated, {} in debian)",
                                pkg.yellow(),
                                deb.version.red()
                            )?;
                        } else if deb.newer {
                            write!(
                                fmt,
                                "{} (newer, {} in debian)",
                                pkg.yellow(),
                                deb.version.magenta()
                            )?;
                        } else {
                            write!(fmt, "{} (in debian)", pkg.green())?;
                        }
                    } else if deb.in_new {
                        if deb.compatible {
                            write!(
                                fmt,
                                "{} ({} in debian NEW queue)",
                                pkg.blue(),
                                deb.version.yellow()
                            )?;
                        } else if deb.outdated {
                            write!(
                                fmt,
                                "{}, (outdated, {} in debian NEW queue)",
                                pkg.blue(),
                                deb.version.red()
                            )?;
                        } else {
                            write!(fmt, "{} (in debian NEW queue)", pkg.blue())?;
                        }
                    } else if deb.outdated {
                        write!(fmt, "{} (outdated, {})", pkg.red(), deb.version.red())?;
                    } else {
                        write!(fmt, "{pkg}")?;
                    }
                } else {
                    write!(fmt, "{pkg}")?;
                }

                match &package.source {
                    Some(source) if !source.is_crates_io() => write!(fmt, " ({source})")?,
                    // https://github.com/rust-lang/cargo/issues/7483
                    None => write!(
                        fmt,
                        " ({})",
                        package.manifest_path.parent().unwrap().display()
                    )?,
                    _ => {}
                }
            }
            Chunk::License => {
                if let Some(license) = &package.license {
                    write!(fmt, "{license}")?
                }
            }
            Chunk::Repository => {
                if let Some(repository) = &package.repository {
                    write!(fmt, "{repository}")?
                }
            }
        }
    }

    Ok(fmt)
}
