#![recursion_limit = "256"]

pub mod components;
pub mod state;
pub mod url_scheme;
pub mod url_scheme_registry;
pub mod utils;

#[cfg(test)]
mod filter_tests {
    use crate::components::results_panel::{
        cell_matches_filter, contains_ignore_case, like_match, operators_for,
        starts_with_ignore_case, ColumnFilter, FilterOp, LikePattern, PreparedFilter,
    };
    use dbstudio_core::result::{CellType, ResultCell};

    #[test]
    fn equality_and_contains_are_case_insensitive() {
        assert!(cell_matches_filter(
            Some(&ResultCell::text("Alice")),
            FilterOp::Equals,
            "alice"
        ));
        assert!(!cell_matches_filter(
            Some(&ResultCell::text("Alice")),
            FilterOp::Equals,
            "bob"
        ));
        assert!(cell_matches_filter(
            Some(&ResultCell::text("Hello World")),
            FilterOp::Contains,
            "WORLD"
        ));
        assert!(!cell_matches_filter(
            Some(&ResultCell::text("Hello World")),
            FilterOp::Contains,
            "foo"
        ));
        assert!(cell_matches_filter(
            Some(&ResultCell::text("Hello World")),
            FilterOp::StartsWith,
            "hello"
        ));
    }

    #[test]
    fn null_cells_only_match_emptiness_operators() {
        let null = ResultCell::null();
        assert!(cell_matches_filter(Some(&null), FilterOp::IsEmpty, ""));
        assert!(!cell_matches_filter(Some(&null), FilterOp::NotEmpty, ""));
        assert!(!cell_matches_filter(Some(&null), FilterOp::Equals, ""));
        assert!(!cell_matches_filter(Some(&null), FilterOp::Contains, "x"));
        assert!(!cell_matches_filter(None, FilterOp::IsEmpty, ""));
    }

    #[test]
    fn like_supports_percent_and_underscore() {
        assert!(like_match("hello world", "hello%"));
        assert!(like_match("hello world", "%world"));
        assert!(like_match("hello world", "%llo w%"));
        assert!(like_match("abc", "a_c"));
        assert!(!like_match("abc", "a__c"));
        assert!(like_match("hello world", "%o_w%"));
    }

    /// Reference matcher for `LIKE`, written the way the retired
    /// dynamic-programming table read: `%` may consume any prefix of what
    /// follows it. The two-pointer scan in `LikePattern` must accept exactly
    /// this language.
    fn like_reference(value: &[char], pattern: &[char]) -> bool {
        match pattern.first() {
            None => value.is_empty(),
            Some('%') => (0..=value.len()).any(|k| like_reference(&value[k..], &pattern[1..])),
            Some(head) => {
                !value.is_empty()
                    && (*head == '_' || *head == value[0])
                    && like_reference(&value[1..], &pattern[1..])
            }
        }
    }

    fn extend(alphabet: &[char], prefix: String, max_len: usize, out: &mut Vec<String>) {
        out.push(prefix.clone());
        if prefix.len() < max_len {
            for c in alphabet {
                let mut next = prefix.clone();
                next.push(*c);
                extend(alphabet, next, max_len, out);
            }
        }
    }

    #[test]
    fn like_matcher_agrees_with_the_reference_over_a_bounded_corpus() {
        let mut values = Vec::new();
        extend(&['a', 'b'], String::new(), 4, &mut values);
        let mut patterns = Vec::new();
        extend(&['a', 'b', '%', '_'], String::new(), 3, &mut patterns);

        for value in &values {
            for pattern in &patterns {
                let v: Vec<char> = value.chars().collect();
                let p: Vec<char> = pattern.chars().collect();
                let expected = like_reference(&v, &p);
                assert_eq!(
                    like_match(value, pattern),
                    expected,
                    "like_match({value:?}, {pattern:?})"
                );
                assert_eq!(
                    LikePattern::new(pattern).matches(value),
                    expected,
                    "prepared pattern {value:?} / {pattern:?}"
                );
            }
        }
    }

