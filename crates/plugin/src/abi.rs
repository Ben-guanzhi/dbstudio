/// The C ABI that a driver plugin compiles against.
///
/// ## Why a C boundary
///
/// The previous boundary was `extern "Rust" fn create_plugin() -> Box<dyn
/// DatabasePlugin>` with methods returning `anyhow::Result<String>`. None of
/// that is stable:
///
/// - `extern "Rust"` is rustc's internal ABI and may change at any time, so a
///   plugin built with one toolchain and a host built with another is undefined
///   behaviour at the first call, with no diagnosable symptom.
/// - `Box<dyn DatabasePlugin>` is two pointers, but the vtable slot order is
///   only accidentally identical when both sides compiled the same trait
///   definition.
/// - `anyhow::Error` is a `Box<dyn Error + Send + Sync>` whose `Display`
///   dispatch goes through the *plugin's* `std::error::Error` object layout —
///   no stability promise across toolchains.
/// - `String` crosses an allocator. The plugin allocates it, the host frees it.
///   That survives on Linux and macOS by lucky symbol interposition and is the
///   classic "invalid pointer" crash on Windows, where separate DLL exports are
///   not interposed the same way.
///
/// So nothing Rust-shaped crosses this boundary. The wire holds exactly four
/// kinds of value: `*const c_char`, `*mut c_char`, `u32` and a callback
/// pointer.
///
/// ## The three rules that keep it honest
///
/// 1. **Ownership is encoded in the pointer type, not in a convention.**
///    A `*mut c_char` returned by a slot was allocated by the *plugin*, so the
///    host must release it with the plugin's own `free_string` slot. The two
///    allocators never mix.
/// 2. **Errors are strings, not `anyhow::Error`.** The rich error type is a
///    host-side concept; on the wire it is a message. Nothing is lost — an
///    `anyhow::Error` already reached the host as an opaque box whose internals
///    could not be inspected anyway — but the plugin keeps its full error chain
///    internally and ships only what the caller can display.
/// 3. **Versioning is probed before anything is called.** The host reads
///    `dbstudio_plugin_abi_version` (a plain `fn() -> u32`, layout-free)
///    before dereferencing the table, so a mismatched layout can never be
///    interpreted.
///
/// ## Growing the table
///
/// Slots are appended, never reordered, and unused trailing slots hold a null
/// pointer. The host treats a null slot as "not implemented" and falls back to
/// the trait default, so a new method does not force an ABI bump — the failure
/// mode v2 had to pay for.
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

/// A plugin-allocated, NUL-terminated UTF-8 string.
///
/// Release with the exporting plugin's `free_string` slot. Never with the
/// host's allocator, and never twice.
pub type PluginString = *mut c_char;

/// Plugin-side: own a string across the boundary without leaking.
pub fn ship_string(value: String) -> PluginString {
    match CString::new(value) {
        Ok(c) => c.into_raw(),
        // An embedded NUL truncates the string at the first one — the only
        // representation a C reader can consume. JSON never contains a raw
        // NUL, so this is unreachable in practice, but it must not panic
        // across an FFI boundary.
        Err(e) => {
            let cut = e.nul_position();
            let mut bytes = e.into_vec();
            bytes.truncate(cut);
            // Sound here: the truncation removed every NUL, which is the only
            // precondition `from_vec_unchecked` needs.
            unsafe { CString::from_vec_unchecked(bytes) }.into_raw()
        }
    }
}

/// Plugin-side: serialize a value to JSON and hand it over. Returns null (and
/// fills `error` when it is non-null) if serialization fails.
///
/// Panics never cross an `extern "C"` frame — Rust aborts the process — so this
/// degrades to a failure instead of risking that.
pub fn ship_json<T: serde::Serialize>(value: &T, error: *mut PluginString) -> PluginString {
    match serde_json::to_string(value) {
        Ok(json) => ship_string(json),
        Err(e) => fail(error, e.to_string()),
    }
}

/// Plugin-side: report a failure. Fills `error` and returns the null sentinel
/// that every slot uses to mean "the call did not succeed".
pub fn fail(error: *mut PluginString, message: impl Into<String>) -> PluginString {
    if !error.is_null() {
        unsafe { *error = ship_string(message.into()) };
    }
    null_foreign()
}

/// Plugin-side: release a string the plugin shipped and the host has finished
/// with. Export this as the table's `free_string` slot.
///
/// # Safety
///
/// `ptr` must have come from `ship_string` in this same plugin, or be null.
pub unsafe extern "C" fn free_foreign(ptr: PluginString) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

