//! Explicit, reviewed npm island inputs and commands. Staging is not a sandbox.
use crate::{digest, inputs, platform};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

const LIMIT: u64 = 32 * 1024 * 1024;
const MARKER: &str = ".nmpool-island-stage.json";
const CONTEXT_NAMES: [&str; 9] = [
    "package.json",
    ".npmrc",
    "pnpm-workspace.yaml",
    ".pnp.cjs",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema: String,
    pub independent_npm_island: bool,
    pub acknowledge_unsandboxed_scripts: bool,
    pub trust_domain: String,
    pub allow_local_attestation: bool,
    pub generator_inputs: Vec<String>,
    pub context_inputs: Vec<String>,
    pub required_probes: Vec<String>,
    pub build_commands: Vec<CommandSpec>,
    pub validation_commands: Vec<CommandSpec>,
    pub runtime_commands: BTreeMap<String, CommandSpec>,
    pub selected_env: Vec<String>,
    pub credential_env: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: Program,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Program {
    Node,
    Npm,
}

pub struct Capture {
    pub package: PathBuf,
    pub policy: Policy,
    pub request_key: String,
    pub policy_hash: String,
    pub runtime_digest: String,
    profile: PathBuf,
    profile_bytes: Vec<u8>,
    files: BTreeMap<String, Option<Vec<u8>>>,
    environment: BTreeMap<String, Option<String>>,
    toolchain: inputs::Toolchain,
}

impl Capture {
    pub fn read(
        package: &Path,
        profile: &Path,
        node: &Path,
        npm_cli: Option<&Path>,
    ) -> Result<Self> {
        let package = platform::absolute(package)?;
        let profile = platform::absolute(profile)?;
        platform::plain_path(&package)?;
        platform::plain_path(&profile)?;
        let package = dunce::canonicalize(package)?;
        let profile = dunce::canonicalize(profile)?;
        let profile_bytes = required(&profile)?;
        let policy: Policy = serde_json::from_slice(&profile_bytes)
            .map_err(|_| anyhow::anyhow!("island_policy_invalid"))?;
        validate_policy(&policy)?;
        let files = capture_files(&package, &policy)?;
        validate_package(&files, &policy)?;
        let environment = selected_environment(&policy)?;
        let toolchain = inputs::Toolchain::discover(node, npm_cli)?;
        let policy_hash = digest(&profile_bytes);
        let runtime_digest = digest(&serde_json::to_vec(&toolchain.runtime)?);
        let request_key = request_key(&files, &environment, &policy_hash, &runtime_digest)?;
        Ok(Self {
            package,
            policy,
            request_key,
            policy_hash,
            runtime_digest,
            profile,
            profile_bytes,
            files,
            environment,
            toolchain,
        })
    }

    pub fn ensure_unchanged(&self) -> Result<()> {
        if required(&self.profile)? != self.profile_bytes
            || capture_files(&self.package, &self.policy)? != self.files
        {
            bail!("island_inputs_changed");
        }
        if selected_environment(&self.policy)? != self.environment {
            bail!("island_environment_changed");
        }
        self.toolchain.unchanged()
    }

    pub fn stage(&self, destination: &Path) -> Result<()> {
        self.ensure_unchanged()?;
        platform::plain_path(destination)?;
        platform::absent(destination)?;
        fs::create_dir(destination)?;
        for name in local_names(&self.policy) {
            stage_file(
                destination,
                &name,
                self.files.get(&name).and_then(Option::as_ref),
            )?;
        }
        fs::write(destination.join(MARKER), &self.request_key)?;
        self.ensure_unchanged()
    }

    pub fn build(&self, stage: &Path) -> Result<()> {
        self.execute(stage, &self.policy.build_commands)
    }

    pub fn validate(&self, stage: &Path) -> Result<()> {
        self.execute(stage, &self.policy.validation_commands)
    }

    pub fn run_runtime(&self, package: &Path, name: &str, runtime: &Path) -> Result<()> {
        self.ensure_unchanged()?;
        platform::plain_path(package)?;
        let command = self
            .policy
            .runtime_commands
            .get(name)
            .context("island_runtime_command_unknown")?;
        platform::plain_path(runtime)?;
        fs::create_dir_all(runtime)?;
        self.run_command(package, runtime, command, false)?;
        self.ensure_unchanged()
    }

    fn execute(&self, stage: &Path, commands: &[CommandSpec]) -> Result<()> {
        self.ensure_unchanged()?;
        self.check_stage(stage)?;
        let runtime = tempfile::tempdir()?;
        let runtime = dunce::canonicalize(runtime.path())?;
        for command in commands {
            self.run_command(stage, &runtime, command, true)?;
        }
        self.check_stage(stage)?;
        self.ensure_unchanged()
    }

    fn check_stage(&self, stage: &Path) -> Result<()> {
        platform::plain_path(stage)?;
        if same_file::is_same_file(stage, &self.package)?
            || required(&stage.join(MARKER))? != self.request_key.as_bytes()
        {
            bail!("island_stage_identity");
        }
        for name in local_names(&self.policy) {
            let expected = self
                .files
                .get(&name)
                .and_then(Option::as_ref)
                .map(|bytes| inputs::keyed_bytes(&name, bytes));
            if optional(&stage.join(&name))? != expected {
                bail!("island_staged_inputs_changed");
            }
        }
        Ok(())
    }

    fn run_command(
        &self,
        stage: &Path,
        runtime: &Path,
        spec: &CommandSpec,
        credentials: bool,
    ) -> Result<()> {
        let mut command = Command::new(&self.toolchain.node);
        command
            .env_clear()
            .current_dir(stage)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        prepare_environment(&mut command, runtime, &self.toolchain.node)?;
        set_selected_environment(&mut command, &self.environment);
        if credentials {
            set_credentials(&mut command, &self.policy.credential_env)?;
        }
        if matches!(spec.program, Program::Npm) {
            command.arg(&self.toolchain.npm_cli);
        }
        for argument in &spec.args {
            command.arg(expand(argument, runtime));
        }
        for (name, value) in &spec.env {
            command.env(name, expand(value, runtime));
        }
        if !command.status().context("island_command_spawn")?.success() {
            bail!("island_command_failed");
        }
        Ok(())
    }
}

fn validate_policy(policy: &Policy) -> Result<()> {
    if policy.schema != "nmpool/island/v1"
        || !policy.independent_npm_island
        || !policy.acknowledge_unsandboxed_scripts
    {
        bail!("island_policy_requires_explicit_approval");
    }
    if !safe_label(&policy.trust_domain)
        || policy.required_probes.is_empty()
        || policy.required_probes.len() > 16
    {
        bail!("island_policy_identity_or_probes");
    }
    if policy.build_commands.is_empty()
        || policy.validation_commands.is_empty()
        || policy.runtime_commands.is_empty()
    {
        bail!("island_policy_commands_required");
    }
    if policy.generator_inputs.len() + policy.context_inputs.len() > 128 {
        bail!("island_policy_input_limit");
    }
    validate_paths(policy)?;
    validate_commands(policy)?;
    validate_environment_policy(policy)
}

fn validate_paths(policy: &Policy) -> Result<()> {
    for name in &policy.generator_inputs {
        safe_relative(name)?;
        if [
            "package.json",
            "package-lock.json",
            "npm-shrinkwrap.json",
            ".npmrc",
            MARKER,
        ]
        .contains(&name.as_str())
        {
            bail!("island_reserved_input");
        }
    }
    for name in &policy.required_probes {
        safe_relative(name)?;
    }
    for name in &policy.context_inputs {
        safe_context(name)?;
    }
    Ok(())
}

fn safe_relative(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['\\', ':']) || name.len() > 1024 {
        bail!("island_input_path");
    }
    for component in Path::new(name).components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("island_input_path");
        }
    }
    if name.split('/').any(|part| {
        ["node_modules", ".git", ".nmpool-runtime"]
            .iter()
            .any(|reserved| part.eq_ignore_ascii_case(reserved))
    }) {
        bail!("island_reserved_path");
    }
    Ok(())
}

