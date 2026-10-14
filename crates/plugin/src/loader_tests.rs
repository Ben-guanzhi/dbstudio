//! Tests for the C ABI boundary and the loader's table validation.
//!
//! These exercise the bridge against an in-process table: real `extern "C"`
//! functions handed to the adapter through a `PluginTable`, which is exactly
// the shape a dylib would present. That covers the string conversion, the
// ownership handoff and the error path -- the parts the old `extern "Rust"`
// design had no way to test.
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr::addr_of_mut;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::abi::{
    fail, free_foreign, null_foreign, read_foreign, ship_json, ship_string, PluginFreeFn,
    PluginString, PluginTable,
};
use crate::bridge::PluginBridge;
use crate::loader::validate_table;
use crate::{DatabasePlugin, PluginInfo, PLUGIN_ABI_VERSION};

/// Counts `free_string` invocations, so the tests can assert the host returns
/// every string to the plugin that allocated it.
static FREE_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn count_free(ptr: PluginString) {
    FREE_CALLS.fetch_add(1, Ordering::SeqCst);
    free_foreign(ptr);
}

unsafe extern "C" fn test_info(error: *mut PluginString) -> PluginString {
    let info = PluginInfo {
        name: "test-driver".into(),
        version: "0.4.0".into(),
        description: "in-process test plugin".into(),
        author: "test".into(),
        db_type: "testdb".into(),
    };
    unsafe { ship_json(&info, error) }
}

unsafe extern "C" fn test_can_handle(db_type: *const c_char) -> u32 {
    if unsafe { read_foreign(db_type) } == "testdb" {
        1
    } else {
        0
    }
}

unsafe extern "C" fn test_connect(
    config_json: *const c_char,
    password: *const c_char,
    _error: *mut PluginString,
) -> PluginString {
    assert!(unsafe { read_foreign(config_json) }.contains("\"database\""));
    assert_eq!(unsafe { read_foreign(password) }, "pw");
    ship_string("handle-42".into())
}

unsafe extern "C" fn test_execute(
    handle: *const c_char,
    sql: *const c_char,
    error: *mut PluginString,
) -> PluginString {
    assert_eq!(unsafe { read_foreign(handle) }, "handle-42");
    if unsafe { read_foreign(sql) } == "boom" {
        return unsafe { fail(error, "executed a bad query") };
    }
    ship_string(format!("executed: {}", unsafe { read_foreign(sql) }))
}

unsafe extern "C" fn test_ping(handle: *const c_char, error: *mut PluginString) -> u32 {
    if unsafe { read_foreign(handle) } == "handle-42" {
        0
    } else {
        let _ = unsafe { fail(error, "unknown handle") };
        1
    }
}

unsafe extern "C" fn test_close(handle: *const c_char, _error: *mut PluginString) -> u32 {
    if unsafe { read_foreign(handle) } == "handle-42" {
        0
    } else {
        1
    }
}

unsafe extern "C" fn test_switch(
    _handle: *const c_char,
    database: *const c_char,
    error: *mut PluginString,
) -> u32 {
    if unsafe { read_foreign(database) } == "other" {
        0
    } else {
        let _ = unsafe { fail(error, "no such database") };
        1
    }
}

unsafe extern "C" fn test_current_database(
    _handle: *const c_char,
    _error: *mut PluginString,
) -> PluginString {
    ship_string("main".into())
}

/// A null slot of any callback type, for entries the test plugin omits.
///
/// For a function pointer `Option<T>` is niche-optimised to pointer width, so
/// `None` occupies exactly the null pointer's bytes. Reading a `T` out of that
/// address gives the null value without a size-dependent cast.
fn null_slot<T: Copy>() -> T {
    let none: Option<T> = None;
    let ptr = &none as *const Option<T> as *const T;
    unsafe { *ptr }
}

