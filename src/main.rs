use crate::args::{ColorMode, Opts};
use crate::db::Connection;
use crate::errors::*;
use clap::Parser;
use colored::control::set_override;
use rustsec::cargo_lock::Name;
use rustsec::database::Query;
use rustsec::Database;
use std::collections::HashMap;
use std::io;
use std::str::FromStr;

mod args;
mod db;
mod debian;
mod errors;
mod format;
mod graph;
mod metadata;
mod tree;

fn main() -> Result<(), Error> {
    env_logger::init();

    let Opts::Tree(args) = Opts::parse();
    if args.color == ColorMode::Always {
        set_override(true);
    } else if args.color == ColorMode::Never {
        set_override(false);
    }
    info!("Reading metadata");
    let metadata = metadata::get(&args)?;
    let database = Database::fetch()?;
    let mut vulns = HashMap::new();
    for p in &metadata.packages {
        let name = Name::from_str(&p.name)?;
        let q = Query::new()
            .package_name(name)
            .package_version(p.version.clone());
        vulns.insert(p.name.to_string(), database.query(&q));
    }
    info!("Building graph");
    let mut graph = graph::build(&args, metadata, &vulns)?;
    info!("Populating with debian data");
    debian::populate(&mut graph, &args, &Connection::new)?;
    info!("Printing graph");
    tree::print(&args, &graph, &mut io::stdout())?;

    Ok(())
}