fn safe_context(name: &str) -> Result<()> {
    let suffix = name.trim_start_matches("../");
    if name.len() - suffix.len() > 64 * 3 || !CONTEXT_NAMES.contains(&suffix) {
        bail!("island_context_path");
    }
    safe_relative(suffix)
}

fn validate_commands(policy: &Policy) -> Result<()> {
    let commands = policy
        .build_commands
        .iter()
        .chain(&policy.validation_commands)
        .chain(policy.runtime_commands.values());
    for command in commands {
        validate_command(command)?;
    }
    if policy.build_commands.len()
        + policy.validation_commands.len()
        + policy.runtime_commands.len()
        > 64
    {
        bail!("island_command_limit");
    }
    if policy.runtime_commands.keys().any(|name| !safe_label(name)) {
        bail!("island_runtime_name");
    }
    Ok(())
}

fn validate_command(command: &CommandSpec) -> Result<()> {
    if command.args.is_empty()
        || command.args.len() > 128
        || command
            .args
            .iter()
            .any(|arg| arg.len() > 8192 || arg.contains('\0'))
    {
        bail!("island_command_arguments");
    }
    for (name, value) in &command.env {
        if !safe_env(name) || secret_name(name) || value.len() > 8192 || value.contains('\0') {
            bail!("island_command_environment");
        }
    }
    Ok(())
}