    #[test]
    fn prepared_like_filter_matches_the_per_cell_path_and_skips_nulls() {
        let filter = ColumnFilter {
            column: 0,
            op: FilterOp::Like,
            value: "%o_w%".to_string(),
        };
        let prepared = PreparedFilter::prepare(&filter);

        assert!(prepared.matches(Some(&ResultCell::text("hello world"))));
        assert!(!prepared.matches(Some(&ResultCell::text("xyz"))));
        // Identical verdict to the unprepared path, which the grid also uses.
        for text in ["hello world", "abc", "", "%o_w%"] {
            let cell = ResultCell::text(text);
            assert_eq!(
                prepared.matches(Some(&cell)),
                cell_matches_filter(Some(&cell), FilterOp::Like, &filter.value),
                "prepared vs direct for {text:?}"
            );
        }
        assert!(!prepared.matches(Some(&ResultCell::null())));
        assert!(!prepared.matches(None));
        // Case folding happens on both sides, as it did in the table version.
        assert!(PreparedFilter::prepare(&ColumnFilter {
            column: 0,
            op: FilterOp::Like,
            value: "HELLO%".to_string(),
        })
        .matches(Some(&ResultCell::text("hello world"))));
    }

    #[test]
    fn prepared_filters_pass_through_other_operators() {
        let prepared = PreparedFilter::prepare(&ColumnFilter {
            column: 2,
            op: FilterOp::GreaterThan,
            value: "5".to_string(),
        });
        assert!(prepared.matches(Some(&ResultCell::integer(10))));
        assert!(!prepared.matches(Some(&ResultCell::integer(3))));

        // A non-numeric expectation still falls back to substring matching, and
        // the prepared wrapper must not change that.
        let loose = PreparedFilter::prepare(&ColumnFilter {
            column: 2,
            op: FilterOp::GreaterThan,
            value: "10y".to_string(),
        });
        let cell = ResultCell::text("x10y");
        assert!(loose.matches(Some(&cell)));
        assert!(cell_matches_filter(
            Some(&cell),
            FilterOp::GreaterThan,
            "10y"
        ));
    }

    #[test]
    fn numeric_operators_use_sort_proxy() {
        assert!(cell_matches_filter(
            Some(&ResultCell::integer(10)),
            FilterOp::GreaterThan,
            "5"
        ));
        assert!(cell_matches_filter(
            Some(&ResultCell::integer(10)),
            FilterOp::LessThan,
            "11"
        ));
        assert!(!cell_matches_filter(
            Some(&ResultCell::float(1.5)),
            FilterOp::GreaterThan,
            "2"
        ));
        // A non-numeric expected value falls back to a substring match.
        assert!(cell_matches_filter(
            Some(&ResultCell::text("x10y")),
            FilterOp::GreaterThan,
            "10y"
        ));
        // Numeric ops against non-numeric cells with numeric values match nothing.
        assert!(!cell_matches_filter(
            Some(&ResultCell::text("x10y")),
            FilterOp::GreaterThan,
            "10"
        ));
    }

    /// Values chosen to cover the cases a per-char fold gets wrong: the only
    /// Unicode code point whose lowercase mapping is longer than one char
    /// (`U+0130`), a final sigma that must *not* change, a title-case letter,
    /// an already-combining mark, plus ASCII / CJK / emoji / empty.
    const CI_VALUES: &[&str] = &[
        "",
        "a",
        "A",
        "Order_Item_7",
        "ORDER_ITEM_7",
        "İstanbul",
        "i̇stanbul",
        "İ",
        "i",
        "ß",
        "ẞ",
        "STRASSE",
        "straße",
        "ΣΣΣ",
        "σσς",
        "ǅ",
        "Ǆ",
        "中文表格",
        "🙂value🙂",
        "İİ",
        "aİb",
        "İi̇",
        "ſ",
        "ſſ",
        "μ",
        "12345",
        "İ12",
    ];

