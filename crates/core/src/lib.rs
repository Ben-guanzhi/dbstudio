pub mod ai;
pub mod models;
pub mod result;
pub mod schema;

/// Product name shown in the UI, window titles and logs.
pub const APP_NAME: &str = "dbstudio";

/// Namespace used for on-disk state (settings, SQLite database) and the OS
/// keyring service.
///
/// New and migrated installs store everything here; [`LEGACY_NAMESPACE`] data
/// is picked up by the one-time migrations in `dbstudio-storage` and the theme
/// loader.
pub const NAMESPACE: &str = "dbstudio";

/// File name of the local SQLite store inside [`NAMESPACE`].
pub const STORE_FILE_NAME: &str = "dbstudio.db";

/// Pre-rename namespace still read by the one-time migrations.
///
/// Existing installs keep their saved connections, query history, theme choice
/// and stored passwords under `dbclient`; the storage layer copies the SQLite
/// store and lazily re-exports keyring passwords into [`NAMESPACE`] on first
/// use. The legacy files/entries are never deleted automatically, so a failed
/// migration can always be retried from the untouched originals.
pub const LEGACY_NAMESPACE: &str = "dbclient";

/// File name of the local SQLite store inside [`LEGACY_NAMESPACE`].
pub const LEGACY_STORE_FILE_NAME: &str = "dbclient.db";
