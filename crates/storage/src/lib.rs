pub mod ai_settings;
pub mod connections;
pub mod favorites;
pub mod history;
pub mod settings;
pub mod store;
pub mod types;

/// Serializes keyring access inside the test suite.
///
/// The OS credential store is process-global: concurrent `set_password` /
/// `delete_credential` calls from different tests can leave a credential
/// unreadable even when the tests use different ids. Every test that touches
/// the keyring must hold this guard.
#[cfg(test)]
pub fn keyring() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};

    static KEYRING_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = KEYRING_LOCK.get_or_init(Mutex::default);
    // A panicked holder still left the lock usable; recover rather than
    // turning one test's panic into every other keyring test failing.
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub use store::AppStore;
