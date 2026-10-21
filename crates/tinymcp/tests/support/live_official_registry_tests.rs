//! Unit tests for the live official-registry acceptance harness.

use super::*;
use tinymcp::transport::stdio::StdioLaunch;

#[test]
fn uses_the_last_path_override_for_node_resolution() {
    let launch = StdioLaunch {
        program: "/second/node/bin/npx".into(),
        args: Vec::new(),
        env: vec![
            ("PATH".to_owned(), "/first/node/bin".to_owned()),
            ("HOME".to_owned(), "/tmp/home".to_owned()),
            ("PATH".to_owned(), "/second/node/bin".to_owned()),
        ],
        cwd: None,
    };

    assert_eq!(support::launch_path(&launch), "/second/node/bin");
}

#[test]
fn accepts_registry_package_specs_and_rejects_urls_or_paths() {
    assert!(support::is_registry_package_spec(
        "io.github.headlessherm-creator/polymath-megablaster-mcp"
    ));
    assert!(support::is_registry_package_spec(
        "@modelcontextprotocol/sdk@1.0.0"
    ));
    assert!(!support::is_registry_package_spec(
        "https://example.com/pkg.tgz"
    ));
    assert!(!support::is_registry_package_spec("../local-package"));
    assert!(!support::is_registry_package_spec(
        "git+https://example.com/pkg.git"
    ));
}

#[test]
#[allow(clippy::expect_used)]
fn resolves_host_executables_before_applying_the_sandbox_path() {
    let directory = tempfile::tempdir().expect("create temporary host PATH");
    let executable = directory.path().join("bwrap");
    std::fs::write(&executable, "host executable").expect("write fake host executable");

    assert_eq!(
        support::resolve_host_executable("bwrap", &directory.path().to_string_lossy())
            .expect("resolve host executable"),
        executable
            .canonicalize()
            .expect("canonicalize host executable")
    );
}