    fn ci_pairs() -> Vec<(String, String)> {
        let mut out = Vec::new();
        for v in CI_VALUES {
            for n in CI_VALUES {
                out.push((v.to_string(), n.to_string()));
            }
            // Suffixes and interior slices, so both hits and misses are dense.
            let chars: Vec<char> = v.chars().collect();
            for start in 0..chars.len() {
                for end in start + 1..=chars.len() {
                    out.push((v.to_string(), chars[start..end].iter().collect()));
                }
            }
        }
        out
    }

    #[test]
    fn allocation_free_case_foldings_equal_the_whole_string_reference() {
        let pairs = ci_pairs();
        let mut hits = 0usize;
        for (value, needle) in &pairs {
            let folded = needle.to_lowercase();
            let want = value.to_lowercase().contains(&folded);
            assert_eq!(
                contains_ignore_case(value, &folded),
                want,
                "contains({value:?}, {needle:?})"
            );
            hits += want as usize;
            let want_prefix = value.to_lowercase().starts_with(&folded);
            assert_eq!(
                starts_with_ignore_case(value, &folded),
                want_prefix,
                "starts_with({value:?}, {needle:?})"
            );
        }
        // Guard against a vacuous corpus: it must contain real matches.
        assert!(hits > pairs.len() / 4, "only {hits} of {} hit", pairs.len());
    }

    #[test]
    fn prepared_text_filters_agree_with_the_per_call_path() {
        for (value, needle) in ci_pairs() {
            let cell = ResultCell::text(&value);
            for op in [FilterOp::Contains, FilterOp::StartsWith] {
                let direct = cell_matches_filter(Some(&cell), op, &needle);
                let filter = ColumnFilter {
                    column: 0,
                    op,
                    value: needle.clone(),
                };
                assert_eq!(
                    PreparedFilter::prepare(&filter).matches(Some(&cell)),
                    direct,
                    "{op:?}({value:?}, {needle:?})"
                );
            }
        }
    }

    #[test]
    fn like_folds_non_ascii_values_exactly_like_the_whole_string_reference() {
        // The matcher folds the value lazily; the retired code folded the whole
        // string. Both must agree with the reference, which is written against
        // `value.to_lowercase()`. Sigma is the case that breaks a naive fold.
        let mut patterns: Vec<String> = Vec::new();
        for p in CI_VALUES {
            patterns.push(p.to_string());
            patterns.push(format!("%{p}"));
            patterns.push(format!("{p}%"));
            patterns.push(format!("%{p}_%"));
        }
        let mut hits = 0usize;
        for value in CI_VALUES {
            let vc: Vec<char> = value.to_lowercase().chars().collect();
            for pattern in &patterns {
                let pc: Vec<char> = pattern.to_lowercase().chars().collect();
                let want = like_reference(&vc, &pc);
                assert_eq!(
                    LikePattern::new(pattern).matches(value),
                    want,
                    "like({value:?}, {pattern:?})"
                );
                hits += want as usize;
            }
        }
        assert!(
            hits > 0 && hits < CI_VALUES.len() * patterns.len(),
            "hits={hits}"
        );
    }

    #[test]
    fn operators_for_group_by_cell_type() {
        let numeric = operators_for(CellType::Integer);
        assert!(numeric.contains(&FilterOp::GreaterThan));
        assert!(!numeric.contains(&FilterOp::Contains));
        let text = operators_for(CellType::Text);
        assert!(text.contains(&FilterOp::Contains));
        assert!(!text.contains(&FilterOp::GreaterThan));
    }
}

#[cfg(test)]
mod pagination_tests {
    use crate::state::operations::paginate_query;
    use dbstudio_core::models::DatabaseType;

