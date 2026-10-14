use crate::components::help_modal::HelpModal;

#[test]
fn shortcut_sections_have_non_empty_rows() {
    let sections = HelpModal::sections();
    assert!(!sections.is_empty());
    for (title, rows) in &sections {
        assert!(!title.trim().is_empty());
        assert!(!rows.is_empty());
        for (keys, desc) in rows {
            assert!(!keys.trim().is_empty());
            assert!(!desc.trim().is_empty());
        }
    }
}

#[test]
fn help_documents_f1_and_quick_open() {
    let rows: Vec<(String, String)> = HelpModal::sections()
        .into_iter()
        .flat_map(|(_, rows)| rows)
        .collect();
    assert!(rows.iter().any(|(keys, _)| keys == "F1"));
    assert!(rows
        .iter()
        .any(|(keys, desc)| keys.ends_with("+K") && desc.contains("command palette")));
    assert!(rows
        .iter()
        .any(|(keys, desc)| keys.ends_with("+P") && desc.contains("Quick open")));
}
