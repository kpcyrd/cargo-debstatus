use crate::args::Opts;
use crate::db::Connection;
use crate::errors::*;
use clap::Parser;
use std::io;

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
    info!("Reading metadata");
    let metadata = metadata::get(&args)?;
    info!("Building graph");
    let mut graph = graph::build(&args, metadata)?;
    info!("Populating with debian data");
    debian::populate(&mut graph, &args, &Connection::new)?;
    info!("Printing graph");
    tree::print(&args, &graph, &mut io::stdout())?;

    Ok(())
}