fn validate_environment_policy(policy: &Policy) -> Result<()> {
    if policy.selected_env.len() + policy.credential_env.len() > 32 {
        bail!("island_environment_limit");
    }
    for name in &policy.selected_env {
        if !safe_env(name) || secret_name(name) || policy.credential_env.contains(name) {
            bail!("island_selected_environment");
        }
    }
    if policy.credential_env.iter().any(|name| !safe_env(name)) {
        bail!("island_credential_environment");
    }
    Ok(())
}

fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn safe_env(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "NODE_OPTIONS",
    ]
    .contains(&upper.as_str())
        || upper.starts_with("NPM_CONFIG_")
    {
        return false;
    }
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn secret_name(name: &str) -> bool {
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "AUTH",
        "CREDENTIAL",
        "PRIVATE_KEY",
    ]
    .iter()
    .any(|part| name.contains(part))
}

fn optional(path: &Path) -> Result<Option<Vec<u8>>> {
    platform::plain_path(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("island_input_unavailable"),
    };
    if !metadata.is_file() || metadata.len() > LIMIT {
        bail!("island_input_type_or_size");
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())? > LIMIT {
        bail!("island_input_size");
    }
    Ok(Some(bytes))
}

fn required(path: &Path) -> Result<Vec<u8>> {
    optional(path)?.context("island_input_missing")
}

fn local_names(policy: &Policy) -> Vec<String> {
    let mut names = vec![
        "package.json".into(),
        "package-lock.json".into(),
        "npm-shrinkwrap.json".into(),
        ".npmrc".into(),
    ];
    names.extend(policy.generator_inputs.iter().cloned());
    names.sort();
    names.dedup();
    names
}

fn capture_files(package: &Path, policy: &Policy) -> Result<BTreeMap<String, Option<Vec<u8>>>> {
    let mut files = BTreeMap::new();
    for name in local_names(policy)
        .into_iter()
        .chain(policy.context_inputs.iter().cloned())
    {
        files.insert(name.clone(), optional(&package.join(&name))?);
    }
    check_generator_inputs(&files, policy)?;
    check_context(package, policy)?;
    Ok(files)
}

fn check_generator_inputs(
    files: &BTreeMap<String, Option<Vec<u8>>>,
    policy: &Policy,
) -> Result<()> {
    for name in &policy.generator_inputs {
        if files.get(name).and_then(Option::as_ref).is_none() {
            bail!("island_generator_input_missing");
        }
    }
    Ok(())
}

fn check_context(package: &Path, policy: &Policy) -> Result<()> {
    for (depth, ancestor) in package.ancestors().enumerate() {
        if depth > 64 {
            bail!("island_ancestor_limit");
        }
        check_ancestor(ancestor, depth, policy)?;
    }
    Ok(())
}