/// Plugin-side: read a host-provided string. Null or invalid UTF-8 becomes the
/// empty string rather than a fault — the host sends empty for absent values.
pub fn read_foreign<'a>(ptr: *const c_char) -> std::borrow::Cow<'a, str> {
    if ptr.is_null() {
        std::borrow::Cow::Borrowed("")
    } else {
        std::borrow::Cow::Owned(
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// A null string pointer: the universal failure sentinel on this wire.
#[inline]
pub fn null_foreign() -> PluginString {
    std::ptr::null_mut()
}

/// An empty error out-parameter, null until the plugin fills it.
///
/// Distinct from [`null_foreign`]: the out-parameter is a *pointer to a
/// string pointer*, since the plugin writes its message into it.
#[inline]
pub fn null_error_out() -> *mut PluginString {
    std::ptr::null_mut()
}

// ── Slot signatures ────────────────────────────────────────────────────────
//
// All of these are `unsafe extern "C"` deliberately: the arguments are raw
// pointers with no layout guarantee to check, and the results need freeing.

/// Frees a `PluginString` this plugin allocated. Mandatory.
pub type PluginFreeFn = unsafe extern "C" fn(PluginString);

/// Whether the plugin claims a database type. `1` = yes, `0` = no.
pub type PluginCanHandleFn = unsafe extern "C" fn(db_type: *const c_char) -> u32;

/// Plugin metadata as JSON. Mandatory.
pub type PluginInfoFn = unsafe extern "C" fn(error: *mut PluginString) -> PluginString;

/// Open a connection. Returns the handle as a **plain C string**, or null on
/// failure. Not JSON-encoded: the host stores this token verbatim and sends it
/// back unchanged on every later call.
pub type PluginConnectFn = unsafe extern "C" fn(
    config_json: *const c_char,
    password: *const c_char,
    error: *mut PluginString,
) -> PluginString;

/// Run SQL and return a result as JSON.
pub type PluginExecFn = unsafe extern "C" fn(
    handle: *const c_char,
    sql: *const c_char,
    error: *mut PluginString,
) -> PluginString;

/// A catalog slot taking only the handle.
///
/// Payload encoding is per-slot, not per-signature, because two slots share
/// this type: `list_databases` returns a JSON array of databases, while
/// `current_database` returns the database name as a plain C string. Check the
/// host's decoding of the slot you implement, not the signature of your
/// neighbours.
pub type PluginHandleOnlyFn =
    unsafe extern "C" fn(handle: *const c_char, error: *mut PluginString) -> PluginString;

/// A catalog slot taking one further string argument.
///
/// `list_tables` returns JSON; nothing else uses this shape yet.
pub type PluginOneArgFn = unsafe extern "C" fn(
    handle: *const c_char,
    arg: *const c_char,
    error: *mut PluginString,
) -> PluginString;

/// A catalog slot taking two further string arguments (table and schema).
///
/// `list_columns`, `list_indexes` and `list_foreign_keys` return JSON arrays.
/// `get_create_table_sql` returns the DDL as a plain C string: the host shows
/// it in an editor rather than decoding it, so JSON-quoting it would leak into
/// the display.
pub type PluginTwoArgFn = unsafe extern "C" fn(
    handle: *const c_char,
    table: *const c_char,
    schema: *const c_char,
    error: *mut PluginString,
) -> PluginString;

/// A void-returning slot. `0` = success; anything else is a failure whose
/// message is in `error`.
pub type PluginStatusFn =
    unsafe extern "C" fn(handle: *const c_char, error: *mut PluginString) -> u32;

/// A void-returning slot with one further string argument.
pub type PluginActionFn = unsafe extern "C" fn(
    handle: *const c_char,
    arg: *const c_char,
    error: *mut PluginString,
) -> u32;

/// Reserved width of [`PluginTable`]. The last slot is padding so that adding a
/// method appends inside an existing reserved slot instead of growing the
/// struct — growing it would change the layout every plugin was compiled
/// against. Never index beyond the slots the host declares.
pub const TABLE_SLOTS: usize = 16;

/// The plugin's export surface: one versioned function-pointer table.
///
/// A plugin exports it as:
///
/// ```ignore
/// #[unsafe(no_mangle)]
/// pub extern "C" fn dbstudio_plugin_entry() -> *const PluginTable {
///     std::ptr::addr_of!(TABLE)
/// }
/// ```
///
/// Fields are callbacks, not data, so `PluginTable` is `Send` and `Sync` by
/// construction: there is no plugin memory behind the struct itself, only
/// addresses into the exporting module's text segment.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PluginTable {
    /// Mandatory. Frees every string this plugin returned.
    pub free_string: PluginFreeFn,
    /// Mandatory.
    pub info: PluginInfoFn,
    /// Mandatory.
    pub can_handle: PluginCanHandleFn,
    pub connect: PluginConnectFn,
    pub execute: PluginExecFn,
    pub list_databases: PluginHandleOnlyFn,
    pub list_tables: PluginOneArgFn,
    pub list_columns: PluginTwoArgFn,
    pub list_indexes: PluginTwoArgFn,
    pub list_foreign_keys: PluginTwoArgFn,
    pub get_create_table_sql: PluginTwoArgFn,
    pub ping: PluginStatusFn,
    pub close: PluginStatusFn,
    pub switch_database: PluginActionFn,
    pub current_database: PluginHandleOnlyFn,

    // Padding, must be zero. It is a plain integer rather than a function
    // pointer so that a plugin author writes `0` and not a null fn pointer,
    // which Rust has no convenient way to spell. A method added later takes
    // over this offset as a callback; the host casts `usize` to whatever
    // signature it needs, which is a legal cast.
    pub _reserved: usize,
}
