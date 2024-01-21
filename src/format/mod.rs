use crate::debian::Pkg;
use crate::format::parse::{Parser, RawChunk};
use anyhow::{anyhow, Error};
use colored::Colorize;
use std::fmt;

mod parse;

enum Chunk {
    Raw(String),
    Package,
    License,
    Repository,
}

pub struct Pattern(Vec<Chunk>);

impl Pattern {
    pub fn new(format: &str) -> Result<Pattern, Error> {
        let mut chunks = vec![];

        for raw in Parser::new(format) {
            let chunk = match raw {
                RawChunk::Text(text) => Chunk::Raw(text.to_owned()),
                RawChunk::Argument("p") => Chunk::Package,
                RawChunk::Argument("l") => Chunk::License,
                RawChunk::Argument("r") => Chunk::Repository,
                RawChunk::Argument(ref a) => {
                    return Err(anyhow!("unsupported pattern `{}`", a));
                }
                RawChunk::Error(err) => return Err(anyhow!("{}", err)),
            };
            chunks.push(chunk);
        }

        Ok(Pattern(chunks))
    }

    pub fn display<'a>(&'a self, package: &'a Pkg) -> Display<'a> {
        Display {
            pattern: self,
            package,
        }
    }
}

pub struct Display<'a> {
    pattern: &'a Pattern,
    package: &'a Pkg,
}

impl<'a> fmt::Display for Display<'a> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in &self.pattern.0 {
            match *chunk {
                Chunk::Raw(ref s) => fmt.write_str(s)?,
                Chunk::Package => {
                    let pkg = format!("{} v{}", self.package.name, self.package.version);
                    if let Some(deb) = &self.package.debinfo {
                        if deb.in_unstable {
                            if deb.compatible {
                                write!(
                                    fmt,
                                    "{} ({} in debian)",
                                    pkg.green(),
                                    deb.version.yellow()
                                )?;
                            } else if deb.outdated {
                                write!(
                                    fmt,
                                    "{} (outdated, {} in debian)",
                                    pkg.yellow(),
                                    deb.version.red()
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

                    match &self.package.source {
                        Some(source) if !source.is_crates_io() => write!(fmt, " ({source})")?,
                        // https://github.com/rust-lang/cargo/issues/7483
                        None => write!(
                            fmt,
                            " ({})",
                            self.package.manifest_path.parent().unwrap().display()
                        )?,
                        _ => {}
                    }
                }
                Chunk::License => {
                    if let Some(ref license) = self.package.license {
                        write!(fmt, "{license}")?
                    }
                }
                Chunk::Repository => {
                    if let Some(ref repository) = self.package.repository {
                        write!(fmt, "{repository}")?
                    }
                }
            }
        }

        Ok(())
    }
}
