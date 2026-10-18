use super::*;

#[derive(Default, Debug)]
struct Client;
impl Retire for Client {
    fn mark_retiring(&self) {}
    fn is_retiring(&self) -> bool {
        false
    }
}

#[test]
fn a_removed_caller_waiting_for_the_cache_lock_cannot_be_admitted() {
    let table = Arc::new(ClientTable::<Client>::new());
    let removed = Arc::new(AtomicBool::new(false));
    let locked = lock(&table.clients);
    let (started, waiting) = std::sync::mpsc::channel();
    let request = {
        let table = Arc::clone(&table);
        let removed = Arc::clone(&removed);
        std::thread::spawn(move || {
            started.send(()).unwrap();
            table.admit(
                Path::new("/removed"),
                || removed.load(Ordering::Acquire),
                || Ok::<_, ()>(Arc::default()),
            )
        })
    };
    waiting.recv().unwrap();
    removed.store(true, Ordering::Release);
    drop(locked);
    assert_eq!(
        request.join().unwrap().unwrap_err(),
        AdmitError::Unavailable
    );
    assert!(table.snapshot().is_empty());
}