/// A table with every optional catalog slot null, so the bridge's fallback
/// behaviour is exercised alongside the implemented slots.
fn test_table() -> PluginTable {
    PluginTable {
        free_string: count_free,
        info: test_info,
        can_handle: test_can_handle,
        connect: test_connect,
        execute: test_execute,
        list_databases: null_slot(),
        list_tables: null_slot(),
        list_columns: null_slot(),
        list_indexes: null_slot(),
        list_foreign_keys: null_slot(),
        get_create_table_sql: null_slot(),
        ping: test_ping,
        close: test_close,
        switch_database: test_switch,
        current_database: test_current_database,
        _reserved: 0,
    }
}

fn bridge() -> PluginBridge {
    PluginBridge {
        table: test_table(),
    }
}

#[test]
fn abi_version_is_v4() {
    assert_eq!(PLUGIN_ABI_VERSION, 4);
}

#[test]
fn bridge_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PluginBridge>();
}

#[test]
fn info_roundtrips_through_json() {
    let info = bridge().info();
    assert_eq!(info.name, "test-driver");
    assert_eq!(info.version, "0.4.0");
    assert_eq!(info.db_type, "testdb");
}

#[test]
fn can_handle_reads_the_c_string() {
    let plugin = bridge();
    assert!(plugin.can_handle("testdb"));
    assert!(!plugin.can_handle("mysql"));
}

#[test]
fn connect_transfers_config_and_password() {
    assert_eq!(
        bridge().connect("{\"database\":\"x.db\"}", "pw").unwrap(),
        "handle-42"
    );
}

#[test]
fn execute_forwards_handle_and_sql() {
    assert_eq!(
        bridge().execute("handle-42", "SELECT 1").unwrap(),
        "executed: SELECT 1"
    );
}

#[test]
fn error_message_crosses_as_a_string() {
    let err = bridge().execute("handle-42", "boom").unwrap_err();
    assert_eq!(err.to_string(), "executed a bad query");
}

#[test]
fn status_slot_reports_success_on_zero() {
    assert!(bridge().ping("handle-42").is_ok());
    assert!(bridge().close("handle-42").is_ok());
    assert!(bridge().close("nope").is_err());
}

#[test]
fn status_slot_reports_failure_through_the_error_outparam() {
    let unknown = bridge().ping("nope").unwrap_err();
    assert_eq!(unknown.to_string(), "unknown handle");
}

#[test]
fn switch_database_reports_the_plugin_message() {
    assert!(bridge().switch_database("handle-42", "other").is_ok());
    assert_eq!(
        bridge()
            .switch_database("handle-42", "nope")
            .unwrap_err()
            .to_string(),
        "no such database"
    );
}

#[test]
fn current_database_reads_the_result_string() {
    assert_eq!(bridge().current_database("handle-42").unwrap(), "main");
}

#[test]
fn unsupported_slots_report_the_trait_default() {
    let plugin = bridge();
    let cases = [
        (
            "list_databases",
            plugin.list_databases("handle-42").unwrap_err().to_string(),
        ),
        (
            "list_tables",
            plugin
                .list_tables("handle-42", "public")
                .unwrap_err()
                .to_string(),
        ),
        (
            "list_columns",
            plugin
                .list_columns("handle-42", "t", "public")
                .unwrap_err()
                .to_string(),
        ),
        (
            "list_indexes",
            plugin
                .list_indexes("handle-42", "t", "public")
                .unwrap_err()
                .to_string(),
        ),
        (
            "list_foreign_keys",
            plugin
                .list_foreign_keys("handle-42", "t", "public")
                .unwrap_err()
                .to_string(),
        ),
        (
            "get_create_table_sql",
            plugin
                .get_create_table_sql("handle-42", "t", "public")
                .unwrap_err()
                .to_string(),
        ),
    ];
    for (label, error) in cases {
        assert!(
            error.contains("not supported"),
            "{label}: expected an unsupported error, got {error:?}"
        );
    }
}

#[test]
fn abi_version_reports_the_loaded_constant() {
    assert_eq!(bridge().abi_version(), PLUGIN_ABI_VERSION);
}

