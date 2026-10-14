//! The host-side adapter: a C [`PluginTable`] presented as a
//! [`DatabasePlugin`].
//!
//! The trait itself is untouched. It is a host-internal contract shared with
//! `dbstudio-db`, so it owes no stability guarantees — and keeping it
//! stable-free means the C boundary above is the *only* place that has to be.
use std::ffi::{CStr, CString};

use std::ptr::addr_of_mut;

use anyhow::{anyhow, Result};

use crate::abi::{null_foreign, PluginFreeFn, PluginString, PluginTable};
use crate::{DatabasePlugin, PluginInfo, PLUGIN_ABI_VERSION};

/// A string a plugin handed to the host. Dropping it calls the plugin's own
/// `free_string`, so the allocator that allocated it is the one that releases
/// it — including on the panic path, which is why this is a `Drop` guard rather
/// than a manual "free after use" step.
struct ForeignString {
    ptr: PluginString,
    free: PluginFreeFn,
}

impl ForeignString {
    fn new(ptr: PluginString, free: PluginFreeFn) -> Self {
        Self { ptr, free }
    }

    /// Consume the pointer, returning an owned string. Takes ownership, so the
    /// `Drop` impl becomes a no-op.
    fn into_owned(mut self) -> String {
        if self.ptr.is_null() {
            return String::new();
        }
        let text = unsafe { CStr::from_ptr(self.ptr.cast()) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.free)(self.ptr) };
        self.ptr = null_foreign();
        text
    }
}

impl Drop for ForeignString {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { (self.free)(self.ptr) };
            self.ptr = null_foreign();
        }
    }
}

/// Turn a host `&str` into a NUL-terminated argument. Fails rather than panics,
/// because a panic would abort the process if it ever reached an `extern "C"`
/// frame.
fn to_cstring(value: &str) -> Result<CString> {
    CString::new(value)
        .map_err(|e| anyhow!("embedded NUL in argument at byte {}", e.nul_position()))
}

/// Read a slot's `(result, error)` pair into an owned `String`.
///
/// A non-null `result` is the payload. A null `result` means failure, and the
/// message lives in `error` — which the plugin may or may not have filled, so
/// an unfilled out-parameter still yields a readable error.
fn read_result(result: PluginString, error: PluginString, free: PluginFreeFn) -> Result<String> {
    if !result.is_null() {
        return Ok(ForeignString::new(result, free).into_owned());
    }
    let message = if error.is_null() {
        String::from("plugin returned no result")
    } else {
        ForeignString::new(error, free).into_owned()
    };
    if message.is_empty() {
        Err(anyhow!("plugin call failed without an error message"))
    } else {
        Err(anyhow!(message))
    }
}

/// A connection to a database driver loaded from a C ABI table.
///
/// `PluginTable` holds only function pointers, so `Send` and `Sync` are derived
/// for free: there is no plugin memory behind the struct to move between
/// threads. Whether it is *safe* to call those pointers concurrently is the
/// plugin's contract, not something the type system can promise.
pub struct PluginBridge {
    pub table: PluginTable,
}

impl PluginBridge {
    /// Slot absent, so the plugin did not implement it: report it the same way
    /// the trait default does, keeping host-side error text uniform.
    fn unsupported(&self, method: &str) -> Result<String> {
        Err(anyhow!("{method} not supported by this plugin"))
    }

    /// The `Result<()>` form of [`Self::unsupported`], for the status slots that
    /// carry no payload.
    fn unsupported_status(&self, method: &str) -> Result<()> {
        Err(anyhow!("{method} not supported by this plugin"))
    }

    /// Status slots report success as `0` and carry the message in `error`
    /// otherwise. There is no payload to read, so `read_result` does not apply.
    fn read_status(status: u32, error: PluginString, free: PluginFreeFn) -> Result<()> {
        if status == 0 {
            return Ok(());
        }
        let message = if error.is_null() {
            String::new()
        } else {
            ForeignString::new(error, free).into_owned()
        };
        if message.is_empty() {
            Err(anyhow!("plugin call failed without an error message"))
        } else {
            Err(anyhow!(message))
        }
    }
}

macro_rules! two_arg_slot {
    ($self:expr, $slot:ident, $handle:expr, $first:expr, $second:expr, $method:literal) => {{
        let f = $self.table.$slot;
        if f as usize == 0 {
            return $self.unsupported($method);
        }
        let h = to_cstring($handle)?;
        let a = to_cstring($first)?;
        let b = to_cstring($second)?;
        let mut error = null_foreign();
        let result = unsafe { f(h.as_ptr(), a.as_ptr(), b.as_ptr(), addr_of_mut!(error)) };
        read_result(result, error, $self.table.free_string)
    }};
}

macro_rules! one_arg_slot {
    ($self:expr, $slot:ident, $handle:expr, $arg:expr, $method:literal) => {{
        let f = $self.table.$slot;
        if f as usize == 0 {
            return $self.unsupported($method);
        }
        let h = to_cstring($handle)?;
        let a = to_cstring($arg)?;
        let mut error = null_foreign();
        let result = unsafe { f(h.as_ptr(), a.as_ptr(), addr_of_mut!(error)) };
        read_result(result, error, $self.table.free_string)
    }};
}

