use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use tinymcp::registry::sources::{
    McpOfficialRegistry, Registries, RegistrySource, RegistryTimeouts,
};
use tinymcp::registry::{McpRegistry, Store};
use tinymcp::transport::stdio::jailed::StdioLaunchers;
use tinymcp::transport::stdio::{StdioLaunch, StdioSpawner};
use tinymcp::{McpClientIdentityConfig, McpRegistryAuthConfig};

#[derive(Debug)]
struct IsolatedSpawner {
    workspace: PathBuf,
    stderr_path: PathBuf,
}

impl StdioSpawner for IsolatedSpawner {
    fn spawn(&self, launch: StdioLaunch) -> BoxFuture<'_, io::Result<tokio::process::Child>> {
        let workspace = self.workspace.clone();
        let stderr_path = self.stderr_path.clone();
        Box::pin(async move {
            let stderr = File::create(stderr_path)?;
            let mut args = vec![
                "--ro-bind".to_owned(),
                "/".to_owned(),
                "/".to_owned(),
                "--tmpfs".to_owned(),
                "/home".to_owned(),
                "--tmpfs".to_owned(),
                "/root".to_owned(),
                "--tmpfs".to_owned(),
                "/tmp".to_owned(),
                "--unshare-net".to_owned(),
                "--bind".to_owned(),
                workspace.to_string_lossy().into_owned(),
                "/sandbox".to_owned(),
                "--chdir".to_owned(),
                "/sandbox".to_owned(),
                "--".to_owned(),
                launch.program.to_string_lossy().into_owned(),
            ];
            args.extend(launch.args);
            let mut command = tokio::process::Command::new("bwrap");
            command
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::from(stderr))
                .kill_on_drop(true)
                .env_clear()
                .current_dir(&workspace);
            for (key, value) in &launch.env {
                let value = value.replace(&workspace.to_string_lossy().to_string(), "/sandbox");
                command.env(key, value);
            }
            if launch.program.file_name().is_some_and(|name| name == "npx") {
                command.env("npm_config_offline", "true");
            }
            command.spawn()
        })
    }
}

pub(crate) struct IsolatedRegistry {
    _scratch: tempfile::TempDir,
    pub(crate) registry: McpRegistry,
    pub(crate) catalog: Registries,
    pub(crate) catalog_store: Store,
    pub(crate) server_env: BTreeMap<String, String>,
    pub(crate) stderr_path: PathBuf,
    npm_cache: PathBuf,
}

