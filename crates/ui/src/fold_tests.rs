#[cfg(test)]
mod scan_bound_evidence_tests {
    use crate::fold::{
        ascii_chunk_contains, ascii_chunk_contains_windowed, byte_compares, chunk_folds,
        contains_ignore_case, only_ascii_folds, reset_byte_compares, reset_chunk_folds,
        reset_scratch_folds, scratch_folds, CHUNK_MAX_BYTES, SCAN_MAX_BYTES, SCRATCH_MAX_BYTES,
    };

    /// The reference every case below is paired against: the copying path,
    /// route (iii), which is the semantic floor of this module.
    fn reference(haystack: &str, needle: &str) -> bool {
        haystack.to_lowercase().contains(needle)
    }

    /// The three pools of the round's bench, verbatim: 90 one-byte, 22 two-byte
    /// and 24 three-byte characters, every one of them gate-true (asserted at
    /// run time below, exactly as the bench asserts it).
    fn pool1() -> Vec<char> {
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-.,;:()[]{}<>=+*/!@#$%&~^|?"
            .iter()
            .map(|b| *b as char)
            .collect()
    }
    fn pool2() -> Vec<char> {
        "·ßµé¼§°×÷àòõλπςабиогдж".chars().collect()
    }
    fn pool3() -> Vec<char> {
        "中文表格订单数据库行列索引용한글사용자名称区域값"
            .chars()
            .collect()
    }

    /// `bench.rs:147-152` - the round's generator, so seeds mean the same thing
    /// here that they mean in the timed cells.
    fn lcg(x: &mut u64) -> u64 {
        *x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *x >> 33
    }

