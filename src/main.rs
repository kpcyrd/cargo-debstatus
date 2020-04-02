use crate::args::Opts;
use anyhow::Error;
use structopt::StructOpt;

mod args;
mod db;
mod debian;
mod format;
mod graph;
mod metadata;
mod tree;

fn main() -> Result<(), Error> {
    let Opts::Tree(args) = Opts::from_args();
    let metadata = metadata::get(&args)?;
    let mut graph = graph::build(&args, metadata)?;
    debian::populate(&mut graph)?;
    tree::print(&args, &graph)?;

    Ok(())
}