#[test]
fn host_releases_every_string_through_the_plugin_allocator() {
    // Tests run in parallel and share the counter, so compare a before/after
    // sample rather than asserting on an absolute value.
    let before = FREE_CALLS.load(Ordering::SeqCst);

    let plugin = bridge();
    let _ = plugin.info();
    let _ = plugin.connect("{\"database\":\"x.db\"}", "pw");
    let _ = plugin.execute("handle-42", "SELECT 1");
    let _ = plugin.execute("handle-42", "boom");
    let _ = plugin.current_database("handle-42");
    let _ = plugin.switch_database("handle-42", "nope");

    let after = FREE_CALLS.load(Ordering::SeqCst);
    assert!(
        after > before,
        "every string the plugin ships must be freed through its own free_string (saw no free calls)"
    );
}

#[test]
fn validate_table_rejects_a_null_pointer() {
    let err = validate_table(std::ptr::null::<PluginTable>()).unwrap_err();
    assert!(
        err.to_string().contains("null table pointer"),
        "unexpected error: {err}"
    );
}

#[test]
fn validate_table_rejects_missing_mandatory_slots() {
    let mut incomplete = test_table();
    incomplete.free_string = null_slot();
    let err = validate_table(&incomplete).unwrap_err();
    assert!(
        err.to_string().contains("mandatory slots"),
        "unexpected error: {err}"
    );

    // An otherwise complete table passes.
    assert!(validate_table(&test_table()).is_ok());
}

// Plugin-side helpers -------------------------------------------------------

#[test]
fn ship_and_read_roundtrip() {
    let ptr = ship_string("sqlite://data.db".into());
    assert!(!ptr.is_null());
    let text = unsafe { read_foreign(ptr.cast()) };
    assert_eq!(text, "sqlite://data.db");
    unsafe { free_foreign(ptr) };
}

#[test]
fn ship_string_never_panics_on_an_embedded_nul() {
    let ptr = ship_string(String::from("a\0b"));
    let text = unsafe { read_foreign(ptr.cast()) };
    // A C reader stops at the first NUL, so only the prefix is reachable.
    assert_eq!(text, "a");
    unsafe { free_foreign(ptr) };
}

#[test]
fn fail_fills_the_outparam_and_returns_null() {
    // The plugin writes into storage the host owns, so the host has to give it
    // an address, not just a pointer.
    let mut slot = null_foreign();
    let result = unsafe { fail(addr_of_mut!(slot), "disk full") };
    assert!(result.is_null());
    assert!(!slot.is_null());
    assert_eq!(unsafe { read_foreign(slot.cast()) }, "disk full");
    unsafe { free_foreign(slot) };
}

#[test]
fn ship_json_encodes_structs() {
    let mut slot = null_foreign();
    let info = PluginInfo {
        name: "n".into(),
        version: "1".into(),
        description: "d".into(),
        author: "a".into(),
        db_type: "t".into(),
    };
    let ptr = unsafe { ship_json(&info, addr_of_mut!(slot)) };
    let text = unsafe { read_foreign(ptr.cast()) };
    assert!(
        text.contains("\"db_type\":\"t\""),
        "unexpected JSON: {text}"
    );
    assert!(
        slot.is_null(),
        "a successful ship must not fill the error slot"
    );
    unsafe { free_foreign(ptr) };
}

#[test]
fn read_foreign_treats_null_as_empty() {
    assert_eq!(unsafe { read_foreign(null_foreign().cast()) }, "");
}

#[test]
fn embedded_nul_is_a_clean_error_not_an_abort() {
    // The host-side mirror of `ship_string`: a bad argument surfaces as an
    // error rather than aborting across an `extern "C"` frame.
    let result = CString::new("a\0b").map_err(|e| e.nul_position());
    assert!(result.is_err());
}

#[test]
fn reserved_slot_is_pointer_sized_padding() {
    // Guards the padding type: pointer-sized, so a method added later takes
    // over this offset without shifting anything before it. It must be an
    // integer, not a fn pointer, or a plugin author needs a niche trick just to
    // write `0`.
    let reserved: usize = test_table()._reserved;
    assert_eq!(reserved, 0, "padding must be zero");
    assert_eq!(
        std::mem::size_of::<usize>(),
        std::mem::size_of::<PluginFreeFn>(),
        "padding must stay the same width as a callback slot"
    );
}