    /// `p.min(len)`, walked back to a character boundary (`bench.rs:224-230`).
    fn floor_b(s: &str, p: usize) -> usize {
        let mut p = p.min(s.len());
        while p > 0 && !s.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    /// `text(fam, len, seed)` of the bench (`bench.rs:165-222`): a gate-true
    /// string of EXACTLY `len` bytes - `A` all ASCII, `C` three-byte-dominated
    /// with the remainder carried by two-byte characters, `X` a mix of 1/2/3.
    fn text(fam: &str, len: usize, seed: u64) -> String {
        let p1 = pool1();
        let p2 = pool2();
        let p3 = pool3();
        let mut x = seed;
        let mut s = String::new();
        while s.len() < len {
            let rem = len - s.len();
            let c = match fam {
                "A" => p1[(lcg(&mut x) % p1.len() as u64) as usize],
                "C" => {
                    if rem >= 3 {
                        p3[(lcg(&mut x) % p3.len() as u64) as usize]
                    } else if rem == 2 {
                        p2[(lcg(&mut x) % p2.len() as u64) as usize]
                    } else {
                        p1[(lcg(&mut x) % p1.len() as u64) as usize]
                    }
                }
                _ => {
                    let r = lcg(&mut x) % 100;
                    if r < 40 {
                        p1[(lcg(&mut x) % p1.len() as u64) as usize]
                    } else if r < 70 && rem >= 2 {
                        p2[(lcg(&mut x) % p2.len() as u64) as usize]
                    } else if rem >= 3 {
                        p3[(lcg(&mut x) % p3.len() as u64) as usize]
                    } else if rem == 2 {
                        p2[(lcg(&mut x) % p2.len() as u64) as usize]
                    } else {
                        p1[(lcg(&mut x) % p1.len() as u64) as usize]
                    }
                }
            };
            s.push(c);
        }
        assert_eq!(s.len(), len, "byte-length drifted for fam={fam}");
        s
    }

    /// The planted region (`bench.rs:298-351`): a legal char string of 8 bytes
    /// (9-10 for a multi-byte tail), carrying an uppercase ASCII letter inside
    /// so the case-insensitive half of the comparison has work to do on every
    /// family, and ending in the byte class the shape asked for. `None` for a
    /// pure-ASCII family with a multi-byte tail requirement - a family `A` cell
    /// has no continuation byte anywhere in it, which is why `nm_last_cont` is
    /// not constructible there and the table test skips it out loud.
    fn region(fam: &str, x: &mut u64, multi_tail: bool) -> Option<String> {
        let p1 = pool1();
        let p2 = pool2();
        let p3 = pool3();
        let mut s = String::new();
        match (fam, multi_tail) {
            ("A", false) => {
                for i in 0..8u64 {
                    let c = p1[(lcg(x) % p1.len() as u64) as usize];
                    s.push(if i == 2 {
                        c.to_ascii_uppercase()
                    } else {
                        c.to_ascii_lowercase()
                    });
                }
            }
            ("C", false) => {
                s.push(p3[(lcg(x) % p3.len() as u64) as usize]);
                s.push(p3[(lcg(x) % p3.len() as u64) as usize]);
                s.push('K');
                s.push(p1[(lcg(x) % p1.len() as u64) as usize].to_ascii_lowercase());
            }
            ("C", true) => {
                s.push(p3[(lcg(x) % p3.len() as u64) as usize]);
                s.push(p3[(lcg(x) % p3.len() as u64) as usize]);
                s.push('K');
                s.push(p3[(lcg(x) % p3.len() as u64) as usize]);
            }
            ("X", false) => {
                s.push(p3[(lcg(x) % p3.len() as u64) as usize]);
                s.push(p2[(lcg(x) % p2.len() as u64) as usize]);
                s.push('M');
                s.push(p1[(lcg(x) % p1.len() as u64) as usize].to_ascii_lowercase());
                s.push(p1[(lcg(x) % p1.len() as u64) as usize].to_ascii_lowercase());
            }
            ("X", true) => {
                s.push(p3[(lcg(x) % p3.len() as u64) as usize]);
                s.push(p2[(lcg(x) % p2.len() as u64) as usize]);
                s.push('M');
                s.push(p3[(lcg(x) % p3.len() as u64) as usize]);
            }
            _ => return None,
        }
        let last = s.chars().last().unwrap();
        if !multi_tail && last.len_utf8() != 1 {
            return None;
        }
        if multi_tail && last.len_utf8() < 2 {
            return None;
        }
        Some(s)
    }

    /// A byte moved to a neighbouring byte of the SAME class, so the encoding
    /// survives (`bench.rs:257-292`). This is what lets `nm_penult` break the
    /// match one byte before the end without breaking the character.
    fn same_class_neighbour(b: u8) -> u8 {
        if b < 0x80 {
            let lo = b.to_ascii_lowercase();
            if lo.is_ascii_alphabetic() {
                if lo == b'a' {
                    b'b'
                } else {
                    b'a'
                }
            } else if b >= b'1' {
                b - 1
            } else {
                b + 1
            }
        } else if b <= 0xBF {
            if b == 0x80 {
                0x81
            } else {
                0x80
            }
        } else if b <= 0xDF {
            if b >= 0xCD {
                0xCE
            } else {
                0xC3
            }
        } else if b >= 0xE5 {
            0xE4
        } else {
            0xE5
        }
    }

    /// First folded byte at which `n` differs from `h` at the planted start,
    /// `-1` when it matches there (`bench.rs:359-371`). Read with plain
    /// `to_ascii_lowercase`, never through `byte_eq`, so that computing it does
    /// not pollute the route counters the assertions below depend on.
    fn first_diff(h: &str, n: &str, p: usize) -> isize {
        let hb = h.as_bytes();
        let nb = n.as_bytes();
        for i in 0..nb.len() {
            if p + i >= hb.len() {
                return i as isize;
            }
            if !hb[p + i].eq_ignore_ascii_case(&nb[i]) {
                return i as isize;
            }
        }
        -1
    }

    /// Does the ring's last-byte anchor pass at the planted start? True is the
    /// signature of a shape the anchor cannot reject in one comparison.
    fn anchor_passes(h: &str, n: &str, p: usize) -> bool {
        let (hb, nb) = (h.as_bytes(), n.as_bytes());
        p + nb.len() <= hb.len() && hb[p + nb.len() - 1].eq_ignore_ascii_case(&nb[nb.len() - 1])
    }

    struct Cell {
        h: String,
        n: String,
        /// byte offset of the planted region inside `h`
        p: usize,
    }

    /// The five *true* mismatch shapes of §4.2 - the ones whose answer must be
    /// the copying path's `false`. `nm_case` and `middle` are deliberately not
    /// among them: they are hit-side cells (only ASCII case differs, which an
    /// ASCII fold must still call a match), the ring stops early on them, and
    /// the round excluded early-stop cells from every multiple it reported
    /// (`RESULTS.md` §八-6). They appear in the assertions as *certified skips*.
    const SHAPES: [&str; 5] = [
        "nm_last_ascii",
        "nm_last_cont",
        "nm_combining",
        "nm_mid",
        "nm_penult",
    ];

    /// `build(fam, len, shape, seed)` of the bench (`bench.rs:394-563`), minus
    /// the `none` branch. `None` is the bench's own meaning: this shape is not
    /// constructible on this family, which is a declared fact rather than a gap.
    fn build(fam: &'static str, len: usize, shape: &'static str, seed: u64) -> Option<Cell> {
        let multi_tail = shape == "nm_last_cont";
        let mut attempt = 0u64;
        loop {
            let mut x = seed + attempt * 31;
            let reg = region(fam, &mut x, multi_tail)?;
            if reg.len() + 8 > len {
                return None;
            }
            let half = floor_b(&text(fam, len, seed), len / 2);
            let pre_len = half.min(len - reg.len());
            let post_len = len - pre_len - reg.len();
            let mut h = text(fam, pre_len, seed + 7 + attempt);
            h.push_str(&reg);
            h.push_str(&text(fam, post_len, seed + 13 + attempt));
            assert_eq!(h.len(), len);
            let p = pre_len;
            let rb = reg.as_bytes();
            let needle: String = match shape {
                // (a) only the last byte differs, at an ASCII position.
                "nm_last_ascii" => {
                    let mut v = rb.to_vec();
                    let last = *v.last().unwrap();
                    if last >= 0x80 {
                        return None;
                    }
                    let alt = if last == b'z' { b'a' } else { b'z' };
                    if alt == last {
                        return None;
                    }
                    *v.last_mut().unwrap() = alt;
                    String::from_utf8(v).ok()?
                }
                // (b) the last continuation byte of the region's final
                // multi-byte character differs.
                "nm_last_cont" => {
                    let mut v = rb.to_vec();
                    let i = v.len() - 1;
                    v[i] = if v[i] == 0xBF { 0x80 } else { 0xBF };
                    String::from_utf8(v).ok()?
                }
                // (d) a combining dot appended to the whole planted region.
                "nm_combining" => {
                    let mut s = reg.clone();
                    s.push('\u{307}');
                    s
                }
                // (g) an interior character replaced, last byte left intact: the
                // anchor passes and the inner loop has to walk.
                "nm_mid" => {
                    let mut chs: Vec<char> = reg.chars().collect();
                    let mid = chs.len() / 2;
                    let bl = chs[mid].len_utf8();
                    let pool = match bl {
                        1 => pool1(),
                        2 => pool2(),
                        _ => pool3(),
                    };
                    let repl = pool[(lcg(&mut x) % pool.len() as u64) as usize];
                    if repl.len_utf8() != bl || repl == chs[mid] {
                        attempt += 1;
                        continue;
                    }
                    chs[mid] = repl;
                    chs.iter().collect()
                }
                // (h) the penultimate byte differs: the anchor passes and the
                // inner loop runs all the way to byte `m - 1`.
                "nm_penult" => {
                    let mut v = rb.to_vec();
                    let i = v.len() - 2;
                    v[i] = same_class_neighbour(v[i]);
                    String::from_utf8(v).ok()?
                }
                _ => unreachable!("unknown shape {shape}"),
            };
            let n = needle.to_lowercase();
            // The shape is a true mismatch only if the folded haystack does not
            // contain the needle anywhere at all - the same requirement the
            // bench imposed before it timed the cell.
            if !reference(&h, &n) {
                return Some(Cell { h, n, p });
            }
            attempt += 1;
            assert!(
                attempt < 300,
                "shape {shape} never planted fam={fam} len={len}"
            );
        }
    }

    /// An absent needle for the routing sweep: taken from a *different* corpus
    /// of the same family, so no leg can stop early (`bench.rs:395-431`).
    fn build_absent(fam: &'static str, len: usize, seed: u64) -> Cell {
        let want_bytes = match fam {
            "A" => 8usize,
            "C" => 7,
            _ => 8,
        };
        let mut attempt = 0u64;
        loop {
            let h = text(fam, len, seed + attempt);
            let other = text(fam, len + 4096, 999_888 + attempt);
            let p0 = floor_b(&other, other.len() / 2);
            let p1b = floor_b(&other, p0 + want_bytes).max(p0 + 1);
            let n = other[p0..p1b].to_lowercase();
            if !n.is_empty() && !reference(&h, &n) {
                return Cell { h, n, p: p0 };
            }
            attempt += 1;
            assert!(attempt < 300, "no absent needle found fam={fam} len={len}");
        }
    }

    /// Semantics, at the three lengths the round owed evidence for: for every
    /// gate-true family, every true near-miss shape and every length in
    /// {64, 65, 96} B, the entry's answer is the copying path's answer, and the
    /// answer is `false` (so a near miss cannot be reported as a hit on either
    /// allocation-free route). The shape's own meaning is asserted too - where
    /// the first folded byte differs, and whether the ring's last-byte anchor
    /// passes there - because that is the only self-certification the round's
    /// raw data has for its shapes (the `kind_ok` field never reached the disk;
    /// `RESULTS.md` §八-2).
    #[test]
    fn a_gate_true_near_miss_answers_the_copying_path_at_every_shape_and_length() {
        // The pools are the gate-true precondition, checked rather than claimed.
        for pool in [pool1(), pool2(), pool3()] {
            for c in pool {
                let mut one = String::new();
                one.push(c);
                assert!(only_ascii_folds(&one), "pool character {c:?}");
            }
        }
        assert_eq!(pool1().len(), 90, "pool1 width drifted from the bench");
        assert_eq!(pool2().len(), 22, "pool2 width drifted from the bench");
        assert_eq!(pool3().len(), 24, "pool3 width drifted from the bench");

        let mut tested = 0usize;
        let mut hits = 0usize;
        let mut skipped: Vec<String> = Vec::new();
        for fam in ["A", "C", "X"] {
            for shape in SHAPES {
                for len in [64usize, 65, 96] {
                    let Some(c) = build(fam, len, shape, 12345) else {
                        // `nm_last_cont` on a pure-ASCII corpus: nothing to skip
                        // past, no empty cell left behind.
                        skipped.push(format!("{fam}/{shape}/{len}"));
                        continue;
                    };
                    assert_eq!(c.h.len(), len, "{fam}/{shape}/{len} length");
                    assert!(only_ascii_folds(&c.h), "{fam}/{shape}/{len} gate");
                    assert!(only_ascii_folds(&c.n), "{fam}/{shape}/{len} needle");
                    assert_eq!(
                        c.n,
                        c.n.to_lowercase(),
                        "the caller's contract: {fam}/{shape}/{len} needle is folded"
                    );
                    let want = reference(&c.h, &c.n);
                    hits += want as usize;
                    // Every one of these five shapes is a true mismatch, so the
                    // reference itself says `false`: this test would be vacuous
                    // if it only ever compared two `true`s.
                    assert!(!want, "{fam}/{shape}/{len} must not match");
                    let got = contains_ignore_case(&c.h, &c.n);
                    assert_eq!(got, want, "entry({fam}/{shape}/{len}, {} B)", c.h.len());
                    // Route, read through the counters: (i) at 64 B, (ii) above
                    // it, and the buffer past `SCRATCH_MAX_BYTES` is out of
                    // reach at these lengths so (iii) cannot be entered by the
                    // window route either.
                    assert!(c.h.len() + c.n.len() <= SCRATCH_MAX_BYTES);
                    reset_byte_compares();
                    reset_scratch_folds();
                    reset_chunk_folds();
                    assert_eq!(contains_ignore_case(&c.h, &c.n), want);
                    let (cmp, folds, chunk) = (byte_compares(), scratch_folds(), chunk_folds());
                    assert_eq!(chunk, 0, "{fam}/{shape}/{len} reached the window route");
                    if len <= SCAN_MAX_BYTES {
                        assert!(cmp > 0, "{fam}/{shape}/{len} did not run the ring");
                        assert_eq!(folds, 0, "{fam}/{shape}/{len} folded, so it left (i)");
                    } else {
                        assert_eq!(
                            folds as usize,
                            c.h.len() + c.n.len(),
                            "{fam}/{shape}/{len} did not fold once per byte"
                        );
                        assert_eq!(cmp, 0, "{fam}/{shape}/{len} ran the ring above the bound");
                    }
                    // Shape certification, per shape, on the cell as built.
                    let m = c.n.len();
                    let mm = first_diff(&c.h, &c.n, c.p);
                    assert!(
                        mm >= 1,
                        "{fam}/{shape}/{len}: first diff at {mm} - a needle that \
                         differs at byte 0 is an absent needle, not a near miss"
                    );
                    match shape {
                        "nm_last_ascii" => {
                            assert_eq!(mm as usize, m - 1, "{fam}/{shape}/{len} diff spot");
                            assert!(c.n.as_bytes()[m - 1] < 0x80);
                            assert!(c.h.as_bytes()[c.p + m - 1] < 0x80);
                        }
                        "nm_last_cont" => {
                            assert_eq!(mm as usize, m - 1, "{fam}/{shape}/{len} diff spot");
                            let hb = c.h.as_bytes()[c.p + m - 1];
                            let nb = c.n.as_bytes()[m - 1];
                            assert!(
                                (0x80..=0xBF).contains(&hb) && (0x80..=0xBF).contains(&nb),
                                "{fam}/{shape}/{len}: {hb:02X} vs {nb:02X} is not a \
                                 continuation-byte pair"
                            );
                        }
                        "nm_combining" => {
                            // The needle is the whole planted region plus a
                            // combining dot, so nothing may differ before the
                            // dot's own two bytes.
                            assert!(
                                mm as usize >= m - 2,
                                "{fam}/{shape}/{len}: diff at {mm}, needle is {m} B \
                                 and only the appended U+0307 should differ"
                            );
                        }
                        "nm_mid" => {
                            assert!(
                                mm >= 1 && (mm as usize) < m - 1,
                                "{fam}/{shape}/{len}: interior diff landed at {mm} of {m}"
                            );
                            assert!(anchor_passes(&c.h, &c.n, c.p), "{fam}/{shape}/{len}");
                        }
                        "nm_penult" => {
                            assert_eq!(mm as usize, m - 2, "{fam}/{shape}/{len} diff spot");
                            assert!(anchor_passes(&c.h, &c.n, c.p), "{fam}/{shape}/{len}");
                        }
                        _ => unreachable!(),
                    }
                    tested += 1;
                }
            }
        }
        // 3 families x 5 shapes x 3 lengths, minus the three structurally
        // impossible `A / nm_last_cont` cells.
        assert_eq!(tested, 42, "cells actually asserted");
        assert_eq!(
            skipped,
            vec![
                "A/nm_last_cont/64".to_string(),
                "A/nm_last_cont/65".to_string(),
                "A/nm_last_cont/96".to_string(),
            ],
            "the only non-constructible combination is the continuation-byte shape \
             on a pure-ASCII corpus"
        );
        assert_eq!(hits, 0, "a true near miss must never be answered as a hit");
    }

    /// Routing, at the seam and across the span the round swept: the dispatch
    /// below `SCAN_MAX_BYTES` runs the ring and folds nothing, the dispatch above
    /// it folds each byte exactly once and runs no ring, and neither side
    /// reaches the copying path. Read through the counters only - the crate has
    /// no counting allocator, so "and neither allocates" is pinned by the bench
    /// (`raw_p*.txt` carries `allocs=0.0000` on every (i)/(ii) cell) and stays
    /// unproven inside this crate.
    ///
    /// What "does not reach (iii)" means here, exactly: (iii) is the only arm of
    /// the dispatch that touches neither `BYTE_COMPARES` nor `SCRATCH_FOLDS`, so
    /// a non-zero signature of the right shape proves the call *entered* (i) or
    /// (ii). It cannot prove the call finished there, because the `from_utf8`
    /// `Err` arm inside `ascii_scratch_contains` falls back to the copy after
    /// folding - that arm is unreachable by construction and has no execution
    /// evidence; the answer-vs-reference assertion below covers it either way.
    #[test]
    fn the_bound_routes_the_ring_below_and_the_reused_buffer_above() {
        assert_eq!(
            SCAN_MAX_BYTES, 64,
            "the bound this round measured and deliberately did not move"
        );
        let mut cells = 0usize;
        let mut below = 0usize;
        let mut above = 0usize;
        for fam in ["A", "C", "X"] {
            for len in [48usize, 56, 60, 63, 64, 65, 66, 68, 96, 128, 160] {
                let c = build_absent(fam, len, 12345);
                assert_eq!(c.h.len(), len);
                assert!(
                    only_ascii_folds(&c.h),
                    "{fam}/{len}: the sweep is a gate-true sweep"
                );
                // The buffer bound is not in play at these lengths, so the only
                // routing question is (i) against (ii).
                assert!(c.h.len() + c.n.len() <= SCRATCH_MAX_BYTES);
                let predicted_ring = c.h.len() <= SCAN_MAX_BYTES && only_ascii_folds(&c.h);
                reset_byte_compares();
                reset_scratch_folds();
                reset_chunk_folds();
                let got = contains_ignore_case(&c.h, &c.n);
                let want = reference(&c.h, &c.n);
                assert_eq!(got, want, "{fam}/{len}: the answer, not just the route");
                assert!(!got, "{fam}/{len}: an absent needle must not be found");
                let (cmp, folds, chunk) = (byte_compares(), scratch_folds(), chunk_folds());
                assert_eq!(chunk, 0, "{fam}/{len} went past the buffer bound");
                if predicted_ring {
                    assert!(
                        cmp > 0 && folds == 0,
                        "{fam}/{len}: predicted (i) ring, saw {cmp} comparisons and \
                         {folds} folds"
                    );
                    below += 1;
                } else {
                    assert_eq!(
                        folds as usize,
                        c.h.len() + c.n.len(),
                        "{fam}/{len}: predicted (ii), folded {folds} bytes for a {} B \
                         cell with a {} B needle",
                        c.h.len(),
                        c.n.len()
                    );
                    assert_eq!(
                        cmp, 0,
                        "{fam}/{len}: above the bound the ring must not also run"
                    );
                    above += 1;
                }
                cells += 1;
            }
        }
        assert_eq!(cells, 33, "3 families x 11 lengths");
        // 48/56/60/63/64 x 3 families: the whole span below the bound, and the
        // seam itself, 64 B on (i) against 65 B on (ii).
        assert_eq!(below, 15, "cells on the ring");
        assert_eq!(above, 18, "cells on the reused buffer");
    }

    /// The seam, spelled out one byte at a time: 63/64 B answer on the ring,
    /// 65/66 B on the reused buffer, and moving the bound by one byte would
    /// change which of the two allocation-free routes answers - which is exactly
    /// the change the round declined to make, so it is the fact a later round
    /// must not break silently.
    #[test]
    fn the_seam_is_one_byte_wide_and_moves_no_answer() {
        let mut answers = Vec::new();
        for fam in ["A", "C", "X"] {
            for len in [63usize, 64, 65, 66] {
                let c = build_absent(fam, len, 777);
                reset_byte_compares();
                reset_scratch_folds();
                let got = contains_ignore_case(&c.h, &c.n);
                let on_ring = byte_compares() > 0 && scratch_folds() == 0;
                let on_buffer =
                    scratch_folds() as usize == c.h.len() + c.n.len() && byte_compares() == 0;
                assert!(on_ring || on_buffer, "{fam}/{len} answered elsewhere");
                assert_eq!(got, reference(&c.h, &c.n), "{fam}/{len}");
                assert_eq!(on_ring, len <= SCAN_MAX_BYTES, "{fam}/{len} route");
                answers.push((fam, len, on_ring, got));
            }
        }
        assert_eq!(answers.len(), 12);
        assert!(
            answers.iter().all(|(_, _, _, got)| !*got),
            "every cell here carries an absent needle"
        );
    }

    /// The §22.7-12 account: round 18 exhausted the short-string tier to
    /// full strings of length <= 3 (`the_chunked_route_agrees_with_the_copying_route_point_by_point`)
    /// because 24^4 + 24^5 haystacks do not fit in a gate. This is the same tier
    /// carried to full strings of 4 and 5 characters, over a declared 12-character
    /// gate-true sub-alphabet of that alphabet (all four UTF-8 byte widths, and
    /// every character the gate admits), with the needles *sampled* rather than
    /// exhausted. So this is a sampled sweep, not an exhaustive proof: what is
    /// exhaustive is the haystack side over these 12 characters at lengths 4 and 5
    /// (12^4 + 12^5 = 269,568 strings); what is sampled is the needle side.
    #[test]
    fn full_strings_of_four_and_five_characters_agree_with_the_copying_route() {
        // Every character is gate-true (`only_ascii_folds` on it is checked
        // below), no character changes under `to_lowercase`, and the byte widths
        // 1/2/3/4 are all present - the reason this alphabet rather than a
        // bigger one: length 5 over 24 characters is ~8 million haystacks.
        const SUB: [char; 12] = [
            'a',
            '_',
            ' ',
            '·',
            'ß',
            'µ',
            'é',
            'σ',
            'б',
            '中',
            '\u{307}',
            '\u{1F642}',
        ];
        const SEED: u64 = 0x9C19_5EED_0000_0001; // fixed: the sample is reproducible
        for c in SUB {
            let mut one = String::new();
            one.push(c);
            assert!(only_ascii_folds(&one), "{c:?} must be admitted");
            assert_eq!(one.to_lowercase(), one, "{c:?} must fold to itself");
        }
        for w in [1usize, 2, 3, 4] {
            assert!(
                SUB.iter().any(|c| c.len_utf8() == w),
                "no {w}-byte character in the sub-alphabet"
            );
        }
        let mut x = SEED;
        let mut haystacks = 0usize;
        let mut cases = 0usize;
        let mut hits = 0usize;
        let mut misses = 0usize;
        for len in [4usize, 5] {
            let total = SUB.len().pow(len as u32);
            for i in 0..total {
                let mut chars = Vec::with_capacity(len);
                let mut d = i;
                for _ in 0..len {
                    chars.push(SUB[d % SUB.len()]);
                    d /= SUB.len();
                }
                let haystack: String = chars.iter().collect();
                haystacks += 1;
                // Two sampled needles per haystack: one substring of it (so the
                // hit leg is exercised), one character of the alphabet, both
                // already folded because every character of `SUB` folds to
                // itself - the caller's contract, checked once per case.
                let mut needles: Vec<String> = Vec::with_capacity(2);
                let a = (lcg(&mut x) % len as u64) as usize;
                let k = 1 + (lcg(&mut x) % 2) as usize;
                let end = (a + k).min(len);
                needles.push(chars[a..end].iter().collect());
                needles.push(String::from(SUB[(lcg(&mut x) % SUB.len() as u64) as usize]));
                for needle in needles {
                    assert_eq!(needle, needle.to_lowercase(), "needle must be folded");
                    let want = reference(&haystack, &needle);
                    hits += want as usize;
                    misses += !want as usize;
                    for window in [4usize, 8, CHUNK_MAX_BYTES] {
                        assert_eq!(
                            ascii_chunk_contains_windowed(&haystack, &needle, window),
                            want,
                            "chunked(window={window})({haystack:?}, {needle:?})"
                        );
                    }
                    assert_eq!(
                        ascii_chunk_contains(&haystack, &needle),
                        want,
                        "chunked({haystack:?}, {needle:?})"
                    );
                    // The entry at these lengths is the ring below the bound; it
                    // is on the same reference, so the tier pins both ends.
                    assert!(haystack.len() <= SCAN_MAX_BYTES, "{haystack:?}");
                    assert_eq!(
                        contains_ignore_case(&haystack, &needle),
                        want,
                        "entry({haystack:?}, {needle:?})"
                    );
                    cases += 1;
                }
            }
        }
        assert_eq!(haystacks, 12 * 12 * 12 * 12 + 12 * 12 * 12 * 12 * 12);
        assert!(
            cases >= 500_000,
            "only {cases} sampled cells: the tier would be too thin to count"
        );
        assert!(hits > 0 && misses > 0, "hits={hits} misses={misses}");
        // Zero disagreements is the assertion above - a single one panics with
        // the case in its message. Nothing here relaxes it.
    }
}

#[cfg(test)]
mod case_lead_table_band_tests {
    use crate::fold::CASE_LEAD_BYTE;

