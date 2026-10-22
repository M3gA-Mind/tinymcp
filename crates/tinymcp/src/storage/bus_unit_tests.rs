//! Tests for the TinyBus-backed storage provider's port wiring.

use super::*;

use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::Scope;

#[test]
fn callback_provider_uses_all_ports_from_the_host_storage_handle()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let storage = runtime
        .block_on(async { MemoryBackend::new().for_scope(&Scope::new("module-tinymcp")?) })?;
    let provider = BusProvider::from_storage_result(Ok(storage), None)?;
    let ports = provider.default_ports()?;
    ports.prepare()?;
    assert!(format!("{provider:?}").contains("BusProvider"));
    let error = BusProvider::from_storage_result(
        Err(tinystoragedrivers_types::StorageError::unavailable(
            "host is offline",
        )),
        None,
    )
    .err()
    .ok_or("unavailable callback storage was accepted")?;
    assert!(matches!(error, crate::error::Error::Store { .. }));
    Ok(())
}