    #[test]
    fn simple_dialects_use_limit_offset() {
        for db in [
            DatabaseType::SQLite,
            DatabaseType::MySQL,
            DatabaseType::PostgreSQL,
        ] {
            assert_eq!(
                paginate_query("SELECT * FROM t ORDER BY id", 100, 200, db),
                "SELECT * FROM ( SELECT * FROM t ORDER BY id ) AS _dbstudio_sub LIMIT 100 OFFSET 200"
            );
        }
    }

    #[test]
    fn trailing_semicolons_are_stripped() {
        assert_eq!(
            paginate_query("SELECT * FROM t;  ", 10, 0, DatabaseType::SQLite),
            "SELECT * FROM ( SELECT * FROM t ) AS _dbstudio_sub LIMIT 10 OFFSET 0"
        );
    }

    #[test]
    fn mssql_uses_offset_fetch() {
        assert_eq!(
            paginate_query("SELECT * FROM t", 100, 200, DatabaseType::MSSQL),
            "SELECT * FROM ( SELECT * FROM t ) AS _dbstudio_sub ORDER BY (SELECT NULL) OFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY"
        );
    }

    #[test]
    fn oracle_uses_offset_fetch() {
        assert_eq!(
            paginate_query("SELECT * FROM t", 100, 200, DatabaseType::Oracle),
            "SELECT * FROM ( SELECT * FROM t ) AS _dbstudio_sub OFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY"
        );
    }
}

#[cfg(test)]
mod guard_tests {
    use crate::state::guard::{classify_sql, requires_confirmation, WriteKind};
    use dbstudio_core::models::Environment;

    #[test]
    fn classifies_common_statements() {
        assert_eq!(classify_sql("  SELECT * FROM t"), Some(WriteKind::Select));
        assert_eq!(
            classify_sql("select id from users"),
            Some(WriteKind::Select)
        );
        assert_eq!(
            classify_sql("INSERT INTO t VALUES (1)"),
            Some(WriteKind::Insert)
        );
        assert_eq!(
            classify_sql("UPDATE t SET a = 1 WHERE id = 2"),
            Some(WriteKind::Update)
        );
        assert_eq!(
            classify_sql("UPDATE t SET a = 1"),
            Some(WriteKind::UpdateNoWhere)
        );
        assert_eq!(
            classify_sql("DELETE FROM t WHERE id = 2"),
            Some(WriteKind::Delete)
        );
        assert_eq!(
            classify_sql("DELETE FROM t"),
            Some(WriteKind::DeleteNoWhere)
        );
        assert_eq!(classify_sql("DROP TABLE t"), Some(WriteKind::Drop));
        assert_eq!(classify_sql("TRUNCATE TABLE t"), Some(WriteKind::Truncate));
        assert_eq!(
            classify_sql("ALTER TABLE t ADD COLUMN c INT"),
            Some(WriteKind::Alter)
        );
        assert_eq!(
            classify_sql("CREATE TABLE t (id INT)"),
            Some(WriteKind::Create)
        );
        assert_eq!(classify_sql("VACUUM"), Some(WriteKind::OtherDdl));
    }

    #[test]
    fn classify_ignores_unknown_and_empty() {
        assert_eq!(classify_sql(""), None);
        assert_eq!(classify_sql("   "), None);
        assert_eq!(classify_sql("NOT SQL AT ALL"), None);
    }

