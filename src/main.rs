#![allow(
    clippy::print_stdout,
    reason = "The CLI intentionally writes its reports to stdout"
)]

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use nmpool::{
    cache::Cache,
    census,
    inputs::{Package, Toolchain},
};
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(
    version,
    about = "Verified private npm install reuse. Existing installs are never replaced."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Read-only inventory of registered Git worktrees. No npm execution.
    #[command(visible_alias = "scan")]
    Census {
        #[arg(long, required = true)]
        repo: Vec<PathBuf>,
        #[arg(long, default_value_t = 2)]
        max_depth: usize,
        #[arg(long)]
        json: bool,
    },
    /// Build a fresh cache entry with npm ci --ignore-scripts in private staging.
    Prepare(Install),
    /// Restore a verified private copy into an absent `node_modules`.
    Restore(Install),
    /// Check a restored install for input or file drift. Exit 2 means not clean.
    Status {
        #[arg(long)]
        package: PathBuf,
        #[command(flatten)]
        runtime: RuntimeArgs,
    },
    /// Explain why two packages need the same or different installs.
    Explain {
        #[arg(long)]
        package: PathBuf,
        #[arg(long)]
        against: PathBuf,
        #[command(flatten)]
        runtime: RuntimeArgs,
        #[arg(long)]
        against_node: Option<PathBuf>,
        #[arg(long)]
        against_npm_cli: Option<PathBuf>,
    },
    /// Read and verify an entry without changing the cache or package.
    Inspect {
        #[arg(long)]
        cache: PathBuf,
        #[arg(long)]
        key: String,
    },
}

#[derive(Args)]
struct Install {
    #[arg(long)]
    package: PathBuf,
    #[arg(long)]
    cache: PathBuf,
    #[command(flatten)]
    runtime: RuntimeArgs,
}

#[derive(Args)]
struct RuntimeArgs {
    #[arg(long, default_value = "node")]
    node: PathBuf,
    /// Path to npm's bin/npm-cli.js; discovered from a standard install if omitted.
    #[arg(long)]
    npm_cli: Option<PathBuf>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("nmpool: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    match cli.command {
        Commands::Census {
            repo,
            max_depth,
            json,
        } => {
            let report = census::run(&repo, max_depth)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            print_census(&report, json);
            Ok(result_code(report.complete_within_scope))
        }
        Commands::Status { package, runtime } => {
            let report =
                nmpool::state::status(&package, &runtime.node, runtime.npm_cli.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(result_code(report.state == "clean"))
        }
        Commands::Explain {
            package,
            against,
            runtime,
            against_node,
            against_npm_cli,
        } => {
            let package = Package::read(&package)?;
            let against = Package::read(&against)?;
            let tools = Toolchain::discover(&runtime.node, runtime.npm_cli.as_deref())?;
            let mut other_npm = against_npm_cli.as_deref();
            if against_node.is_none() && other_npm.is_none() {
                other_npm = runtime.npm_cli.as_deref();
            }
            let other =
                Toolchain::discover(against_node.as_deref().unwrap_or(&runtime.node), other_npm)?;
            let report = nmpool::state::explain(&package, &against, &tools, &other)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(0)
        }
        Commands::Prepare(args) => install(&args, false),
        Commands::Restore(args) => install(&args, true),
        Commands::Inspect { cache, key } => {
            let receipt = nmpool::cache::inspect(&cache, &key)?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            Ok(0)
        }
    }
}

fn install(args: &Install, restore: bool) -> Result<u8> {
    let started = std::time::Instant::now();
    let package = Package::read(&args.package)?;
    let tools = Toolchain::discover(&args.runtime.node, args.runtime.npm_cli.as_deref())?;
    let cache = Cache::open(&args.cache)?;
    let mut outcome = install_outcome(&cache, &package, &tools, restore)?;
    outcome.elapsed_ms = started.elapsed().as_millis();
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(0)
}

const fn result_code(clean: bool) -> u8 {
    if clean {
        return 0;
    }
    2
}

fn print_census(report: &census::Census, json: bool) {
    if !json {
        for row in &report.rows {
            println!(
                "{}\t{}\t{}",
                row.install_state,
                row.package.display(),
                row.unsupported_reason
                    .as_deref()
                    .unwrap_or("candidate; provenance unverified")
            );
        }
        for error in &report.errors {
            eprintln!("scan-error: {error}");
        }
        println!(
            "{} package paths; {} repeated worktree enumerations; physical savings unknown",
            report.rows.len(),
            report.duplicate_worktree_enumerations
        );
    }
}

fn install_outcome(
    cache: &Cache,
    package: &Package,
    tools: &Toolchain,
    restore: bool,
) -> Result<nmpool::cache::Outcome> {
    if restore {
        return cache.restore(package, tools);
    }
    cache.prepare(package, tools)
}
