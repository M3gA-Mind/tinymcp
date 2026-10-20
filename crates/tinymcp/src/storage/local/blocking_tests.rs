use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn bridge_runs_futures_from_sync_and_current_thread_runtime_contexts() -> TestResult {
    let bridge = Blocking::new()?;
    assert_eq!(bridge.run(async { 42 })?, 42);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    assert_eq!(
        runtime.block_on(async { bridge.run(async { "ready" }) })?,
        "ready"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_runs_from_a_multithread_worker() -> TestResult {
    let bridge = Blocking::new()?;
    assert_eq!(bridge.run(async { 7usize })?, 7);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_runs_from_a_spawn_blocking_thread() -> TestResult {
    let bridge = Blocking::new()?;
    assert_eq!(
        tokio::task::spawn_blocking(move || bridge.run(async { 11usize })).await??,
        11
    );
    Ok(())
}

#[test]
fn a_future_cannot_reenter_its_own_bridge() -> TestResult {
    let bridge = Blocking::new()?;
    let inner = bridge.clone();
    let inner_result = bridge.run(async move { inner.run(async {}) })?;
    let error = inner_result.err().ok_or("reentry would deadlock")?;
    assert_eq!(error.kind(), tinystoragedrivers_types::ErrorKind::Backend);
    Ok(())
}

#[test]
fn panicking_jobs_are_reported_to_the_waiting_caller() -> TestResult {
    let bridge = Blocking::new()?;
    let result = bridge.run(async { std::panic::panic_any("intentional bridge job failure") });
    let error = result.err().ok_or("panicking job should close its reply channel")?;
    assert_eq!(error.kind(), tinystoragedrivers_types::ErrorKind::Backend);
    Ok(())
}