fn check_ancestor(ancestor: &Path, depth: usize, policy: &Policy) -> Result<()> {
    for name in CONTEXT_NAMES {
        check_context_file(ancestor, depth, name, policy)?;
    }
    Ok(())
}

fn check_context_file(ancestor: &Path, depth: usize, name: &str, policy: &Policy) -> Result<()> {
    if depth == 0
        && [
            "package.json",
            "package-lock.json",
            "npm-shrinkwrap.json",
            ".npmrc",
        ]
        .contains(&name)
    {
        return Ok(());
    }
    let key = format!("{}{name}", "../".repeat(depth));
    if optional(&ancestor.join(name))?.is_some() && !policy.context_inputs.contains(&key) {
        bail!("island_undeclared_context");
    }
    Ok(())
}

fn validate_package(files: &BTreeMap<String, Option<Vec<u8>>>, policy: &Policy) -> Result<()> {
    let manifest = file_json(files, "package.json")?;
    if !manifest.is_object() || manifest.get("workspaces").is_some() {
        bail!("island_package_boundary");
    }
    if manifest
        .get("packageManager")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.starts_with("npm@"))
    {
        bail!("island_package_manager");
    }
    let lock_name = selected_lock(files);
    let lock = file_json(files, lock_name)?;
    if !matches!(
        lock.get("lockfileVersion").and_then(Value::as_u64),
        Some(2 | 3)
    ) {
        bail!("island_lock_version");
    }
    validate_lock_packages(&lock, policy)?;
    validate_npmrc(files.get(".npmrc").and_then(Option::as_ref), policy)
}

fn selected_lock(files: &BTreeMap<String, Option<Vec<u8>>>) -> &'static str {
    if files
        .get("npm-shrinkwrap.json")
        .and_then(Option::as_ref)
        .is_some()
    {
        return "npm-shrinkwrap.json";
    }
    "package-lock.json"
}

fn file_json(files: &BTreeMap<String, Option<Vec<u8>>>, name: &str) -> Result<Value> {
    let bytes = files
        .get(name)
        .and_then(Option::as_ref)
        .context("island_input_missing")?;
    serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("island_input_json"))
}

fn validate_lock_packages(lock: &Value, policy: &Policy) -> Result<()> {
    let packages = lock
        .get("packages")
        .and_then(Value::as_object)
        .context("island_lock_packages")?;
    if !packages.contains_key("") {
        bail!("island_lock_root");
    }
    for (name, value) in packages {
        validate_dependency(name, value, policy)?;
    }
    Ok(())
}

fn validate_dependency(name: &str, value: &Value, policy: &Policy) -> Result<()> {
    if name.is_empty() {
        return Ok(());
    }
    if !inputs::registry_package_path(name)
        || value.get("link").and_then(Value::as_bool) == Some(true)
    {
        bail!("island_local_dependency");
    }
    let resolved = value
        .get("resolved")
        .and_then(Value::as_str)
        .context("island_resolved_missing")?;
    if !resolved.starts_with("https://")
        || resolved
            .trim_start_matches("https://")
            .split('/')
            .next()
            .is_some_and(|host| host.contains('@'))
        || resolved.contains(['?', '#'])
    {
        bail!("island_registry_url");
    }
    let pinned = value
        .get("integrity")
        .and_then(Value::as_str)
        .is_some_and(|value| value.starts_with("sha512-"));
    if !pinned && !policy.allow_local_attestation {
        bail!("island_local_attestation_required");
    }
    Ok(())
}

fn validate_npmrc(bytes: Option<&Vec<u8>>, policy: &Policy) -> Result<()> {
    let bytes = bytes.map_or(&[][..], Vec::as_slice);
    let text = std::str::from_utf8(bytes).context("island_npmrc_encoding")?;
    for line in text.lines().map(str::trim) {
        validate_config_line(line, policy)?;
    }
    Ok(())
}