    #[test]
    fn safe_mode_confirms_every_write() {
        assert!(requires_confirmation(
            Some(WriteKind::Insert),
            Environment::Dev,
            true
        ));
        assert!(requires_confirmation(
            Some(WriteKind::Update),
            Environment::Dev,
            true
        ));
        assert!(requires_confirmation(
            Some(WriteKind::UpdateNoWhere),
            Environment::Staging,
            true
        ));
        assert!(requires_confirmation(
            Some(WriteKind::Delete),
            Environment::Dev,
            true
        ));
        assert!(requires_confirmation(
            Some(WriteKind::DeleteNoWhere),
            Environment::Dev,
            true
        ));
        assert!(requires_confirmation(
            Some(WriteKind::Drop),
            Environment::Dev,
            true
        ));
        assert!(requires_confirmation(
            Some(WriteKind::Alter),
            Environment::Dev,
            true
        ));
        assert!(requires_confirmation(
            Some(WriteKind::Create),
            Environment::Dev,
            true
        ));
        assert!(requires_confirmation(
            Some(WriteKind::OtherDdl),
            Environment::Dev,
            true
        ));
        assert!(!requires_confirmation(
            Some(WriteKind::Select),
            Environment::Dev,
            true
        ));
        assert!(!requires_confirmation(
            Some(WriteKind::Select),
            Environment::Production,
            true
        ));
    }

    #[test]
    fn without_safe_mode_environments_apply() {
        assert!(!requires_confirmation(
            Some(WriteKind::Update),
            Environment::Dev,
            false
        ));
        assert!(!requires_confirmation(
            Some(WriteKind::Delete),
            Environment::Staging,
            false
        ));
        assert!(requires_confirmation(
            Some(WriteKind::DeleteNoWhere),
            Environment::Dev,
            false
        ));
        assert!(requires_confirmation(
            Some(WriteKind::Insert),
            Environment::Production,
            false
        ));
        assert!(!requires_confirmation(
            Some(WriteKind::Select),
            Environment::Production,
            false
        ));
    }

    // ── 绕过回归：注释前缀、DML-CTE、注释伪造 WHERE、多语句批次 ──

    #[test]
    fn comment_prefix_cannot_hide_a_write() {
        // Safe Mode 曾被 `--` 前缀完全绕过：首 token 是 `--`，落入未知分支返回 None。
        assert_eq!(
            classify_sql(
                "-- hello
DROP TABLE users;"
            ),
            Some(WriteKind::Drop)
        );
        assert_eq!(
            classify_sql("/* header */ DELETE FROM users"),
            Some(WriteKind::DeleteNoWhere)
        );
        // 纯注释输入没有可执行语句。
        assert_eq!(classify_sql("-- just a comment"), None);
    }

    #[test]
    fn dml_cte_is_not_read_only() {
        // `WITH` 曾一律归为 Select：数据修改 CTE 在 safe mode 下不弹确认直接执行。
        assert_eq!(
            classify_sql("WITH d AS (DELETE FROM users RETURNING *) SELECT * FROM d"),
            Some(WriteKind::DeleteNoWhere)
        );
        assert_eq!(
            classify_sql("WITH u AS (UPDATE users SET x = 1 RETURNING x) SELECT * FROM u"),
            Some(WriteKind::UpdateNoWhere)
        );
        // 只读 CTE 仍是 Select。
        assert_eq!(
            classify_sql("WITH c AS (SELECT 1 AS x) SELECT * FROM c"),
            Some(WriteKind::Select)
        );
    }

    #[test]
    fn comment_cannot_fake_a_where_clause() {
        // 曾用 `upper.find(" WHERE ")` 判定：注释里的 WHERE 会让无界 DELETE
        // 冒充有界写，在 safe mode 关闭的 dev 环境跳过确认。
        assert_eq!(
            classify_sql("DELETE FROM t /* WHERE */"),
            Some(WriteKind::DeleteNoWhere)
        );
        assert_eq!(
            classify_sql("DELETE FROM t WHERE id = 1"),
            Some(WriteKind::Delete)
        );
        // WHERE 前是换行也能识别（旧实现只匹配 " WHERE "）。
        assert_eq!(
            classify_sql(
                "DELETE FROM t
WHERE id = 1"
            ),
            Some(WriteKind::Delete)
        );
    }