    /// The sieve is indexed by *every* byte of a string, so the one thing it
    /// must never do is answer true for a UTF-8 continuation byte: a
    /// continuation byte says nothing about which character it belongs to, and
    /// a shape that skips by character width (round 20's candidate (a2)) turns
    /// "only a lead byte can hit" into "one pass, no per-byte lookups at the
    /// tail of a character". That is the disjointness this asserts, by name.
    ///
    /// Why `lib.rs`'s `the_gate_admits_only_characters_that_fold_to_themselves`
    /// does not cover this directly: it collects, for every code point that has
    /// a lowercase mapping, the *first* byte of its encoding
    /// (`crates/ui/src/lib.rs:838`) and then requires the table to equal that
    /// set of bytes over all 256 indices (`:864-870`). A true bit inside
    /// `0x80..=0xBF` would indeed break that equality - no character's first
    /// UTF-8 byte is ever a continuation byte - so the property is *guarded*,
    /// but only through one step of reasoning about UTF-8's encoding
    /// structure. Round 20 stage ② states the same fact without that step.
    #[test]
    fn no_continuation_byte_is_a_case_lead_byte() {
        for b in 0x80u16..=0xBF {
            assert!(
                !CASE_LEAD_BYTE[b as usize],
                "U+{b:04X} is a UTF-8 continuation byte (0x80..=0xBF) yet the \
                 lead-byte table admits it, so a width-skipping sieve could no \
                 longer treat every hit as the start of a character"
            );
        }
        // The two other bands that must stay empty: ASCII, which leg 1 of the
        // gate short-circuits before the table is ever read, and the `C0`/`C1`
        // (overlong) plus `C2` band, whose characters - `Â`..`à` - carry no
        // lowercase mapping. Round 20 measured all three as booleans, not as
        // timings: `raw_probe.txt` `true_below_0x80=false`,
        // `true_in_0x80_0xBF=false`, `true_in_0xC0_0xC2=false`.
        for b in 0x00u16..0x80 {
            assert!(
                !CASE_LEAD_BYTE[b as usize],
                "ASCII byte {b:#04X} is flagged as a case lead, but the gate \
                 returns at its first leg for any all-ASCII string"
            );
        }
        for b in 0xC0u16..=0xC2 {
            assert!(
                !CASE_LEAD_BYTE[b as usize],
                "lead byte {b:#04X} (overlong or the Latin-1 `Â`..`à` band) is \
                 flagged although no character it encodes has a lowercase mapping"
            );
        }
    }

