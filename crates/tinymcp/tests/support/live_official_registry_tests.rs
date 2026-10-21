//! Unit tests for the live official-registry acceptance harness.

use super::support::launch_path;
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

    assert_eq!(launch_path(&launch), "/second/node/bin");
}