    #[test]
    fn batch_and_hidden_writes_are_caught() {
        assert_eq!(
            classify_sql("SELECT 1; DROP TABLE x"),
            Some(WriteKind::Drop)
        );
        // 字符串与标识符里的写关键字不误判。
        assert_eq!(
            classify_sql("SELECT 'DROP TABLE x' AS note FROM t"),
            Some(WriteKind::Select)
        );
        assert_eq!(
            classify_sql("SELECT updated_at, deleted FROM t"),
            Some(WriteKind::Select)
        );
        // SELECT ... FOR UPDATE 是锁定读，不是写。
        assert_eq!(
            classify_sql("SELECT * FROM t WHERE id = 1 FOR UPDATE"),
            Some(WriteKind::Select)
        );
        // 括号包裹的写语句不再落到未知分支。
        assert_eq!(
            classify_sql("(DELETE FROM t)"),
            Some(WriteKind::DeleteNoWhere)
        );
    }

    #[test]
    fn where_attribution_follows_the_syntax_tree() {
        // 子查询里的 WHERE 只属于子查询：主语句仍是无界写（保守归类）。
        // 旧启发式只看全文有没有 "WHERE"，会把无界 UPDATE 冒充有界写，
        // 在 safe mode 关闭的 Dev 环境跳过确认。
        assert_eq!(
            classify_sql("UPDATE t SET x = (SELECT MAX(id) FROM u WHERE y = 1)"),
            Some(WriteKind::UpdateNoWhere)
        );
        // 反过来，DELETE 的 WHERE（含子查询）是真有界。
        assert_eq!(
            classify_sql("DELETE FROM t WHERE id IN (SELECT id FROM u)"),
            Some(WriteKind::Delete)
        );
        // EXPLAIN 前缀后的写语句仍是写，作用域照常判定。
        assert_eq!(
            classify_sql("EXPLAIN DELETE FROM t"),
            Some(WriteKind::DeleteNoWhere)
        );
        assert_eq!(
            classify_sql("EXPLAIN SELECT * FROM t"),
            Some(WriteKind::Select)
        );
        // CTE 内的写语句作用归属不明确，一律按无界（破坏性）处理。
        assert_eq!(
            classify_sql("WITH d AS (DELETE FROM users WHERE id = 1 RETURNING *) SELECT * FROM d"),
            Some(WriteKind::DeleteNoWhere)
        );
    }
}

#[cfg(test)]
mod vim_tests {
    use crate::components::vim::{VimBuf, VimMode};

    fn buf(text: &str) -> VimBuf {
        let mut v = VimBuf::default();
        v.text = text.to_string();
        v.caret = 0;
        v
    }

    #[test]
    fn motions_are_byte_aware_and_utf8_safe() {
        let mut v = buf("héllo wörld");
        v.step("w", false, false, false);
        assert_eq!(v.caret, 7); // skips the multi-byte 'é'
        v.step("b", false, false, false);
        assert_eq!(v.caret, 0);
        v.step("$", false, false, false);
        assert_eq!(v.caret, 13);
    }

    #[test]
    fn numeric_count_composition() {
        let mut v = buf("abcdef");
        v.step("3", false, false, false);
        v.step("l", false, false, false);
        assert_eq!(v.caret, 3);
    }

    #[test]
    fn insert_roundtrip_after_a() {
        let mut v = buf("ab\ncd");
        v.caret = 1;
        let step = v.step("a", false, false, false).unwrap();
        assert!(step.to_insert);
        assert!(matches!(v.mode, VimMode::Insert));
        let back = v.step("escape", false, false, false).unwrap();
        assert!(!back.to_insert);
        assert!(matches!(v.mode, VimMode::Normal));
    }