impl IsolatedRegistry {
    #[allow(clippy::expect_used)]
    pub(crate) fn new() -> Self {
        let scratch = tempfile::tempdir().expect("create isolated npm home");
        let npm_cache = scratch.path().join("npm-cache");
        let uv_cache = scratch.path().join("uv-cache");
        let xdg_cache = scratch.path().join("cache");
        let temp_dir = scratch.path().join("tmp");
        for directory in [&npm_cache, &uv_cache, &xdg_cache, &temp_dir] {
            std::fs::create_dir_all(directory)
                .expect("create isolated cache and temporary directories");
        }
        let npm_user_config = scratch.path().join("npmrc");
        std::fs::write(&npm_user_config, "").expect("create empty npm user config");
        let stderr_path = scratch.path().join("child-stderr.log");
        let server_env = BTreeMap::from([
            (
                "HOME".to_owned(),
                scratch.path().to_string_lossy().into_owned(),
            ),
            ("TMPDIR".to_owned(), temp_dir.to_string_lossy().into_owned()),
            ("TMP".to_owned(), temp_dir.to_string_lossy().into_owned()),
            ("TEMP".to_owned(), temp_dir.to_string_lossy().into_owned()),
            (
                "XDG_CACHE_HOME".to_owned(),
                xdg_cache.to_string_lossy().into_owned(),
            ),
            (
                "UV_CACHE_DIR".to_owned(),
                uv_cache.to_string_lossy().into_owned(),
            ),
            (
                "NPM_CONFIG_CACHE".to_owned(),
                npm_cache.to_string_lossy().into_owned(),
            ),
            (
                "NPM_CONFIG_USERCONFIG".to_owned(),
                npm_user_config.to_string_lossy().into_owned(),
            ),
            (
                "PATH".to_owned(),
                std::env::var("PATH").expect("PATH is needed to resolve npx"),
            ),
        ]);
        let spawner = Arc::new(IsolatedSpawner {
            workspace: scratch.path().to_path_buf(),
            stderr_path: stderr_path.clone(),
        });
        let auth = McpRegistryAuthConfig::default();
        let registry = McpRegistry::new(
            Store::open_in_memory().expect("open in-memory store"),
            auth.clone(),
            McpClientIdentityConfig::default(),
            None,
        )
        .expect("build registry")
        .with_stdio(StdioLaunchers::sandboxed(spawner));
        // The exact `polymath` query took about 34 seconds during the live
        // preflight. Keep production defaults unchanged and give this
        // network-only acceptance client a measured 60-second budget.
        let catalog = Registries::with_official(
            auth,
            McpOfficialRegistry::with_timeouts(RegistryTimeouts {
                search: std::time::Duration::from_secs(60),
                detail: std::time::Duration::from_secs(60),
                ..RegistryTimeouts::default()
            })
            .expect("build official registry client with live-test budgets"),
        )
        .expect("build configured catalog dispatcher");
        let catalog_store = Store::open_in_memory().expect("open catalog cache");

        Self {
            _scratch: scratch,
            registry,
            catalog,
            catalog_store,
            server_env,
            stderr_path,
            npm_cache,
        }
    }

    pub(crate) async fn cache_npm_package(&self, package: &str) -> Result<(), String> {
        let mut command = tokio::process::Command::new("npm");
        command
            .args(["cache", "add", package, "--cache"])
            .arg(&self.npm_cache)
            .env_clear()
            .envs(&self.server_env);
        let output = tokio::time::timeout(std::time::Duration::from_secs(90), command.output())
            .await
            .map_err(|_| "npm package cache prefetch exceeded 90 seconds".to_owned())?
            .map_err(|error| format!("could not prefetch npm package: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "npm package cache prefetch failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

#[allow(clippy::expect_used, clippy::panic)]
pub(crate) async fn assert_search_lists(
    registries: &Registries,
    store: &Store,
    qualified_name: &str,
) {
    let first_page = registries
        .search(store, RegistrySource::McpOfficial, Some("polymath"), 1, 20)
        .await
        .expect("search official MCP registry for Polymath servers");
    if first_page
        .servers
        .iter()
        .any(|server| server.qualified_name == qualified_name)
    {
        return;
    }

    assert!(
        first_page.total_pages >= 2,
        "official registry search for `polymath` did not list {qualified_name}"
    );
    let second_page = registries
        .search(store, RegistrySource::McpOfficial, Some("polymath"), 2, 20)
        .await
        .expect("search second official registry page for Polymath servers");
    assert!(
        second_page
            .servers
            .iter()
            .any(|server| server.qualified_name == qualified_name),
        "official registry search for `polymath` did not list the verified package"
    );
}

pub(crate) fn child_diagnostics(path: &Path) -> String {
    let Ok(mut stderr) = File::open(path) else {
        return "stderr was unavailable".to_owned();
    };
    let Ok(length) = stderr.metadata().map(|metadata| metadata.len()) else {
        return "stderr size was unavailable".to_owned();
    };
    let start = length.saturating_sub(8 * 1024);
    if stderr.seek(SeekFrom::Start(start)).is_err() {
        return "stderr could not be read".to_owned();
    }
    let mut bytes = Vec::with_capacity(8 * 1024);
    if stderr.take(8 * 1024).read_to_end(&mut bytes).is_err() {
        return "stderr could not be read".to_owned();
    }
    String::from_utf8_lossy(&bytes).trim().to_owned()
}
