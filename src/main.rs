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
    about = "Reuse npm installs with private copies or explicit shared generations."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Attach one fixed shared generation to an absent `node_modules`.
    Link(SharedArgs),
    /// Plan or execute adoption without moving the original install.
    Adopt(SharedArgs),
    /// Validate an exact adopted candidate using the policy's application checks.
    Qualify(SharedArgs),
    /// Inspect or execute rollback of an exact retained replacement.
    Recover {
        #[arg(long)]
        cache: PathBuf,
        #[arg(long)]
        transaction: String,
        #[arg(long, conflicts_with = "execute")]
        plan: bool,
        #[arg(long)]
        execute: bool,
    },
    /// List retained install provenance; no garbage collection is performed.
    Retained {
        #[arg(long)]
        cache: PathBuf,
    },
    /// Inspect a shared generation; use --full for a current content audit.
    SharedInspect {
        #[arg(long)]
        cache: PathBuf,
        #[arg(long)]
        artifact: String,
        #[arg(long)]
        full: bool,
    },
    /// Report a shared attachment; structural status is not a clean-content claim.
    SharedStatus {
        #[command(flatten)]
        args: SharedArgs,
        #[arg(long)]
        full: bool,
    },
    /// Run a policy-approved tool with private writable runtime state.
    Run {
        #[command(flatten)]
        args: SharedArgs,
        #[arg(long)]
        tool: String,
    },
    /// Read-only sharing assessment. Reports blockers without running npm.
    Assess {
        #[arg(long)]
        package: PathBuf,
    },
    /// Test write protection in a new disposable fixture, never a live install.
    ProtectionProbe {
        /// Existing ordinary directory on the volume to qualify.
        #[arg(long)]
        parent: PathBuf,
    },
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
struct SharedArgs {
    #[arg(long, conflicts_with = "plan_id")]
    plan: bool,
    #[arg(long)]
    plan_id: Option<String>,
    #[arg(long)]
    package: PathBuf,
    #[arg(long)]
    cache: PathBuf,
    #[arg(long)]
    profile: PathBuf,
    #[arg(long)]
    artifact: Option<String>,
    #[command(flatten)]
    runtime: RuntimeArgs,
}

#[derive(Args)]
struct Install {
    #[arg(long)]
    profile: Option<PathBuf>,
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
        Commands::Link(args) => shared_link(&args),
        Commands::Adopt(args) => shared_adopt(&args),
        Commands::Qualify(args) => shared_qualify(&args),
        Commands::Recover {
            cache,
            transaction,
            plan: _,
            execute,
        } => {
            let plan = nmpool::shared::Store::open(&cache)?.recover(&transaction, execute)?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(0)
        }
        Commands::Retained { cache } => {
            let plans = nmpool::shared::Store::open(&cache)?.retained()?;
            println!("{}", serde_json::to_string_pretty(&plans)?);
            Ok(0)
        }
        Commands::SharedInspect {
            cache,
            artifact,
            full,
        } => {
            let header = nmpool::shared::Store::open(&cache)?.read(&artifact, full)?;
            println!("{}", serde_json::to_string_pretty(&header)?);
            Ok(0)
        }
        Commands::SharedStatus { args, full } => shared_status(&args, full),
        Commands::Run { args, tool } => {
            reject_planning(&args)?;
            let capture = shared_capture(&args)?;
            let record =
                nmpool::shared::Store::open_for_runtime(&args.cache)?.run_tool(&capture, &tool)?;
            println!("{}", serde_json::to_string_pretty(&record)?);
            Ok(0)
        }
        Commands::Assess { package } => {
            let report = nmpool::assessment::run(&package)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(2)
        }
        Commands::ProtectionProbe { parent } => {
            let report = nmpool::platform::protection::run(&parent)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(result_code(report.qualified_fixture))
        }
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
    if let Some(profile) = &args.profile {
        if restore {
            anyhow::bail!("use_link_for_shared_profile");
        }
        return shared_prepare(args, profile);
    }
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

fn shared_capture(args: &SharedArgs) -> Result<nmpool::island::Capture> {
    nmpool::island::Capture::read(
        &args.package,
        &args.profile,
        &args.runtime.node,
        args.runtime.npm_cli.as_deref(),
    )
}

fn artifact_argument(args: &SharedArgs) -> Result<&str> {
    args.artifact
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("artifact_required"))
}

fn shared_link(args: &SharedArgs) -> Result<u8> {
    if args.plan_id.is_some() && args.artifact.is_some() {
        anyhow::bail!("artifact_bound_by_plan");
    }
    let capture = shared_capture(args)?;
    let store = nmpool::shared::Store::open(&args.cache)?;
    if args.plan {
        let plan = store.plan_replace(&capture, artifact_argument(args)?)?;
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(0);
    }
    let record = match &args.plan_id {
        Some(id) => store.replace(&capture, id)?,
        None => store.link(&capture, artifact_argument(args)?)?,
    };
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(0)
}

fn shared_adopt(args: &SharedArgs) -> Result<u8> {
    if args.artifact.is_some() {
        anyhow::bail!("adoption_does_not_select_existing_artifact");
    }
    let capture = shared_capture(args)?;
    let store = nmpool::shared::Store::open(&args.cache)?;
    if args.plan {
        println!(
            "{}",
            serde_json::to_string_pretty(&store.plan_adopt(&capture)?)?
        );
        return Ok(0);
    }
    let id = args
        .plan_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("adoption_plan_required"))?;
    let (artifact, header) = store.adopt(&capture, id)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"artifact_id":artifact,"header":header}))?
    );
    Ok(0)
}

fn shared_prepare(args: &Install, profile: &std::path::Path) -> Result<u8> {
    let capture = nmpool::island::Capture::read(
        &args.package,
        profile,
        &args.runtime.node,
        args.runtime.npm_cli.as_deref(),
    )?;
    let (artifact, header) = nmpool::shared::Store::open(&args.cache)?.prepare(&capture)?;
    let cleanup_warning = header.cleanup_warning.clone();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"artifact_id":artifact,"header":header,"cleanup_warning":cleanup_warning})
        )?
    );
    Ok(0)
}

fn shared_qualify(args: &SharedArgs) -> Result<u8> {
    reject_planning(args)?;
    let capture = shared_capture(args)?;
    let header =
        nmpool::shared::Store::open(&args.cache)?.qualify(&capture, artifact_argument(args)?)?;
    let mut result = serde_json::to_value(&header)?;
    result
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("header_not_object"))?
        .insert(
            "cleanup_warning".into(),
            serde_json::to_value(&header.cleanup_warning)?,
        );
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(0)
}

fn shared_status(args: &SharedArgs, full: bool) -> Result<u8> {
    reject_planning(args)?;
    let capture = shared_capture(args)?;
    let record = nmpool::shared::Store::open(&args.cache)?.status(&capture, full)?;
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(2)
}

fn reject_planning(args: &SharedArgs) -> Result<()> {
    if args.plan || args.plan_id.is_some() {
        anyhow::bail!("planning_flags_require_link_or_adopt");
    }
    Ok(())
}
