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
    /// Restore a verified private copy into an absent node_modules.
    Restore(Install),
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
            Ok(if report.complete_within_scope { 0 } else { 2 })
        }
        Commands::Prepare(args) => install(args, false),
        Commands::Restore(args) => install(args, true),
        Commands::Inspect { cache, key } => {
            let receipt = nmpool::cache::inspect(&cache, &key)?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            Ok(0)
        }
    }
}

fn install(args: Install, restore: bool) -> Result<u8> {
    let package = Package::read(&args.package)?;
    let tools = Toolchain::discover(&args.node, args.npm_cli.as_deref())?;
    let cache = Cache::open(&args.cache)?;
    let outcome = if restore {
        cache.restore(&package, &tools)?
    } else {
        cache.prepare(&package, &tools)?
    };
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(0)
}