fn validate_config_line(line: &str, policy: &Policy) -> Result<()> {
    if line.is_empty() || line.starts_with(['#', ';']) {
        return Ok(());
    }
    let (key, value) = line.split_once('=').context("island_npmrc_setting")?;
    let key = key.trim();
    let value = value.trim();
    if ["engine-strict", "legacy-peer-deps"].contains(&key) && ["true", "false"].contains(&value) {
        return Ok(());
    }
    if key == "registry" || key.ends_with(":registry") {
        return validate_config_registry(value);
    }
    if key.ends_with(":_authToken") && declared_placeholder(value, &policy.credential_env) {
        return Ok(());
    }
    bail!("island_npmrc_setting");
}

fn validate_config_registry(value: &str) -> Result<()> {
    if !value.starts_with("https://") || value.contains(['@', '?', '#', '$']) {
        bail!("island_registry_configuration");
    }
    Ok(())
}

fn declared_placeholder(value: &str, names: &[String]) -> bool {
    names.iter().any(|name| value == format!("${{{name}}}"))
}

fn selected_environment(policy: &Policy) -> Result<BTreeMap<String, Option<String>>> {
    policy
        .selected_env
        .iter()
        .map(|name| {
            let value = match std::env::var(name) {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                Err(std::env::VarError::NotUnicode(_)) => bail!("island_environment_encoding"),
            };
            Ok((name.clone(), value))
        })
        .collect()
}

fn request_key(
    files: &BTreeMap<String, Option<Vec<u8>>>,
    environment: &BTreeMap<String, Option<String>>,
    policy: &str,
    runtime: &str,
) -> Result<String> {
    let hashes: BTreeMap<_, _> = files
        .iter()
        .map(|(name, bytes)| {
            (
                name,
                bytes
                    .as_ref()
                    .map(|bytes| digest(&inputs::keyed_bytes(name, bytes))),
            )
        })
        .collect();
    Ok(digest(&serde_json::to_vec(&(
        hashes,
        environment,
        policy,
        runtime,
    ))?))
}

fn stage_file(destination: &Path, name: &str, bytes: Option<&Vec<u8>>) -> Result<()> {
    if let Some(bytes) = bytes {
        let path = destination.join(name);
        platform::plain_path(&path)?;
        fs::create_dir_all(path.parent().context("island_stage_parent")?)?;
        fs::write(path, inputs::keyed_bytes(name, bytes))?;
    }
    Ok(())
}

fn prepare_environment(command: &mut Command, runtime: &Path, node: &Path) -> Result<()> {
    let config = runtime.join("user.npmrc");
    let global = runtime.join("global.npmrc");
    fs::write(&config, b"")?;
    fs::write(&global, b"")?;
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("PATH", command_path(node)?);
    for name in ["HOME", "USERPROFILE", "TEMP", "TMP", "TMPDIR"] {
        command.env(name, runtime);
    }
    command
        .env("NPM_CONFIG_USERCONFIG", &config)
        .env("NPM_CONFIG_GLOBALCONFIG", &global)
        .env("NPM_CONFIG_WORKSPACES", "false")
        .env("NPM_CONFIG_CACHE", runtime.join("npm-cache"));
    Ok(())
}

fn set_selected_environment(command: &mut Command, environment: &BTreeMap<String, Option<String>>) {
    for (name, value) in environment {
        if let Some(value) = value {
            command.env(name, value);
        }
    }
}

fn set_credentials(command: &mut Command, names: &[String]) -> Result<()> {
    for name in names {
        command.env(
            name,
            std::env::var_os(name).context("island_credential_unavailable")?,
        );
    }
    Ok(())
}

#[allow(
    clippy::literal_string_with_formatting_args,
    reason = "The runtime placeholder belongs to the policy format, not Rust formatting"
)]
fn expand(value: &str, runtime: &Path) -> String {
    value.replace("{runtime}", &runtime.to_string_lossy())
}

fn command_path(node: &Path) -> Result<std::ffi::OsString> {
    let mut paths = vec![node.parent().context("island_node_parent")?.to_owned()];
    #[cfg(unix)]
    paths.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    #[cfg(windows)]
    paths.push(
        PathBuf::from(std::env::var_os("SystemRoot").context("island_system_root")?)
            .join("System32"),
    );
    Ok(std::env::join_paths(paths)?)
}
