# Previous-release storage fixture

`tinymcp-v0.19.1-server.json` is the document body emitted by the public
`Store::insert_server` writer in TinyMCP v0.19.1, with its real v1.0.0
`tinystoragedrivers-core` dependency. The checked-in bytes came from running
`emit_previous_release_server_fixture`, injected into the archived release's
test module, plus that tag's TinyBus submodule:

```sh
mkdir -p target/legacy-writer-v0191/vendor/tinybus
git archive v0.19.1 | tar -x -C target/legacy-writer-v0191
git -C vendor/tinybus archive 1f33d1be1901a2d44bfe674cd7cf294cf3848821 \
  | tar -x -C target/legacy-writer-v0191/vendor/tinybus
python3 -c 'from pathlib import Path; p=Path("target/legacy-writer-v0191/crates/tinymcp/src/registry/store/mod_tests.rs"); s=p.read_text(); marker="#[test]\nfn a_record_from_before_the_enabled_field_loads_as_enabled()"; generator="""#[test]
fn emit_previous_release_server_fixture() {
    let store = store();
    store.insert_server(&stdio_server("released-fixture-server")).unwrap();
    let stored = store
        .get_raw(SERVERS, "released-fixture-server")
        .expect("the previous public writer persisted its server");
    println!("PREVIOUS_RELEASE_SERVER={}", serde_json::to_string(&stored).unwrap());
}

"""; assert marker in s; p.write_text(s.replace(marker, generator + marker, 1))'
set -o pipefail
TMPDIR="$PWD/target/legacy-writer-v0191/target/compiler-scratch" CARGO_BUILD_JOBS=1 \
  cargo test --locked \
  --manifest-path target/legacy-writer-v0191/Cargo.toml \
  -p tinymcp --lib emit_previous_release_server_fixture -- --nocapture \
  | sed -n 's/^PREVIOUS_RELEASE_SERVER=//p' \
  > crates/tinymcp/src/storage/fixtures/tinymcp-v0.19.1-server.json
```

The release tag resolves to `5f5eb4171fa73e6e7f8e869b4dc39ee41c5f3c6e`;
the TSD dependency resolves to v1.0.0 commit
`141fd9ed0ad5d14a8fb5a1b3e837a719e81ac08d`. The generator used the
released `Store::open_in_memory` and `Store::insert_server` APIs, then read the
stored body through the release's test-only raw read helper. The migration
regression opens the fixture through today's public `Store::open` path and
checks the typed readback; the separate v1 migration tests exercise legacy
collection conversion. The archived production source is unmodified; only its
test module is injected with the generator shown above. The checked-in fixture
SHA-256 is
`48d484e92efa488bf99da036254ea7f582e33307c649dcfd70ce5aa1eaebfb86`.