    /// The other half of the same disjointness, phrased from the table's side:
    /// its lowest true index sits strictly above the continuation band, so the
    /// set of hits and the set of continuation bytes cannot overlap at all -
    /// which is what makes per-byte table lookup in a single pass over a
    /// well-formed `&str` equivalent to checking lead bytes only.
    #[test]
    fn the_lowest_case_lead_byte_sits_above_the_continuation_band() {
        let mut hits: Vec<u16> = Vec::new();
        for b in 0..=0xFFu16 {
            if CASE_LEAD_BYTE[b as usize] {
                hits.push(b);
            }
        }
        let first = *hits
            .first()
            .expect("the lead-byte table has no true entry at all");
        assert!(
            first > 0xBF,
            "the lowest lead byte is {first:#04X}, which lies inside the \
             continuation band 0x80..=0xBF, so a table hit no longer implies \
             'a character starts here'"
        );
        assert!(
            first >= 0xC3,
            "the lowest lead byte moved below 0xC3 ({first:#04X}); round 20's \
             enumeration gave 0xC3 (`raw_probe.txt`: `first=0xC3`)"
        );
        for b in hits {
            assert!(
                !(0x80..=0xBF).contains(&b),
                "lead byte {b:#04X} is also a continuation byte"
            );
        }
    }
}