unsafe impl DatabasePlugin for PluginBridge {
    fn abi_version(&self) -> u32 {
        // Informational: the loader already verified the version through the
        // `dbstudio_plugin_abi_version` C symbol before constructing this.
        PLUGIN_ABI_VERSION
    }

    fn info(&self) -> PluginInfo {
        let f = self.table.info;
        if f as usize == 0 {
            return PluginInfo {
                name: String::from("unknown-plugin"),
                version: String::new(),
                description: String::new(),
                author: String::new(),
                db_type: String::new(),
            };
        }
        let mut error = null_foreign();
        let result = unsafe { f(addr_of_mut!(error)) };
        match read_result(result, error, self.table.free_string) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                tracing::warn!("Plugin 'info' returned unparseable JSON: {e}");
                PluginInfo {
                    name: String::from("unknown-plugin"),
                    version: String::new(),
                    description: String::new(),
                    author: String::new(),
                    db_type: String::new(),
                }
            }),
            Err(e) => {
                tracing::warn!("Plugin 'info' failed: {e}");
                PluginInfo {
                    name: String::from("unknown-plugin"),
                    version: String::new(),
                    description: String::new(),
                    author: String::new(),
                    db_type: String::new(),
                }
            }
        }
    }

    fn can_handle(&self, db_type: &str) -> bool {
        let f = self.table.can_handle;
        if f as usize == 0 {
            return false;
        }
        match to_cstring(db_type) {
            Ok(c) => unsafe { f(c.as_ptr()) != 0 },
            Err(_) => false,
        }
    }

    fn connect(&self, config: &str, password: &str) -> Result<String> {
        let f = self.table.connect;
        if f as usize == 0 {
            return self.unsupported("connect");
        }
        let cfg = to_cstring(config)?;
        let pw = to_cstring(password)?;
        let mut error = null_foreign();
        let result = unsafe { f(cfg.as_ptr(), pw.as_ptr(), addr_of_mut!(error)) };
        read_result(result, error, self.table.free_string)
    }

    fn execute(&self, handle: &str, sql: &str) -> Result<String> {
        let f = self.table.execute;
        if f as usize == 0 {
            return self.unsupported("execute");
        }
        let h = to_cstring(handle)?;
        let s = to_cstring(sql)?;
        let mut error = null_foreign();
        let result = unsafe { f(h.as_ptr(), s.as_ptr(), addr_of_mut!(error)) };
        read_result(result, error, self.table.free_string)
    }

    fn list_databases(&self, handle: &str) -> Result<String> {
        let f = self.table.list_databases;
        if f as usize == 0 {
            return self.unsupported("list_databases");
        }
        let h = to_cstring(handle)?;
        let mut error = null_foreign();
        let result = unsafe { f(h.as_ptr(), addr_of_mut!(error)) };
        read_result(result, error, self.table.free_string)
    }

    fn list_tables(&self, handle: &str, schema: &str) -> Result<String> {
        one_arg_slot!(self, list_tables, handle, schema, "list_tables")
    }

    fn list_columns(&self, handle: &str, table: &str, schema: &str) -> Result<String> {
        two_arg_slot!(self, list_columns, handle, table, schema, "list_columns")
    }

    fn list_indexes(&self, handle: &str, table: &str, schema: &str) -> Result<String> {
        two_arg_slot!(self, list_indexes, handle, table, schema, "list_indexes")
    }

    fn list_foreign_keys(&self, handle: &str, table: &str, schema: &str) -> Result<String> {
        two_arg_slot!(
            self,
            list_foreign_keys,
            handle,
            table,
            schema,
            "list_foreign_keys"
        )
    }

    fn get_create_table_sql(&self, handle: &str, table: &str, schema: &str) -> Result<String> {
        two_arg_slot!(
            self,
            get_create_table_sql,
            handle,
            table,
            schema,
            "get_create_table_sql"
        )
    }

    fn switch_database(&self, handle: &str, database: &str) -> Result<()> {
        let f = self.table.switch_database;
        if f as usize == 0 {
            return self.unsupported_status("switch_database");
        }
        let h = to_cstring(handle)?;
        let db = to_cstring(database)?;
        let mut error = null_foreign();
        let status = unsafe { f(h.as_ptr(), db.as_ptr(), addr_of_mut!(error)) };
        Self::read_status(status, error, self.table.free_string)
    }

    fn current_database(&self, handle: &str) -> Result<String> {
        let f = self.table.current_database;
        if f as usize == 0 {
            return self.unsupported("current_database");
        }
        let h = to_cstring(handle)?;
        let mut error = null_foreign();
        let result = unsafe { f(h.as_ptr(), addr_of_mut!(error)) };
        read_result(result, error, self.table.free_string)
    }

    fn ping(&self, handle: &str) -> Result<()> {
        let f = self.table.ping;
        if f as usize == 0 {
            return self.unsupported_status("ping");
        }
        let h = to_cstring(handle)?;
        let mut error = null_foreign();
        let status = unsafe { f(h.as_ptr(), addr_of_mut!(error)) };
        Self::read_status(status, error, self.table.free_string)
    }

    fn close(&self, handle: &str) -> Result<()> {
        let f = self.table.close;
        if f as usize == 0 {
            return self.unsupported_status("close");
        }
        let h = to_cstring(handle)?;
        let mut error = null_foreign();
        let status = unsafe { f(h.as_ptr(), addr_of_mut!(error)) };
        Self::read_status(status, error, self.table.free_string)
    }
}