    #[test]
    fn open_line_above_and_below_land_on_blank_line() {
        let mut o = buf("a\nb\nc");
        o.caret = 2; // on 'b'
        let step = o.step("o", false, false, false).unwrap();
        assert!(matches!(o.mode, VimMode::Insert));
        assert_eq!(step.caret, 4);
        assert_eq!(o.text, "a\nb\n\nc");

        let mut big = buf("a\nb\nc");
        big.caret = 2;
        let step = big.step("O", false, false, false).unwrap();
        assert_eq!(big.text, "a\n\nb\nc");
        assert_eq!(step.caret, 2);

        let mut tail = buf("a");
        tail.step("o", false, false, false).unwrap();
        assert_eq!(tail.text, "a\n");
        assert_eq!(tail.caret, 2);
    }

    #[test]
    fn x_deletes_char_with_undo_and_redo() {
        let mut v = buf("abc");
        v.caret = 1;
        let step = v.step("x", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("ac"));
        assert_eq!(step.caret, 1);
        v.step("u", false, false, false);
        assert_eq!(v.text, "abc");
        v.step("r", true, false, false); // ctrl-r redo
        assert_eq!(v.text, "ac");
    }

    #[test]
    fn dd_deletes_current_line() {
        let mut v = buf("a\nb\nc");
        v.caret = 2;
        v.step("d", false, false, false);
        assert!(v.operator.is_some());
        let step = v.step("d", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("a\nc"));
    }

    #[test]
    fn visual_char_delete_removes_selected_range() {
        let mut v = buf("abcd ef");
        v.caret = 2;
        v.step("v", false, false, false);
        assert!(matches!(v.mode, VimMode::VisualChar));
        v.step("l", false, false, false); // extend to 'cd'
        let step = v.step("d", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("ab ef"));
    }

    #[test]
    fn visual_block_deletes_characters_per_line() {
        let mut v = buf("abc\ndef\nghi");
        v.caret = 1; // 'b'
        v.step("v", true, false, false);
        assert!(matches!(v.mode, VimMode::VisualBlock));
        v.step("j", false, false, false); // anchor the second line
        let step = v.step("d", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("ac\ndf\nghi"));
    }

    #[test]
    fn yank_motion_then_paste_repeats_word() {
        let mut v = buf("one two");
        v.caret = 0;
        v.step("y", false, false, false);
        v.step("w", false, false, false);
        assert_eq!(v.register, "one "); // range runs to the next word start
        v.step("p", false, false, false);
        assert_eq!(v.text, "oone ne two");
    }

    #[test]
    fn replace_pending_r_swaps_one_char() {
        let mut v = buf("abc");
        v.caret = 1;
        v.step("r", false, false, false);
        let step = v.step("z", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("azc"));
        assert_eq!(v.register, "b");
    }

    #[test]
    fn join_lines_replaces_newline_with_space() {
        let mut v = buf("select *\nfrom t");
        v.caret = 0;
        let step = v.step("J", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("select * from t"));
    }
}

#[cfg(test)]
mod help_tests {
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
}

#[cfg(test)]
mod fk_options_tests {
    use crate::state::build_fk_options_sql;
    use dbstudio_core::models::DatabaseType;

    #[test]
    fn fk_options_sql_selects_distinct_references() {
        let sql = build_fk_options_sql("customers", "id", 100, 0, DatabaseType::PostgreSQL);
        assert!(
            sql.contains("SELECT DISTINCT \"id\" FROM \"customers\""),
            "{sql}"
        );
        assert!(sql.contains("IS NOT NULL"), "{sql}");
        assert!(sql.contains("ORDER BY \"id\""), "{sql}");
        assert!(sql.contains("LIMIT 100"), "{sql}");
    }

    #[test]
    fn fk_options_sql_quotes_per_dialect() {
        let sql = build_fk_options_sql("customers", "id", 50, 0, DatabaseType::MySQL);
        assert!(sql.contains("`id`"), "{sql}");
        assert!(sql.contains("`customers`"), "{sql}");
        assert!(sql.contains("LIMIT 50"), "{sql}");

        let sql = build_fk_options_sql("customers", "id", 50, 0, DatabaseType::MSSQL);
        assert!(sql.contains("[id]"), "{sql}");
    }
}
