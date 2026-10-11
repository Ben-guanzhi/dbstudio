#![recursion_limit = "256"]

pub mod components;
pub mod fold;
pub mod state;
pub mod url_scheme;
pub mod url_scheme_registry;
pub mod utils;

#[cfg(test)]
mod filter_tests {
    use crate::components::results_panel::{
        cell_matches_filter, like_match, operators_for, ColumnFilter, FilterOp, LikePattern,
        PreparedFilter,
    };
    use crate::fold::{
        ascii_contains, byte_compares, contains_ignore_case, has_lowercase_mapping, is_titlecase,
        only_ascii_folds, reset_byte_compares, starts_with_ignore_case, CASE_LEAD_BYTE,
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

    /// The matcher switches strategy at `SCAN_MAX_BYTES`: a byte window scan
    /// below it, a fold-then-search copy above. Both sides must answer exactly
    /// like the whole-string reference, and a hit has to survive on each side of
    /// the boundary - including one that ends at the very last byte of a long
    /// haystack, which is what a truncated scan would miss.
    ///
    /// The bases are the shapes the boundary is argued from, not one of them:
    /// free ASCII text (the shape that loses), a `·`-punctuated label (the shape
    /// that sits on the crossover) and a mixed CJK/ASCII column value (the shape
    /// that does not cross at all), each repeated so its byte length walks
    /// across 64 from both sides.
    #[test]
    fn case_insensitive_contains_agrees_on_both_sides_of_the_threshold() {
        let mut hits = 0usize;
        let mut checked = 0usize;
        let mut lengths = vec![];
        for base in [
            "abc",
            "Order_Item_7",
            "中文表格",
            "ΣΣΣ",
            "İstanbul",
            "🙂x",
            "Table · heap",
            "showing 12 of 34+ rows · 5 ms",
            "postgres@db.internal · reporting",
            "订单 · 表格 · reporting",
            "中文表格 · 列值",
            "order detail rows filtered",
        ] {
            // Repeating walks every base across the threshold.
            for repeat in 1..=30usize {
                let haystack = base.repeat(repeat);
                lengths.push(haystack.len());
                for needle in [
                    base.to_lowercase(),
                    base.to_uppercase(),
                    "absent-needle".to_string(),
                ] {
                    let folded = needle.to_lowercase();
                    let want = haystack.to_lowercase().contains(&folded);
                    assert_eq!(
                        contains_ignore_case(&haystack, &folded),
                        want,
                        "contains(len={}) with {needle:?}",
                        haystack.len()
                    );
                    hits += want as usize;
                    checked += 1;
                    let want_prefix = haystack.to_lowercase().starts_with(&folded);
                    assert_eq!(
                        starts_with_ignore_case(&haystack, &folded),
                        want_prefix,
                        "starts_with(len={}) with {needle:?}",
                        haystack.len()
                    );
                }
            }
        }
        assert!(
            lengths.iter().any(|l| *l < crate::fold::SCAN_MAX_BYTES)
                && lengths.iter().any(|l| *l > crate::fold::SCAN_MAX_BYTES),
            "corpus never straddled the threshold: {lengths:?}"
        );
        assert!(
            hits > 0 && hits < checked,
            "corpus must contain both hits and misses (hits={hits}, checked={checked})"
        );
    }

    /// The anchored matcher is gated by `only_ascii_folds` alone, while
    /// `contains_ignore_case` keeps the `SCAN_MAX_BYTES` term: the shapes that
    /// cross that bound were measured to cross it in opposite directions, so
    /// one length term cannot price both loops. These cells are the ones only
    /// that split can reach - the same three shapes the boundary is argued
    /// from, walked from 63 B out to 1 KiB, each opening and closing on `ABC`
    /// so a needle can only be answered through ASCII folding.
    #[test]
    fn starts_with_takes_the_scan_beyond_the_contains_bound() {
        // `len` exact bytes: `ABC`, `base` as often as fits, `x` to fill, then
        // `ABC` again so the last byte of the cell is reached too.
        fn haystack(len: usize, base: &str) -> String {
            let mut s = String::from("ABC");
            while s.len() + base.len() + 4 <= len {
                s.push_str(base);
            }
            while s.len() + 4 <= len {
                s.push('x');
            }
            s.push_str("ABC");
            assert_eq!(s.len(), len, "builder missed {len} bytes");
            s
        }

        let mut checked = 0usize;
        let mut fold_only_hits = 0usize;
        let mut misses = 0usize;
        let mut scanned_past_the_bound = 0usize;
        for base in [
            "order detail rows filtered",
            "订单 · 表格 · reporting",
            "中文表格 · 列值",
        ] {
            for len in [63usize, 64, 65, 96, 256, 1024] {
                let haystack = haystack(len, base);
                // The gates as they are written, copied here so a cell that
                // never takes the scan cannot report itself as one that does.
                let admits = only_ascii_folds(&haystack);
                assert!(admits, "{haystack:?} is not admitted at all");
                let contains_scans = haystack.len() <= crate::fold::SCAN_MAX_BYTES && admits;
                if admits && !contains_scans {
                    scanned_past_the_bound += 1;
                }
                for needle in ["abc", "abcd"] {
                    let want = haystack.to_lowercase().starts_with(needle);
                    assert_eq!(
                        starts_with_ignore_case(&haystack, needle),
                        want,
                        "starts_with(len={len}, {base:?}, {needle:?})"
                    );
                    checked += 1;
                    if want && !haystack.starts_with(needle) {
                        // Answered only because the scan folds ASCII case.
                        fold_only_hits += 1;
                    }
                    misses += !want as usize;
                }
            }
        }
        // Teeth, in counts rather than in `any`: 12 of the 18 cells sit past
        // `SCAN_MAX_BYTES` and are admitted, and the needles make 24 of the
        // 36 comparisons. Put the length term back on the anchored gate
        // and `scanned_past_the_bound` drops to 0; make the scan compare bytes
        // verbatim and the hit needle stops answering.
        assert_eq!(checked, 36, "needle x shape x length grid");
        assert_eq!(
            scanned_past_the_bound, 12,
            "the two gates must differ on exactly the cells past the bound"
        );
        assert_eq!(
            fold_only_hits, 18,
            "every `abc` cell has to hit through folding, not verbatim"
        );
        assert_eq!(misses, 18, "every `abcd` cell has to miss");
    }

    #[test]
    fn prepared_text_filters_agree_with_the_per_call_path() {
        // Four operators, two prepared routes: the two text operators reach
        // `Folded`, `>` / `<` reach `Folded` too whenever the operand is not a
        // number and `Number` when it is (`"12345"` in this corpus is).
        for (value, needle) in ci_pairs() {
            let cell = ResultCell::text(&value);
            for op in [
                FilterOp::Contains,
                FilterOp::StartsWith,
                FilterOp::GreaterThan,
                FilterOp::LessThan,
            ] {
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

    /// The truth table of hoisting `>` / `<` onto the prepared route, cell by
    /// cell against the per-call path it replaced. The operand set is every
    /// shape the parse accepts and every shape it rejects - measured, not
    /// assumed: `"1e400"` overflows `f64` and *is* accepted, as `+inf`, while
    /// `"2026-01-01"`, the everyday date-column operand, is not - and the cell
    /// set covers NULL, the absent cell, cells with and without a sort proxy,
    /// and the fold traps (`ß`, `ΑΣ`, `İ12`, `STRASSE`).
    #[test]
    fn prepared_numeric_filters_agree_with_the_per_call_path() {
        let operands = [
            "12",
            " 12 ",
            "1e3",
            "inf",
            "-0",
            "1e400",
            "NaN",
            "",
            "5x",
            "abc",
            "2026-01-01",
            "2026-01-01 12:00:00",
            "МСК",
            "10y",
            "%x",
        ];
        let cells = [
            ResultCell::text("x10y"),
            ResultCell::text(""),
            ResultCell::text("12"),
            ResultCell::text(" 12 "),
            ResultCell::text("STRASSE"),
            ResultCell::text("ß"),
            ResultCell::text("ΑΣ"),
            ResultCell::text("İ12"),
            ResultCell::integer(10),
            ResultCell::integer(-3),
            ResultCell::float(1.5),
            ResultCell::float(f64::NAN),
            ResultCell::null(),
        ];
        for op in [FilterOp::GreaterThan, FilterOp::LessThan] {
            for operand in operands {
                let filter = ColumnFilter {
                    column: 0,
                    op,
                    value: operand.to_string(),
                };
                let prepared = PreparedFilter::prepare(&filter);
                for cell in &cells {
                    assert_eq!(
                        prepared.matches(Some(cell)),
                        cell_matches_filter(Some(cell), op, operand),
                        "{}({}, {:?})",
                        op.label(),
                        cell.value,
                        operand
                    );
                }
                assert!(!prepared.matches(None), "{op:?} against no cell");
            }
        }

        // The one divergence a careless hoist produces, pinned on its own
        // rather than left to the grid above: NULL cells answer value-bearing
        // operators with `false`, but the fallback fold asks
        // `contains_ignore_case(value, "")`, and an empty needle is `true`.
        // Without the `!cell.is_null` guard `NULL > ""` flips to `true`.
        for op in [FilterOp::GreaterThan, FilterOp::LessThan] {
            let empty = PreparedFilter::prepare(&ColumnFilter {
                column: 0,
                op,
                value: String::new(),
            });
            assert!(!empty.matches(None), "{op:?} against no cell");
            assert!(
                !empty.matches(Some(&ResultCell::null())),
                "{op:?} against NULL"
            );
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

    /// The palette/tree labels separate their parts with `·` (U+00B7). It is a
    /// symbol, so case folding leaves it alone - which is what makes the search
    /// box behave: the dot must neither break a match nor have to be typed.
    /// These literals used to hold a mojibaked `路` (a UTF-8 `·` read once as
    /// GBK), which broke both the display and this assumption.
    #[test]
    fn middle_dot_separator_stays_transparent_to_case_folding() {
        assert_eq!("Table · heap".to_lowercase(), "table · heap");
        for haystack in [
            "Table · heap",
            "postgres@db.internal · reporting",
            "sales · #uk #eu",
        ] {
            for needle in ["table", "TABLE", "· heap", "reporting", "#UK"] {
                let folded = needle.to_lowercase();
                assert_eq!(
                    contains_ignore_case(haystack, &folded),
                    haystack.to_lowercase().contains(&folded),
                    "{haystack:?} vs {needle:?}"
                );
            }
            assert!(contains_ignore_case(haystack, "· "), "{haystack:?}");
            assert!(!contains_ignore_case(haystack, "路"), "{haystack:?}");
        }
    }

    /// The scan gate admits a haystack on one property (`only_ascii_folds`) and
    /// used to demand an ASCII needle on top. Dropping that demand rests on how
    /// UTF-8 and ASCII folding are built: folding changes one byte and leaves the
    /// result below 0x80, so it can never create or destroy the byte sequence a
    /// multi-byte needle looks for, and a valid-UTF-8 needle that matches bytes
    /// must match whole characters. Exhaustive over the product here, not
    /// sampled, with `to_lowercase()` as the oracle.
    ///
    /// The two C1 controls in the haystack alphabet are not representative of UI
    /// labels - they are there because their second bytes (0x90, 0x97) differ
    /// from `·`'s (0xB7) by exactly the ASCII case bit. A scanner that folded
    /// with `byte | 0x20` instead of `eq_ignore_ascii_case` would call
    /// `contains("\u{97}", "·")` a hit; this corpus is built so that mistake
    /// cannot pass, which is what makes the rest of the claim testable.
    ///
    /// §16 added `中` and `ß` to the haystack alphabet. They are not more
    /// "representative" material: they are the two shapes the gate started to
    /// admit in that round (a 3-byte `Lo` ideograph the old `is_alphabetic` proxy
    /// refused, and a 2-byte lowercase letter with no mapping), so the pairing
    /// claim above is now tested against scan routes that only exist because of
    /// the widening. It moved the teeth count from 820 x 98 = 80,360 to
    /// 1,464 x 98 = 143,472 - every haystack in the corpus takes the scan now,
    /// where before only the symbol-bearing ones did.
    #[test]
    fn non_ascii_needles_are_safe_for_the_byte_scan() {
        const HAYS: &[char] = &[
            'a', 'B', '_', '·', '→', '\u{301}', '1', '\u{90}', '\u{97}', '中', 'ß',
        ];
        const NEEDLES: &[char] = &['a', 'B', '·', '\u{301}', 'İ', 'Σ', 'ς', '中', '→', '_'];

        fn strings(alphabet: &[char], max_len: usize) -> Vec<String> {
            let mut out = vec![String::new()];
            let mut level = vec![String::new()];
            for _ in 0..max_len {
                let mut next = Vec::with_capacity(level.len() * alphabet.len());
                for prefix in &level {
                    for c in alphabet {
                        let mut s = prefix.clone();
                        s.push(*c);
                        next.push(s);
                    }
                }
                out.extend(next.iter().cloned());
                level = next;
            }
            out
        }

        let hay = strings(HAYS, 3);
        let needles = strings(NEEDLES, 2);
        let mut scanned_with_non_ascii_needle = 0usize;
        let mut hits = 0usize;
        let mut checked = 0usize;
        for haystack in &hay {
            // The gate the change widened: if this pair does not take the scan,
            // it proves nothing about it.
            let scans = haystack.len() <= crate::fold::SCAN_MAX_BYTES
                && crate::fold::only_ascii_folds(haystack);
            for needle in &needles {
                let folded = needle.to_lowercase();
                let want = haystack.to_lowercase().contains(&folded);
                assert_eq!(
                    contains_ignore_case(haystack, &folded),
                    want,
                    "contains({haystack:?}, {needle:?})"
                );
                let want_prefix = haystack.to_lowercase().starts_with(&folded);
                assert_eq!(
                    starts_with_ignore_case(haystack, &folded),
                    want_prefix,
                    "starts_with({haystack:?}, {needle:?})"
                );
                checked += 1;
                hits += want as usize + want_prefix as usize;
                if scans && !folded.is_empty() && !folded.is_ascii() {
                    scanned_with_non_ascii_needle += 1;
                }
            }
        }
        // Teeth: without these the test would still pass if someone put the
        // needle gate back, or if the corpus drifted into all-hits/all-misses.
        // 1,464 haystacks x 98 needles whose fold is non-ASCII = 143,472 pairs
        // that must reach the scan (every haystack in the alphabet now passes the
        // gate; before §16 only the 820 symbol-bearing ones did, for 80,360).
        assert!(
            scanned_with_non_ascii_needle > 120_000,
            "the widened gate was barely exercised: {scanned_with_non_ascii_needle}"
        );
        assert!(
            hits > 0 && hits < 2 * checked,
            "corpus must contain both hits and misses (hits={hits}, checked={checked})"
        );
    }

    /// `fold::only_ascii_folds` lets a non-ASCII haystack into the byte scan on
    /// the strength of one claim: every code point with a lowercase mapping is
    /// alphabetic. Brute-forced here instead of trusted, because a stray symbol
    /// mapping would make the scan answer a filter box wrongly rather than crash.
    #[test]
    fn only_alphabetic_code_points_have_a_lowercase_mapping() {
        let mut mapped = 0usize;
        for cp in 0x80u32..0x110000 {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            let mut lower = c.to_lowercase();
            if lower.next() == Some(c) && lower.next().is_none() {
                continue;
            }
            mapped += 1;
            assert!(
                c.is_alphabetic(),
                "U+{cp:04X} has a lowercase mapping but is not alphabetic, so \
                 `only_ascii_folds` would scan it byte-wise and fold it wrongly"
            );
        }
        assert!(mapped > 1000, "degenerate run: {mapped} mapped code points");
    }

    /// The edge of the admitted set, spelled out. §15's version of this list put
    /// `中文表格`, `Table · 报表`, `straße`, `µs` on the *rejecting* side, because
    /// its proxy asked "is this character a letter". The gate now asks "does this
    /// character have a lowercase mapping", those four do not, and they moved to
    /// the admitting half - that move is the whole point of the round: it is what
    /// takes a 10,000-row x 20-column CJK grid from 200,000 `to_lowercase` copies
    /// per pass to 0. `Ⅻ` and `Ａ` stay rejected because Roman numerals and
    /// fullwidth letters *are* `Lu`, which the old proxy also caught, for
    /// different reasons.
    ///
    /// The last three lines are this gate's own over-rejections: `𝐀𝐁`, `ℂℕℝ`, `ϒ`
    /// fold to themselves, so admitting them would have been safe, and the gate
    /// refuses them because `is_uppercase` cannot tell them apart from `Ａ`. They
    /// are spelled out so the price of a *sufficient* condition stays visible:
    /// 549 such characters, all uppercase, none of them a correctness problem.
    #[test]
    fn the_scan_admits_symbols_and_rejects_case_mapped_scripts() {
        for s in [
            "",
            "abc",
            "Table · heap",
            "showing 12 of 34+ rows · 5 ms",
            "a → b",
            "🙂 #1 ①",
            "1234 · 5678",
            "≤ ≥ ± × ÷",
            // newly admitted, §16:
            "中文表格",
            "Table · 报表",
            "straße",
            "µs",
            "订单表_2024",
            "归档记录 · 订单明细表",
            "αβγδεζηθικλμ",
            "한",
            "café · crème",
            "你好，世界！ 주문",
        ] {
            assert!(only_ascii_folds(s), "{s:?} should scan");
        }
        for s in [
            "ΣΣΣ",
            "İstanbul",
            "ǅ",
            "Ⅻ",
            "Ａ",
            "МОСКВА",
            "ΝΤΟΚΙ",
            "Пользователь",
            "ΑΒΓαβγ",
            "订单À_2024",
            // admitted by the property, refused by the gate: harmless.
            "𝐀𝐁",
            "ℂℕℝ",
            "ϒ",
        ] {
            assert!(!only_ascii_folds(s), "{s:?} must keep the copying path");
        }
    }

    /// The contract of `only_ascii_folds`, brute-forced over every code point: a
    /// character may enter the byte scan only if `to_lowercase` leaves it exactly
    /// as it is. Everything §16 measured rests on that one implication, so it is
    /// checked instead of argued, and the two hand-written tables the fast path is
    /// built from are checked against the property they stand for rather than
    /// trusted as literals:
    ///   * `CASE_LEAD_BYTE` must be exactly the set of first bytes of cased
    ///     characters - one missing entry would wave a cased string through the
    ///     prefilter without decoding it;
    ///   * `is_titlecase` must be exactly the 31 `Lt` code points, every one of
    ///     them cased and none of them `is_uppercase` - that residue is why the
    ///     category test alone is not enough.
    ///
    /// Note this is NOT §15's `only_alphabetic_code_points_have_a_lowercase_mapping`
    /// (still true, still checked there): "cased => alphabetic" says nothing about
    /// whether the gate catches the cased ones, which is the direction that keeps
    /// the scan correct.
    #[test]
    fn the_gate_admits_only_characters_that_fold_to_themselves() {
        let mut mapped = 0usize;
        let mut admitted_mapped = 0usize;
        let mut over_rejected = 0usize;
        let mut title = 0usize;
        let mut newly_admitted = 0usize;
        let mut newly_refused = 0usize;
        let mut leads: Vec<u8> = Vec::new();
        for cp in 0x80u32..0x110000 {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            let mut buf = [0u8; 4];
            let enc = c.encode_utf8(&mut buf);
            let mut lower = c.to_lowercase();
            let identity = lower.next() == Some(c) && lower.next().is_none();
            let admits = only_ascii_folds(enc);
            if identity {
                if !admits {
                    over_rejected += 1;
                    assert!(
                        has_lowercase_mapping(c) && c.is_uppercase(),
                        "U+{cp:04X} {c:?} folds to itself, the gate refuses it, and the \
                         gate only refuses characters it calls uppercase or titlecase - so \
                         the titlecase list contains an identity character"
                    );
                }
            } else {
                mapped += 1;
                admitted_mapped += admits as usize;
                assert!(
                    enc.bytes().any(|b| CASE_LEAD_BYTE[b as usize]),
                    "U+{cp:04X} {c:?} is cased but none of its bytes is a flagged lead \
                     byte, so `only_ascii_folds` would wave it through undecoded"
                );
                assert!(
                    c.is_uppercase() || is_titlecase(c),
                    "U+{cp:04X} {c:?} is cased but neither uppercase nor titlecase, so \
                     the gate would admit it into the byte scan"
                );
                leads.push(*enc.as_bytes().first().unwrap());
            }
            if is_titlecase(c) {
                title += 1;
                assert!(
                    !identity,
                    "the titlecase list carries an identity character U+{cp:04X}"
                );
            }
            // The proxy §15 shipped, per character, so the test can say which
            // direction the set moved in.
            let old_admits = c.is_ascii() || !c.is_alphabetic();
            newly_refused += (old_admits && !admits) as usize;
            newly_admitted += (identity && !old_admits && admits) as usize;
        }
        assert_eq!(
            admitted_mapped, 0,
            "`only_ascii_folds` admitted a character that has a lowercase mapping"
        );
        leads.sort_unstable();
        leads.dedup();
        assert_eq!(
            leads.len(),
            21,
            "the number of UTF-8 lead bytes that can start a cased character moved"
        );
        for b in 0..=255u8 {
            assert_eq!(
                CASE_LEAD_BYTE[b as usize],
                leads.contains(&b),
                "the lead-byte table and the cased characters disagree on {b:#04X}"
            );
        }
        assert_eq!(
            title, 31,
            "`is_titlecase` no longer lists exactly the 31 titlecase code points"
        );
        assert!(
            (1_400..1_500).contains(&mapped),
            "the cased surface moved: {mapped} code points"
        );
        assert!(
            (500..600).contains(&over_rejected),
            "the gate's over-rejection moved: {over_rejected} identity characters refused"
        );
        assert_eq!(
            newly_refused, 0,
            "the widened gate refuses something the old proxy admitted"
        );
        assert!(
            newly_admitted > 140_000,
            "the gain surface this round bought is gone: {newly_admitted} characters"
        );
        for c in ['\u{1d400}', '\u{2102}', '\u{3d2}'] {
            let mut lower = c.to_lowercase();
            assert_eq!(
                (lower.next(), lower.next()),
                (Some(c), None),
                "U+{:04X} is the documented over-rejection example and must fold to itself",
                c as u32
            );
            let mut buf = [0u8; 4];
            assert!(
                !only_ascii_folds(c.encode_utf8(&mut buf)),
                "U+{:04X} must still be refused by the gate: {c:?}",
                c as u32
            );
        }
    }

    /// The pairing claim of `non_ascii_needles_are_safe_for_the_byte_scan`,
    /// re-run over the material §16 let in: every string up to length 3 over 12
    /// characters that mix newly admitted scripts (`中`, `한`, `α`, `σ`, `ς`, `ß`,
    /// `µ`) with separators and the cased survivors (`Σ`, `İ`), crossed with the
    /// needle alphabet, each pair asserted as "the scan answers exactly what the
    /// copying path answers".
    ///
    /// §17 widened the needle alphabet from 8 to 11 characters by adding `한`,
    /// `글` and `β` - the last two because `글`/`한` and `β`/`α` are the pairs whose
    /// *final* byte differs while the earlier ones are shared, which is exactly
    /// the shape the scan now rejects one byte at a time. With 8 needles the
    /// corpus was 1,885 x 73 = 137,605 pairs and a needle of at most 6 bytes
    /// almost always mismatched in its first one or two, so the last-byte
    /// rejection path never ran; the widened product is 1,885 x 133 = **250,705
    /// pairs**. Neither half mismatches: the count is 0 before the widening and
    /// 0 after it, and that is an asserted quantity rather than a measured one -
    /// a wrong answer panics here.
    ///
    /// This is the bounded shape of a longer sweep: the scratch version crossed
    /// every 4-character string over a 36-character alphabet with 9 needles,
    /// 10,673,289 pairs, 331,776 admitted strings, 2,985,984 answers taken from
    /// the scan, 0 mismatches. That does not finish inside a unit test, so what
    /// lives here is the smaller product; §16.7 records what the bound leaves out.
    #[test]
    fn newly_admitted_scripts_answer_correctly_in_pairs() {
        const HAYS: &[char] = &['中', '한', 'α', 'σ', 'ς', 'ß', 'µ', '·', 'a', 'B', 'Σ', 'İ'];
        const NEEDLES: &[char] = &['中', 'α', 'σ', 'ß', 'a', '·', 'Σ', '_', '한', '글', 'β'];

        fn strings(alphabet: &[char], max_len: usize) -> Vec<String> {
            let mut out = vec![String::new()];
            let mut level = vec![String::new()];
            for _ in 0..max_len {
                let mut next = Vec::with_capacity(level.len() * alphabet.len());
                for prefix in &level {
                    for c in alphabet {
                        let mut s = prefix.clone();
                        s.push(*c);
                        next.push(s);
                    }
                }
                out.extend(next.iter().cloned());
                level = next;
            }
            out
        }

        let old_proxy =
            |s: &str| s.is_ascii() || s.chars().all(|c| c.is_ascii() || !c.is_alphabetic());

        let mut checked = 0usize;
        let mut hits = 0usize;
        let mut newly_scanned = 0usize;
        for haystack in &strings(HAYS, 3) {
            let scans = haystack.len() <= crate::fold::SCAN_MAX_BYTES && only_ascii_folds(haystack);
            for needle in &strings(NEEDLES, 2) {
                let folded = needle.to_lowercase();
                let want = haystack.to_lowercase().contains(&folded);
                assert_eq!(
                    contains_ignore_case(haystack, &folded),
                    want,
                    "contains({haystack:?}, {needle:?})"
                );
                let want_prefix = haystack.to_lowercase().starts_with(&folded);
                assert_eq!(
                    starts_with_ignore_case(haystack, &folded),
                    want_prefix,
                    "starts_with({haystack:?}, {needle:?})"
                );
                checked += 1;
                hits += want as usize + want_prefix as usize;
                if scans && !old_proxy(haystack) {
                    newly_scanned += 1;
                }
            }
        }
        // Teeth, same shape as the test above: 1,071 of the 1,885 haystacks are
        // admitted now and were refused before, times 133 needles. The third
        // line is §13's teeth: without the widened needle alphabet `checked`
        // stalls at 137,605, which is what the corpus measured before the
        // last-byte rejection existed.
        assert!(
            newly_scanned > 60_000,
            "the part of the admitted set §16 bought barely ran: {newly_scanned}"
        );
        assert!(
            hits > 0 && hits < 2 * checked,
            "corpus must contain both hits and misses (hits={hits}, checked={checked})"
        );
        assert!(
            checked > 240_000,
            "the needle alphabet §13 widened for the last-byte shape shrank back: {checked} pairs"
        );
    }

    /// The shapes §17's last-byte rejection is built for, pinned pair by pair.
    /// The exhaustive corpora above cannot reach them: their haystacks are at
    /// most 3 characters / 9 bytes long, and "a 63-byte haystack and a 33-byte
    /// needle that share its first 31 bytes" is a shape that structurally does
    /// not exist in a sweep of ≤3-byte strings. So the long forms are spelled
    /// out here - §15.6's C and Ct ASCII forms, their CJK (`订`/`格`), Korean
    /// (`한`/`글`) and Greek (`α`/`β`) twins, the `n == h` / `n == h - 1` /
    /// `n > h` and 64-vs-65-byte gate-boundary forms, and the single-byte needle
    /// that takes the early return - 40 pairs in all, plus the 12 corner pairs
    /// below: the label and mixed shapes at 63, 64 and 65 bytes, each with one
    /// needle that is a real substring ending on the last byte and one that
    /// differs from it only there. Those two shapes are the ones `SCAN_MAX_BYTES`
    /// is argued from, so the boundary they are pinned at is the point.
    ///
    /// Every pair is asserted against the copying oracle `haystack
    /// .to_lowercase().contains(needle)`, with the needle folded the way callers
    /// fold it: this round moved *how* the scan gets to an answer, and the answer
    /// itself must not move by one bit. 0 of the 52 disagree (each would panic),
    /// and the teeth below say the corpus is genuinely adversarial rather than
    /// genuinely empty: enough of these haystacks pass the scan gate, and the set
    /// contains both hits and misses.
    #[test]
    fn near_miss_shapes_answer_as_the_copying_path_does() {
        let mut cases: Vec<(String, String)> = vec![
            // §15.6 C: shared prefix, mismatch at the needle's last byte.
            ("a".repeat(63), format!("{}·", "a".repeat(31))),
            ("a".repeat(63), format!("{}b", "a".repeat(31))),
            ("a".repeat(64), format!("{}·", "a".repeat(32))),
            ("a".repeat(64), format!("{}b", "a".repeat(32))),
            ("a".repeat(32), format!("{}·", "a".repeat(16))),
            ("a".repeat(32), format!("{}b", "a".repeat(16))),
            ("A".repeat(63), format!("{}·", "a".repeat(31))),
            ("A".repeat(63), "a".repeat(31)),
            ("a".repeat(63), "a".repeat(31)),
            // n == h, n == h - 1, n > h, and the 64/65-byte gate edge.
            ("a".repeat(64), "a".repeat(64)),
            ("a".repeat(64), "a".repeat(63)),
            ("a".repeat(64), "a".repeat(65)),
            ("a".repeat(63), "a".repeat(63)),
            ("a".repeat(63), "a".repeat(64)),
            ("a".repeat(65), format!("{}·", "a".repeat(32))),
            ("a".repeat(65), "a".repeat(64)),
            ("a".repeat(66), "订".repeat(22)),
            // needle of one byte: the early return must keep answering, too.
            ("a".repeat(63), "b".to_string()),
            ("a".repeat(63), "A".to_string()),
            // CJK.
            ("订".repeat(21), format!("{}格", "订".repeat(10))),
            ("订".repeat(21), format!("{}订", "订".repeat(10))),
            ("订".repeat(21), "订".repeat(21)),
            ("订".repeat(21), "订".repeat(20)),
            ("订".repeat(21), "订".repeat(22)),
            (
                format!("格{}", "订".repeat(20)),
                format!("{}格", "订".repeat(10)),
            ),
            (
                format!("{}格{}", "订".repeat(10), "订".repeat(10)),
                format!("{}格", "订".repeat(10)),
            ),
            (
                format!("{}格{}", "订".repeat(10), "订".repeat(10)),
                format!("格{}", "订".repeat(10)),
            ),
            // Korean.
            ("한".repeat(21), format!("{}글", "한".repeat(10))),
            ("한".repeat(21), format!("{}한", "한".repeat(10))),
            (
                format!("글{}", "한".repeat(20)),
                format!("{}글", "한".repeat(10)),
            ),
            (
                format!("{}글{}", "한".repeat(10), "한".repeat(10)),
                format!("{}글", "한".repeat(10)),
            ),
            ("한".repeat(21), "한".repeat(21)),
            // Greek.
            ("α".repeat(31), format!("{}β", "α".repeat(15))),
            ("α".repeat(31), format!("{}α", "α".repeat(15))),
            (
                format!("β{}", "α".repeat(15)),
                format!("{}β", "α".repeat(15)),
            ),
            (
                format!("{}β{}", "α".repeat(15), "α".repeat(15)),
                format!("{}β", "α".repeat(15)),
            ),
            // micro sign and sharp s, plus a mid-string separator.
            ("µ".repeat(30), format!("{}ß", "µ".repeat(15))),
            ("µ".repeat(30), "µ".repeat(15)),
            ("ß".repeat(31), format!("{}µ", "ß".repeat(15))),
            (
                format!("{}·{}", "a".repeat(20), "a".repeat(20)),
                format!("{}·", "a".repeat(20)),
            ),
        ];
        // The two shapes the boundary is measured on, pinned at the three byte
        // lengths that straddle it: `<prefix><x run><marker>`, built to the exact
        // length below so the arithmetic cannot drift. Each gets one needle that
        // is that tail - ending on the haystack's last byte - and one that
        // differs from it only there. 63 and 64 bytes answer from the scan, 65
        // from the copy, and both routes must return the same bit.
        let shapes = [("订单 · 表格 · ", "报表"), ("中文表格 · 列值 ", "订单")];
        for (prefix, marker) in shapes {
            for len in [63usize, 64, 65] {
                let pad = len - prefix.len() - marker.len();
                let haystack = format!("{}{}{}", prefix, "x".repeat(pad), marker);
                assert_eq!(
                    haystack.len(),
                    len,
                    "shape {:?} does not build a {}-byte haystack",
                    prefix,
                    len
                );
                let hit = format!("{}{}", "x".repeat(pad), marker);
                let miss = format!("{}{}z", "x".repeat(pad), marker);
                cases.push((haystack.clone(), hit));
                cases.push((haystack, miss));
            }
        }
        let mut scanned = 0usize;
        let mut hits = 0usize;
        for (haystack, raw_needle) in &cases {
            let needle = raw_needle.to_lowercase();
            let want = haystack.to_lowercase().contains(&needle);
            assert_eq!(
                contains_ignore_case(haystack, &needle),
                want,
                "contains(len={}, {haystack:?}, {needle:?})",
                haystack.len()
            );
            let want_prefix = haystack.to_lowercase().starts_with(&needle);
            assert_eq!(
                starts_with_ignore_case(haystack, &needle),
                want_prefix,
                "starts_with(len={}, {haystack:?}, {needle:?})",
                haystack.len()
            );
            hits += want as usize;
            scanned += (haystack.len() <= crate::fold::SCAN_MAX_BYTES && only_ascii_folds(haystack))
                as usize;
        }
        assert_eq!(cases.len(), 52, "the near-miss table moved");
        // 41 of the 52 haystacks are 64 bytes or shorter and admitted, so the
        // scan gate is what answers them: the 37 ASCII/CJK/Korean/Greek forms of
        // §15.6 plus the four 63- and 64-byte label and mixed corners (the three
        // 65/66-byte ones, and the three corners one byte past them, are here for
        // the boundary itself); without this the table could have drifted
        // entirely onto the copying path and still "passed".
        assert!(
            scanned >= 40,
            "the near-miss corpus barely takes the scan any more: {scanned}/{}",
            cases.len()
        );
        assert!(
            hits >= 21 && hits < cases.len(),
            "the corpus must contain both hits and misses (hits={hits}, checked={})",
            cases.len()
        );
    }

    /// §15.10's first gap, closed: the tests above lock the *answer*, so they
    /// stay green no matter how slow the route to it becomes - replace the scan
    /// with `to_lowercase` and 143,472 pairs still pass, only the performance is
    /// gone. This one locks the *path*, by calling the scan directly and counting
    /// the byte comparisons it makes.
    ///
    /// The cell is §15.6's worst one: 63 `'a'` bytes against a 33-byte needle
    /// that shares 31 of them. The plain nested window loop compares about 992
    /// bytes there (31 starts x 32 comparisons before the mismatch at the
    /// needle's tail); rejecting on the last byte needs one per start, 31 in
    /// all, and the bound below (`h.len() + n.len()` = 96) separates the two
    /// regimes by more than a factor of ten while staying loose enough to allow
    /// a legitimate variant.
    ///
    /// What this does NOT cover, stated plainly: it guards one shape, the
    /// mismatch at the end of a long shared prefix. A needle that mismatches in
    /// the *middle* of a corpus whose last byte is common (`a·a…` against `aaaa…`)
    /// stays quadratic and no assertion here goes red for it - §17.7 keeps it as
    /// an open hole. The counter is `#[cfg(test)]` and thread-local, so the
    /// product binary carries neither the increment nor the storage.
    #[test]
    fn the_scan_rejects_a_near_miss_in_one_comparison_per_start() {
        let h = "a".repeat(63);
        let needle = format!("{}·", "a".repeat(31)).to_lowercase();
        // The pair must really be on the scan route, or the count below measures
        // a copying path that never enters `ascii_contains`.
        assert!(h.len() <= crate::fold::SCAN_MAX_BYTES && only_ascii_folds(&h));
        assert_eq!(needle.len(), 33);
        assert_eq!(
            contains_ignore_case(&h, &needle),
            h.to_lowercase().contains(&needle),
            "the answer must not move"
        );
        reset_byte_compares();
        assert!(!ascii_contains(h.as_bytes(), needle.as_bytes()));
        let compares = byte_compares();
        let bound = (h.len() + needle.len()) as u32;
        assert!(compares > 0, "the scan ran but made no comparison at all");
        assert!(
            compares <= bound,
            "the near miss cost {compares} byte comparisons; one per start would \
             be {}, the plain nested window loop about {}, the budget this test \
             allows is {bound}",
            h.len() - needle.len() + 1,
            (h.len() - needle.len() + 1) * needle.len()
        );

        // Same guard on the pure-ASCII twin (§15.6's Ct form) and on the 64-byte
        // variant, so a filter that only works for a two-byte needle tail would
        // not pass for both.
        for tail in ["b", "·"] {
            let haystack = "a".repeat(64);
            let nee = format!("{}{}", "a".repeat(32), tail).to_lowercase();
            assert!(haystack.len() <= crate::fold::SCAN_MAX_BYTES && only_ascii_folds(&haystack));
            reset_byte_compares();
            assert!(!ascii_contains(haystack.as_bytes(), nee.as_bytes()));
            let compares = byte_compares();
            let bound = (haystack.len() + nee.len()) as u32;
            assert!(
                compares <= bound,
                "Ct near miss cost {compares} comparisons, budget {bound}"
            );
        }

        // And the bound must not be an accident of "the needle is never there":
        // a hit one window in still stops counting as soon as it matches.
        let hit = "a".repeat(31);
        reset_byte_compares();
        assert!(ascii_contains(h.as_bytes(), hit.as_bytes()));
        let compares = byte_compares();
        assert!(
            compares <= h.len() as u32,
            "a hit at the first start cost {compares} comparisons on a {}-byte \
             corpus, so the scan is not short-circuiting",
            h.len()
        );
    }

    /// The bound now routes between two allocation-free scans, it does not
    /// decide the answer: for a haystack [`only_ascii_folds`] admits, the window
    /// ring below `SCAN_MAX_BYTES`, the prefolded buffer above it and the
    /// copying path above `SCRATCH_MAX_BYTES` must all return the same bit. The
    /// shapes are the ones the route is argued from - plain ASCII, an admitted
    /// `·`-style label with a non-ASCII character in it, and an admitted
    /// lowercase Greek one - walked from 63 B (inside the bound) to 512 B
    /// (outside it). Every cell is checked against the both-sides-ASCII-folded
    /// reference, which is what all three routes here do, and the folded needles
    /// are additionally checked against `haystack.to_lowercase().contains(needle)`,
    /// the equivalence the contract promises and that must not move.
    #[test]
    fn the_answer_does_not_depend_on_the_bound_for_admitted_haystacks() {
        fn corpus(len: usize, tail: &str) -> String {
            let haystack = format!("{}{}", "a".repeat(len - tail.len()), tail);
            assert_eq!(haystack.len(), len, "builder missed {len} bytes");
            assert!(
                only_ascii_folds(&haystack),
                "{haystack:?} is not admitted at len={len}"
            );
            haystack
        }

        let lengths = [63usize, 64, 65, 96, 128, 512];
        let shapes = [("ascii", "zqx"), ("label", "é zqx"), ("greek", "σ zqx")];
        let needles = ["zqx", "ZQX", "ZqX", "é", "σ", "Σ", "α", "zqxz"];
        let mut cells = 0usize;
        let mut inside_bound = 0usize;
        let mut outside_bound = 0usize;
        for (name, tail) in shapes {
            for needle in needles {
                let mut answers = Vec::with_capacity(lengths.len());
                for len in lengths {
                    let haystack = corpus(len, tail);
                    let answer = contains_ignore_case(&haystack, needle);
                    let folded_both = haystack
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase());
                    assert_eq!(
                        answer, folded_both,
                        "{name} len={len} needle={needle:?}: the route must answer as \
                         if both sides were ASCII-folded"
                    );
                    if needle == needle.to_lowercase() {
                        assert_eq!(
                            answer,
                            haystack.to_lowercase().contains(needle),
                            "{name} len={len} needle={needle:?}: the contracted \
                             equivalence moved"
                        );
                    }
                    if haystack.len() <= crate::fold::SCAN_MAX_BYTES {
                        inside_bound += 1;
                    } else {
                        outside_bound += 1;
                    }
                    answers.push(answer);
                    cells += 1;
                }
                // The name of the test, said plainly: one shape and one needle,
                // answered at six lengths that straddle the bound, is one answer
                // repeated six times - the 63/64 B cells from the ring, the rest
                // from the buffer.
                assert!(
                    answers.iter().all(|a| *a == answers[0]),
                    "{name} needle={needle:?} answered {answers:?} across the bound"
                );
            }
        }
        assert_eq!(cells, 3 * 8 * 6, "3 shapes x 8 needles x 6 lengths");
        assert_eq!(inside_bound, 48, "two of six lengths sit inside the bound");
        assert_eq!(outside_bound, 96, "and four of six sit outside it");
    }

    /// What the route above the bound does per call, counted: it walks the
    /// haystack once and the needle once - never twice, never once per window
    /// start - it leaves the buffer alone above `SCRATCH_MAX_BYTES`, and it does
    /// not run at all for a haystack the gate refuses. `byte_compares` is read
    /// alongside because the two counters do not overlap: the prefolded route
    /// calls no `byte_eq`, so above the bound this guard would read zero and the
    /// fold count is the only observable that has to stay linear.
    #[test]
    fn the_scratch_route_folds_once_per_byte_and_falls_back_above_the_cap() {
        let haystack = "a".repeat(127);
        let needle = format!("{}b", "a".repeat(31));
        assert!(haystack.len() > crate::fold::SCAN_MAX_BYTES);
        assert!(only_ascii_folds(&haystack));
        crate::fold::reset_scratch_folds();
        reset_byte_compares();
        assert!(!contains_ignore_case(&haystack, &needle));
        assert_eq!(
            crate::fold::scratch_folds() as usize,
            haystack.len() + needle.len(),
            "one fold per byte of the corpus plus one per byte of the needle, \
             never a pass per start"
        );
        assert_eq!(
            byte_compares(),
            0,
            "the window ring is not what answers above the bound"
        );

        // A longer needle adds exactly its own bytes: nothing here scales like
        // the ring's (h - m + 1) x m, which is what the bound-internal guard
        // measures and what this route cannot be measured by.
        let long_needle = format!("{}b", "a".repeat(100));
        crate::fold::reset_scratch_folds();
        assert!(!contains_ignore_case(&haystack, &long_needle));
        assert_eq!(
            crate::fold::scratch_folds() as usize,
            haystack.len() + long_needle.len(),
            "the corpus was walked once, not once per needle byte"
        );

        // Above the buffer cap the route declines: the copying path's answer,
        // and no fold at all. Both fallbacks land in the same place.
        let over_cap = "a".repeat(crate::fold::SCRATCH_MAX_BYTES + 1);
        crate::fold::reset_scratch_folds();
        reset_byte_compares();
        assert_eq!(
            contains_ignore_case(&over_cap, "aa"),
            over_cap.to_lowercase().contains("aa"),
            "the fallback must answer the truth"
        );
        assert!(contains_ignore_case(&over_cap, "aa"));
        assert_eq!(
            crate::fold::scratch_folds(),
            0,
            "a haystack over SCRATCH_MAX_BYTES must not touch the buffer"
        );

        // The cap counts the bytes the buffer has to hold, corpus plus needle,
        // so one byte of needle at exactly the cap is already the copying path
        // and one byte short of it still prefolds.
        let at_cap = "a".repeat(crate::fold::SCRATCH_MAX_BYTES - 2);
        crate::fold::reset_scratch_folds();
        assert!(contains_ignore_case(&at_cap, "aa"));
        assert_eq!(
            crate::fold::scratch_folds() as usize,
            at_cap.len() + 2,
            "a corpus that fills the buffer exactly still folds once per byte"
        );
        let at_cap_plus_needle = "a".repeat(crate::fold::SCRATCH_MAX_BYTES - 1);
        crate::fold::reset_scratch_folds();
        assert!(contains_ignore_case(&at_cap_plus_needle, "aa"));
        assert_eq!(
            crate::fold::scratch_folds(),
            0,
            "corpus plus needle over the cap must decline, not resize the buffer"
        );

        // A haystack the gate refuses never reaches the fold route either, at
        // any length. Uppercase Cyrillic is what the gate is for: `is_ascii`
        // waves ASCII through whatever its case, so only a character with a
        // non-ASCII lowercase mapping keeps a long haystack on the copy.
        let cased = "МОСКВА".repeat(40);
        assert!(cased.len() > crate::fold::SCAN_MAX_BYTES);
        assert!(!only_ascii_folds(&cased));
        crate::fold::reset_scratch_folds();
        assert_eq!(
            contains_ignore_case(&cased, "москва"),
            cased.to_lowercase().contains("москва"),
            "a case-mapped haystack keeps the copying path's answer"
        );
        assert!(contains_ignore_case(&cased, "москва"));
        assert_eq!(crate::fold::scratch_folds(), 0, "refused means unfolded");

        // And below the bound nothing folds: the ring answers there, unchanged.
        let inside = "a".repeat(64);
        crate::fold::reset_scratch_folds();
        assert!(contains_ignore_case(&inside, "aa"));
        assert_eq!(crate::fold::scratch_folds(), 0, "the ring folds nothing");
    }

    /// The one behaviour change of this round, on its own: an *unfolded* needle
    /// used to be answered differently on the two sides of `SCAN_MAX_BYTES`,
    /// because the window ring ASCII-folds the needle it is handed while the
    /// copying path left it alone. The route above the bound folds the needle
    /// too, so the two sides now give the same bit for the same pair - which
    /// makes the entry's answer for an unfolded needle independent of the
    /// haystack's length again, and does not add a new inconsistency to it. The
    /// agreement is on ASCII's fold and stops there: a needle whose case only
    /// Unicode folding would remove still matches nowhere, on either side, and
    /// that is what the last cell below pins.
    ///
    /// This replaces the design of §六's needle regression, two cells with
    /// 3-byte haystacks. Both of those sit inside the bound, so they could not
    /// see the flips above it - they stayed green whether the answer past the
    /// bound widened or narrowed. And one of them, `("aBc", "aB")`, already
    /// answered `true` before this round: a haystack that is all ASCII is
    /// admitted by the gate through its `is_ascii` fast path whatever its case,
    /// and the ring folds the needle it is handed. A haystack the gate refuses
    /// has to carry a non-ASCII cased letter - `İstanbul`, `"МОСКВА"` - and
    /// those went to the copying path at every length, so they were never the
    /// pair that flipped.
    #[test]
    fn an_unfolded_needle_answers_the_same_way_on_both_sides_of_the_bound() {
        let lengths = [63usize, 64, 65, 96, 128, 512];
        let mut cells = 0usize;
        let mut wider_inside = 0usize;
        let mut wider_outside = 0usize;
        for tail in ["zqx", "é zqx"] {
            for needle in ["ZQX", "ZqX", "xZQ", "Σ"] {
                let mut answers = Vec::with_capacity(lengths.len());
                for len in lengths {
                    let haystack = format!("{}{}", "a".repeat(len - tail.len()), tail);
                    assert_eq!(haystack.len(), len, "builder missed {len} bytes");
                    assert!(only_ascii_folds(&haystack), "{haystack:?} refused");
                    let answer = contains_ignore_case(&haystack, needle);
                    let folded_both = haystack
                        .to_ascii_lowercase()
                        .contains(&needle.to_ascii_lowercase());
                    assert_eq!(
                        answer, folded_both,
                        "len={len}, needle={needle:?} must answer as both-folded"
                    );
                    // Where an unfolded ASCII needle is findable at all, the
                    // copying path that used to answer outside the bound says
                    // `false`: inside it the ring always did, outside it the
                    // change of this round does.
                    if folded_both && needle != needle.to_ascii_lowercase() {
                        assert!(
                            !haystack.to_lowercase().contains(needle),
                            "len={len}, needle={needle:?}: the copy must still be \
                             the one that misses"
                        );
                        if len <= crate::fold::SCAN_MAX_BYTES {
                            wider_inside += 1;
                        } else {
                            wider_outside += 1;
                        }
                    }
                    answers.push(answer);
                    cells += 1;
                }
                assert!(
                    answers.iter().all(|a| *a == answers[0]),
                    "needle {needle:?} answered {answers:?} across the bound for \
                     tail {tail:?}"
                );
            }
        }
        assert_eq!(cells, 2 * 4 * 6, "2 tails x 4 needles x 6 lengths");
        assert_eq!(
            wider_inside, 8,
            "two unfolded ASCII needles x 2 tails x the two lengths inside the bound"
        );
        assert_eq!(
            wider_outside, 16,
            "...and the same pair over the four lengths outside it, which is the \
             half that changed"
        );

        // The same three needles named at the four lengths that straddle the
        // bound: 63 and 64 answer from the ring, 65 and 96 from the reused
        // buffer, and the bit must not care which. These are the pairs §六's
        // two 3-byte cells could never reach.
        let tail = "zqx";
        let at = |len: usize| {
            let haystack = format!("{}{}", "a".repeat(len - tail.len()), tail);
            assert_eq!(haystack.len(), len, "builder missed {len} bytes");
            assert!(only_ascii_folds(&haystack), "{haystack:?} refused");
            haystack
        };
        let h63 = at(63);
        let h64 = at(64);
        let h65 = at(65);
        let h96 = at(96);
        for needle in ["ZQX", "ZqX", "xZQ"] {
            let inside = contains_ignore_case(&h64, needle);
            for (len, other) in [(63usize, &h63), (65, &h65), (96, &h96)] {
                let answer = contains_ignore_case(other, needle);
                assert_eq!(answer, inside, "{len} vs 64 with {needle:?}");
            }
        }

        // The widening stops at ASCII: `Σ` against a haystack of admitted
        // lowercase Greek matches nowhere, inside or outside the bound, even
        // though Unicode folding both sides would find it. Byte routes have
        // never done Unicode case, and this round did not start.
        let sigma = format!("{}σ", "a".repeat(70));
        assert!(only_ascii_folds(&sigma));
        assert!(!contains_ignore_case(&sigma, "Σ"));
        assert!(!contains_ignore_case(&format!("{}σ", "a".repeat(200)), "Σ"));
        assert!(
            sigma.to_lowercase().contains("σ"),
            "the Unicode truth would say true, and neither route here does"
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

    #[test]
    fn smart_indent_steps_only_on_trigger_lines() {
        // 触发词：换行带一级缩进，光标落在缩进之后，且仍在插入模式。
        let mut v = buf("BEGIN");
        v.caret = 5;
        v.mode = VimMode::Insert;
        let step = v.step("enter", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("BEGIN\n  "));
        assert_eq!(v.caret, 8);
        assert!(step.to_insert);
        assert!(matches!(v.mode, VimMode::Insert));

        // 非触发词不接管，交回原生编辑器插裸换行。
        let mut v = buf("SELECT * FROM t");
        v.caret = 15;
        v.mode = VimMode::Insert;
        assert!(v.step("enter", false, false, false).is_none());

        // 整词匹配；注释与字符串字面量开头的行不算语句。
        for text in ["FORM", "THENISH", "-- BEGIN", "/* BEGIN", "'BEGIN'"] {
            let mut v = buf(text);
            v.caret = text.len();
            v.mode = VimMode::Insert;
            assert!(v.step("enter", false, false, false).is_none(), "{text}");
        }

        // 中间行命中也缩进，其余行原样保留。
        let mut v = buf("select 1\nwhen x = 1 then\nselect 2");
        v.caret = 24; // end of the `when` line
        v.mode = VimMode::Insert;
        let step = v.step("enter", false, false, false).unwrap();
        assert_eq!(
            step.text.as_deref(),
            Some("select 1\nwhen x = 1 then\n  \nselect 2")
        );
    }

    #[test]
    fn smart_indent_undo_restores_the_buffer() {
        let mut v = buf("LOOP");
        v.caret = 4;
        v.mode = VimMode::Insert;
        let step = v.step("enter", false, false, false).unwrap();
        assert_eq!(step.text.as_deref(), Some("LOOP\n  "));
        v.step("escape", false, false, false).unwrap();
        assert!(matches!(v.mode, VimMode::Normal));
        let undone = v.step("u", false, false, false).unwrap();
        assert_eq!(undone.text.as_deref(), Some("LOOP"));
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

/// The panels that mirror an `AppState` list keep a private copy and re-sync it
/// from a global observer. GPUI fires that observer on *any* mutable access to
/// `AppState`, so the copy is only refreshed when the two lists differ -- which
/// makes field-level `PartialEq` on the mirrored types load bearing: a field the
/// comparison cannot see would keep the panel showing stale data.
#[cfg(test)]
mod snapshot_eq_tests {
    use crate::state::FavoriteEntry;
    use dbstudio_core::models::{ConnectionConfig, DatabaseType, Environment, SslMode};
    use dbstudio_core::schema::{TableInfo, TableType};
    use dbstudio_storage::types::QueryHistoryEntry;

    fn table() -> TableInfo {
        TableInfo {
            name: "orders".to_string(),
            schema: Some("public".to_string()),
            database: Some("app".to_string()),
            table_type: TableType::Table,
            row_count: Some(12),
            comment: Some("audit trail".to_string()),
        }
    }

    fn history() -> QueryHistoryEntry {
        QueryHistoryEntry {
            id: 7,
            connection_id: "conn-1".to_string(),
            sql: "SELECT 1".to_string(),
            execution_time_ms: 42,
            row_count: Some(1),
            is_error: false,
            executed_at: "2026-09-30 10:00:00".to_string(),
        }
    }

    fn favorite() -> FavoriteEntry {
        FavoriteEntry {
            id: 3,
            name: "Top customers".to_string(),
            sql: "SELECT 1".to_string(),
            connection_id: Some("conn-1".to_string()),
        }
    }

    fn connection() -> ConnectionConfig {
        ConnectionConfig::new(DatabaseType::PostgreSQL, "prod-db".to_string())
    }

    type EditFn<T> = fn(&mut T);

    /// Applies each edit to a clone of `base` and requires the result to differ
    /// from `base`. Comparing against a saved clone (not a freshly built value)
    /// matters: `ConnectionConfig::new` mints a random id and timestamp, so a
    /// fresh second value would differ on its own and prove nothing.
    fn each_edit_changes<T: Clone + PartialEq + std::fmt::Debug>(
        base: &T,
        edits: &[(&str, EditFn<T>)],
    ) {
        assert_eq!(base, &base.clone());
        for (field, edit) in edits {
            let mut v = base.clone();
            edit(&mut v);
            assert_ne!(&v, base, "field `{field}` is invisible to PartialEq");
        }
    }

    /// Every field of every mirrored type must participate in equality, so a
    /// one-field edit can never be mistaken for "nothing changed".
    #[test]
    fn equality_is_field_sensitive() {
        each_edit_changes(
            &table(),
            &[
                ("name", |t| t.name = "orders_2".into()),
                ("schema", |t| t.schema = None),
                ("database", |t| t.database = None),
                ("table_type", |t| t.table_type = TableType::View),
                ("row_count", |t| t.row_count = None),
                ("comment", |t| t.comment = None),
            ],
        );

        each_edit_changes(
            &history(),
            &[
                ("id", |h| h.id += 1),
                ("connection_id", |h| h.connection_id = "conn-2".into()),
                ("sql", |h| h.sql.push(' ')),
                ("execution_time_ms", |h| h.execution_time_ms += 1),
                ("row_count", |h| h.row_count = None),
                ("is_error", |h| h.is_error = true),
                ("executed_at", |h| {
                    h.executed_at = "2026-09-30 10:00:01".into()
                }),
            ],
        );

        each_edit_changes(
            &favorite(),
            &[
                ("id", |f| f.id += 1),
                ("name", |f| f.name = "Other".into()),
                ("sql", |f| f.sql = "SELECT 2".into()),
                ("connection_id", |f| f.connection_id = None),
            ],
        );

        each_edit_changes(
            &connection(),
            &[
                ("id", |c| c.id = "other".into()),
                ("name", |c| c.name = "stage-db".into()),
                ("db_type", |c| c.db_type = DatabaseType::MySQL),
                ("host", |c| c.host = "db.internal".into()),
                ("port", |c| c.port += 1),
                ("database", |c| c.database = "app".into()),
                ("username", |c| c.username = "readonly".into()),
                ("color", |c| c.color = Some("#000000".into())),
                ("environment", |c| c.environment = Environment::Production),
                ("ssl_mode", |c| c.ssl_mode = SslMode::Require),
                ("group", |c| c.group = Some("Prod".into())),
                ("tags", |c| c.tags = vec!["core".into()]),
                ("ssh_enabled", |c| c.ssh_enabled = true),
                ("ssh_host", |c| c.ssh_host = Some("bastion".into())),
                ("ssh_port", |c| c.ssh_port = Some(2222)),
                ("ssh_username", |c| c.ssh_username = Some("tunnel".into())),
                ("ssh_auth_type", |c| c.ssh_auth_type = Some("key".into())),
                ("ssh_key_path", |c| c.ssh_key_path = Some("/k".into())),
                ("extra_params", |c| {
                    c.extra_params = Some("sslmode=require".into())
                }),
                ("plugin_name", |c| c.plugin_name = Some("duckdb".into())),
                ("created_at", |c| {
                    c.created_at = "2026-01-01 00:00:00".into()
                }),
                ("updated_at", |c| {
                    c.updated_at = "2026-09-30 10:00:00".into()
                }),
            ],
        );
    }

    /// The mirrored lists are compared as slices: the history panel prepends, so
    /// a pure reordering has to count as a change.
    #[test]
    fn list_equality_is_order_sensitive() {
        let a = history();
        let mut b = history();
        b.id += 1;
        assert_ne!(vec![a.clone(), b.clone()], vec![b.clone(), a.clone()]);
        assert_eq!(vec![a.clone(), b.clone()], vec![a, b]);
    }
}
