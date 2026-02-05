use crate::args::{ColorMode, Opts};
use crate::db::Connection;
use crate::errors::*;
use clap::Parser;
use colored::control::set_override;
use std::io;

mod args;
mod db;
mod debian;
mod errors;
mod filter;
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
    info!("Building graph");
    let mut graph = graph::build(&args, metadata)?;
    info!("Populating with debian data");
    debian::populate(&mut graph, &args, &Connection::new)?;
    for filter in &args.filter {
        filter.run(&mut graph);
    }
    info!("Printing graph");
    tree::print(&args, &graph, &mut io::stdout())?;

    Ok(())
}
