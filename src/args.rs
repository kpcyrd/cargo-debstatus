use clap::{ArgAction, Parser, ValueEnum};
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

use crate::filter::DependencyFilter;

#[derive(Parser)]
#[clap(bin_name = "cargo")]
pub enum Opts {
    #[clap(name = "debstatus")]
    /// Display a tree visualization of a dependency graph
    Tree(Args),
}

#[derive(ValueEnum, Clone, Default, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// Do not add colors to the output
    Never,
    /// Attempt to detect if the output stream supports colors
    #[default]
    Auto,
    /// Always add colors to the output
    Always,
}

impl Display for ColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ColorMode::Never => "never",
            ColorMode::Auto => "auto",
            ColorMode::Always => "always",
        })
    }
}

#[derive(Parser, Clone)]
pub struct Args {
    #[clap(long = "package", short = 'p', value_name = "SPEC")]
    /// Package to be used as the root of the tree
    pub package: Option<String>,
    #[clap(long = "include", value_name = "PACKAGES")]
    /// Comma-separated list of workspace members to include in output
    pub included: Option<String>,
    #[clap(long = "exclude", value_name = "PACKAGES")]
    /// Comma-separated list of workspace members to exclude from output
    pub excluded: Option<String>,
    #[clap(long = "features", value_name = "FEATURES")]
    /// Space-separated list of features to activate
    pub features: Option<String>,
    #[clap(long = "all-features")]
    /// Activate all available features
    pub all_features: bool,
    #[clap(long = "no-default-features")]
    /// Do not activate the `default` feature
    pub no_default_features: bool,
    #[clap(long = "target", value_name = "TARGET")]
    /// Set the target triple
    pub target: Option<String>,
    #[clap(long = "all-targets")]
    /// Return dependencies for all targets. By default only the host target is matched.
    pub all_targets: bool,
    #[clap(long = "skip-cache")]
    /// Do not read from disk cache for Debian database results
    pub skip_cache: bool,
    #[clap(long = "concurrency", short = 'j', default_value = "24")]
    /// How many database connections to use concurrently
    pub concurrency: usize,
    #[clap(long = "no-dev-dependencies")]
    /// Skip dev dependencies.
    pub no_dev_dependencies: bool,
    #[clap(long = "filter", value_delimiter = ',')]
    /// Filter dependencies based on their debian availability
    pub filter: Vec<DependencyFilter>,
    #[clap(long = "manifest-path", value_name = "PATH")]
    /// Path to Cargo.toml
    pub manifest_path: Option<PathBuf>,
    #[clap(long = "collapse-workspace", short = 'w')]
    /// Hide the dependency trees of workspace members which are dependencies of other members
    pub collapse_workspace: bool,
    #[clap(long = "invert", short = 'i')]
    /// Invert the tree direction
    pub invert: bool,
    #[clap(long = "no-indent")]
    /// Display the dependencies as a list (rather than a tree)
    pub no_indent: bool,
    #[clap(long = "prefix-depth")]
    /// Display the dependencies as a list (rather than a tree), but prefixed with the depth
    pub prefix_depth: bool,
    #[clap(long = "all", short = 'a')]
    /// Don't truncate dependencies that have already been displayed
    pub all: bool,
    #[clap(long = "json")]
    /// Print package information as machine-readable output
    pub json: bool,
    #[clap(long = "duplicate", short = 'd')]
    /// Show only dependencies which come in multiple versions (implies -i)
    pub duplicates: bool,
    #[clap(long = "charset", value_name = "CHARSET", default_value = "utf8")]
    /// Character set to use in output: utf8, ascii
    pub charset: Charset,
    #[clap(
        long = "format",
        short = 'f',
        value_name = "FORMAT",
        default_value = "{p}"
    )]
    /// Format string used for printing dependencies
    pub format: String,
    #[clap(long = "verbose", short = 'v', action(ArgAction::Count))]
    /// Use verbose output (-vv very verbose/build.rs output)
    pub verbose: u8,
    #[clap(long = "quiet", short = 'q')]
    /// No output printed to stdout other than the tree
    pub quiet: bool,
    #[clap(long = "color", default_value_t = ColorMode::Auto)]
    /// Coloring: auto, always, never
    pub color: ColorMode,
    #[clap(long = "frozen")]
    /// Require Cargo.lock and cache are up to date
    pub frozen: bool,
    #[clap(long = "locked")]
    /// Require Cargo.lock is up to date
    pub locked: bool,
    #[clap(long = "offline")]
    /// Do not access the network
    pub offline: bool,
    #[clap(short = 'Z', value_name = "FLAG")]
    /// Unstable (nightly-only) flags to Cargo
    pub unstable_flags: Vec<String>,
}

#[derive(Clone, Copy)]
pub enum Charset {
    Utf8,
    Ascii,
}

impl FromStr for Charset {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Charset, &'static str> {
        match s {
            "utf8" => Ok(Charset::Utf8),
            "ascii" => Ok(Charset::Ascii),
            _ => Err("invalid charset"),
        }
    }
}
