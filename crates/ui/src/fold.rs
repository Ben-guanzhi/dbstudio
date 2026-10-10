//! Case-insensitive substring matching, folded once per filter pass instead of
//! once per item.
//!
//! Every search box in the app re-filters a list on each keystroke. The plain
//! `item.to_lowercase().contains(&query)` allocates a lowercased copy per item
//! per keystroke; these helpers fold the query once (the caller's job) and match
//! the item in place wherever that is cheaper.
//!
//! "Wherever that is cheaper" is a length question, and the answer is a family
//! of crossovers rather than one number: where the scan loses depends on what
//! the haystack is made of. Measured against fold-then-`str::contains` (median
//! of copy over median of scan, 5,000 calls per timed region, 300 rounds, 3
//! processes; >1 means the scan wins, and 0.7-1.3 is inside the noise floor of
//! that harness): lowercase ASCII wins while it is short and crosses between
//! 72 and 96 B - 16 B 2.52, 24 B 2.67, 32 B 1.76, 48 B 1.36, 63/64 B
//! 1.25/1.13, 65 B 1.11, 72 B 1.05, 96 B 0.71, 128 B 0.58 - because a window
//! scan is O(haystack x needle) while std's search skips a long haystack on
//! the first byte; a `·`-punctuated label wins 4.05-4.98 across 16-128 B
//! (2.84-4.46 held open to 1024 B) and mixed CJK/ASCII column values 4.53-5.02
//! (3.22-4.79), so neither of those shapes crosses at all. The reason they
//! differ is the denominator: `to_lowercase` costs ~0.28-0.30 ns per byte on
//! ASCII and 9.9-11.5 ns per byte on multi-byte characters, so a long ASCII
//! haystack is the scan's worst case and a long CJK one its best. Those
//! second readings come from an instrument whose bound is held open, and on
//! this side it runs ~1.48x high: the gate plus
//! [`ascii_contains`] compiled into a second module instance cost 1.376x the
//! hand-written wrapper below 64 B and 1.480x at 65-128 B, while the product
//! entry and the wrapper agreed to 0.84-1.17x (mean 1.05) - the bias is in the
//! instance, not in the wrapper. So the ASCII numbers bound what the scan can
//! cost rather than pricing what it does cost, and what this file acts on is
//! the direction, not the crossover's exact position.
//! Those crossovers priced the ring against *a copy*, and above the bound an
//! admitted haystack is not copied any more - it goes to
//! [`ascii_scratch_contains`], which folds the bytes once into a buffer that
//! is reused per thread and then lets `str::contains` search it. So the family
//! that routes the entry now is the one between the two allocation-free scans,
//! and that family has two prices. Folding is the flat one: the same window
//! ring comparing with `eq_ignore_ascii_case`, over the same ring comparing
//! pre-folded bytes with a plain `==`, costs 1.9-2.1x, at every length and
//! every shape, because folding is ~0.3 ns per byte whoever pays it - inside a
//! comparison or up front. The structure is the stepping one: the ring handed
//! bytes that are already folded, against `str::contains` over the same, is
//! 1.70x at 64 B and under, 2.44x at 65-128 B and 4.90x at 256 B and above
//! (4 processes, 5,000 calls per timed region, 300 rounds, medians of those),
//! worst 24.7x where a shared prefix mismatches in the middle of the needle.
//! Both readings are ranges, not multiples: that instrument's control, a
//! semantically identical reorder of the same scan, spans 0.54-2.29x on one
//! cell, so only the step above 2.3 is a measured difference. Its shape
//! dependency is real and per start: counted on `'a' x 63` against
//! `'a' x 31 + '·'`, the anchorless `windows().any()` ring and the
//! first-byte-filtered ring make 32 comparisons for every one the last-byte
//! anchor makes. Past the bound a haystack pays the first price once per byte
//! and none of the second; below it a short haystack covers neither, which is
//! why the ring still answers there. What that route costs in product terms is
//! 0.158-0.436 of the entry it replaced on multi-byte corpora past the bound
//! and 0.30-0.87 on ASCII ones, every one of 96 cross-process envelopes below
//! 1, folding the needle as well as the haystack being free to the resolution
//! of the instrument (0.89-1.26 across 44 of 48 cells).
//! `SCAN_MAX_BYTES` is the midpoint of that family -
//! read it as a compromise, never as the crossover of one shape. It is no
//! longer a cost gate: both sides of it are now allocation-free, the ring below
//! and the prefolded buffer above, so what it picks is which of them answers,
//! and second what one cell may be charged. The second role is why the number
//! stays where it is: the window loop is Theta((h-m+1) x m) and h <= B holds
//! that to B^2/4. At B=64 the cap is 1,024 comparisons per cell, under
//! 4 ms over the 2,000 cells of a pass; at B=1,024 it is 262,144, about
//! 1,042 ms a pass - where the worst measured cell lands. Which cell gets
//! scanned is a decision about the haystack only, and the needle is whatever
//! the search box holds, so nothing else bounds the work. The prefolded route
//! is linear in both, so it needs no such cap; it has a bound of its own,
//! `SCRATCH_MAX_BYTES`, that caps a buffer rather than a loop, and above that
//! one an admitted haystack is not handed back to the copy either -
//! [`ascii_chunk_contains`] streams it through the same reused buffer
//! `CHUNK_MAX_BYTES` at a time, so the folded `String` per item per keystroke
//! is left for the haystacks that carry a real case mapping, where folding is
//! not a byte-wise operation and no window-shaped substitute was found that
//! both costs less and agrees with it.
//! So the ring below the bound, the reused buffer above it, the reused window
//! past that, and the copy for the haystacks none of the three may enter.
//!
//! `starts_with_ignore_case` takes the scan whenever [`only_ascii_folds`]
//! admits the haystack, with no length term of its own: the comparison is
//! anchored at byte 0, so it makes at most min(h, m) comparisons and there is
//! no crossover for it to sit near. Against fold-then-`str::starts_with` it
//! wins at every length above the bound - label 12.71 at 65 B rising to 13.65
//! at 1024 B, mixed 14.45 to 15.60, ASCII 6.23-9.03 with one weak cell at 96 B
//! (2.08) - and 6.9-15.1 below it, where the same route is already open today.
//!
//! Inside the scan, the window loop rejects on the needle's *last* byte (see
//! [`ascii_contains`]): the shape that makes it O(haystack x needle) is a shared
//! prefix with the mismatch at the end, so the obvious first-byte filter taxes
//! every start and still reads the whole window. Measured on `'a' x 63` against
//! `'a' x 31 + '·'`: 2,972 ns plain, 5,826 ns first-byte filtered, 129 ns
//! rejecting at the end. At that shape's full size the same cell costs
//! 521,212 ns on the bound-held-open instrument, about 350,000 once its ~1.48x
//! is folded out - 1,042 ms / 704 ms per 2,000 cells. That is the price the
//! length term on `contains` exists to refuse, and the anchored comparison
//! above cannot be charged it.
//! The same shape with the mismatch in the middle of the needle instead of at
//! its tail is the anchor's blind spot, and there the ring has no answer at
//! all: 7.2-24.7x what `str::contains` charges for the same cell at m = 4-8,
//! 2.0-4.1x at m = 16-32, every one of those measured on bytes folded before
//! the search began, so it is structure and not folding being priced. That is
//! evidence for the anchor rather than against it - one comparison per start
//! is what keeps the middle-mismatch cell from degenerating further - and it
//! is one more reason the route above the bound hands the searching to
//! `str::contains`, whose skip does not depend on where the mismatch sits.
//!
//! "Which strings may be scanned" is a second, orthogonal question, answered by
//! [`only_ascii_folds`]: any haystack whose only case folding is ASCII's, not
//! just an ASCII one - which includes CJK, Greek and Cyrillic *lowercase*, and
//! excludes anything with a lowercase mapping.

/// Below this haystack length the byte window scan answers
/// [`contains_ignore_case`]; above it an admitted haystack goes to
/// [`ascii_scratch_contains`] and only a case-mapped one reaches the copying
/// path. The value is the midpoint of a family of crossovers measured against
/// *a copy* - lowercase ASCII crossing only between 72 and 96 B, neither
/// multi-byte shape crossing inside 1024 B - so read it as a compromise over
/// that family rather than the reading of any single shape, and see the module
/// comment for the numbers. It is not what makes a haystack cheap any
/// more: both routes past it are allocation-free, and what the length term
/// still buys is its second identity, the cap on the comparisons one cell may
/// make (the window loop is Theta((h-m+1) x m), so h <= B holds it to B^2/4).
/// That cap is why the bound stays at 64 while the scanning route grows past
/// it, and it gates `contains_ignore_case` alone.
///
/// Re-measured against the *other allocation-free route* rather than against a
/// copy, and left exactly where it is. Pure ASCII is the one family that crosses
/// inside the measured span: the prefold/ring ratio is 1.270 at 63 B and 0.661 at
/// 64 B, 0.855 at 65 B, so the crossing sits between 63 and 65 B (least-squares
/// interpolation of the same 48-160 B sweep puts it at 61.4 B). Gate-true
/// multi-byte has no crossing at all in 48-160 B - twenty lengths of
/// CJK-dominated cells mediate 1.204-2.059 and of mixed 1/2/3-byte cells
/// 1.091-2.289, every one above 1, so the ring is the cheaper route on both
/// sides of this bound for them. Hence 64 is the crossover of the ASCII family
/// only, which is what "a family of crossovers, never the crossover of one shape"
/// means, and it is why neither direction of moving it pays: raising it to 96
/// hands ASCII 65-96 B back to the ring, where those cells measure 0.546-0.860
/// of the buffer's price, i.e. 1.16-1.83x more; lowering it to 48 hands 56-64 B
/// to the buffer and costs the multi-byte cells 1.557-1.981x uniformly and the
/// ASCII cells 1.091-1.270x at 56-63 B (at 64 B itself ASCII lands at 0.661, so
/// even that direction is not uniformly a saving for ASCII). The ratios are
/// median-of-3 of per-process readings on the round's instrument, whose absolute
/// nanoseconds run ~1.4x high for a second-instance route, so they are direction
/// and not price; no cell reached the "stable across three processes" bar the
/// harness sets for calling a multiple a multiple. None of this makes the bound a
/// cost gate: both sides still allocate nothing, and it still only picks which of
/// the two answers, plus the cap above.
pub(crate) const SCAN_MAX_BYTES: usize = 64;

/// The ceiling on the buffer [`ascii_scratch_contains`] folds into, counted in
/// the bytes it has to hold, haystack plus needle. Past it that route declines:
/// for a haystack [`only_ascii_folds`] admits [`ascii_chunk_contains`] answers
/// from a window of [`CHUNK_MAX_BYTES`], and only a case-mapped one reaches the
/// copying path. Either way the purpose of a bound here is unchanged - a
/// paste-sized cell must not make one filter pass grow a buffer of arbitrary
/// size - and the bound is what keeps it true now that the buffer is reused
/// past it too: the route below holds the largest cell a thread actually
/// prefolded, the route above never holds more than one window. Deliberately a
/// second constant - `SCAN_MAX_BYTES` caps the comparisons one cell may make,
/// this one caps a buffer, and the two numbers have nothing to do with each
/// other. 4096 leaves four times the longest corpus the route was measured on
/// (1024 B); the routes past it are pinned for the answer they give and for
/// touching nothing beyond a window, never for their cost.
pub(crate) const SCRATCH_MAX_BYTES: usize = 4096;

/// The window [`ascii_chunk_contains`] folds into, in bytes, above the bound
/// [`ascii_scratch_contains`] is willing to hold whole. A third constant with a
/// third meaning, deliberately not an alias of either above: `SCAN_MAX_BYTES`
/// caps the comparisons one cell may make, `SCRATCH_MAX_BYTES` caps the buffer
/// one cell may fill, this one caps how much of a cell the streaming route
/// materialises at a time. It happens to equal `SCRATCH_MAX_BYTES` - that is
/// where the route was measured (a 64 KiB window reads the same on every cell
/// the 4 KiB one does, and costs 64 KiB of residency per thread to say it),
/// while the residency claim below is that a cell of any length costs this
/// many bytes and nothing else.
pub(crate) const CHUNK_MAX_BYTES: usize = 4096;

/// The UTF-8 lead bytes that can begin a code point with a lowercase mapping.
/// Exactly 21 of them - `C3..C9`, `CD..D5`, `E1`, `E2`, `EA`, `EF`, `F0` -
/// derived by brute-forcing `char::to_lowercase` over `U+0080..U+0x110000`
/// (`the_gate_admits_only_characters_that_fold_to_themselves` re-derives the
/// set from this predicate and asserts none is missing). A string whose bytes
/// hit none of them therefore contains no cased character at all and needs no
/// decoding.
///
/// The count was re-derived a second time, independently of that test, by walking
/// every code point `0x80..0x10FFFF` and taking the first byte of each character
/// `has_lowercase_mapping` accepts: 21 leads, and zero disagreements with this
/// table (`target/bench_scanbound/probe_lead.txt`, the round that measured
/// `SCAN_MAX_BYTES` against the other allocation-free route; its first draft
/// derived the ranges from the lead bytes instead of the code points and reported
/// 17 with six false mismatches - kept on disk as
/// `probe_lead_draft_bandbug.txt`). That walk covers the 4-byte band too - lead
/// `F0` reaches `U+10400` - which the round's *timed* corpora did not: its
/// character pools held no 4-byte character at all, so no price for 4-byte
/// content was measured.
///
/// This is a *prefilter*, not the gate. It is indexed by every byte of the
/// string, not just the first one, which is still sound: a cased character puts
/// its own lead byte somewhere in it. Bytes `0x80..0xBF` are all false - a
/// continuation byte says nothing about which character it belongs to, so it may
/// not be used to admit or reject anything.
///
/// `pub(crate)` so the exhaustive test can check the table against the property
/// it stands for instead of trusting the literals.
pub(crate) const CASE_LEAD_BYTE: [bool; 256] = {
    const fn may_start_a_cased_char(b: u8) -> bool {
        matches!(b, 0xC3..=0xC9 | 0xCD..=0xD5 | 0xE1 | 0xE2 | 0xEA | 0xEF | 0xF0)
    }
    let mut table = [false; 256];
    let mut i = 0usize;
    while i < 256 {
        table[i] = may_start_a_cased_char(i as u8);
        i += 1;
    }
    table
};

/// The 31 code points of `General_Category=Lt` (titlecase letters). They are
/// spelled out because `char::is_titlecase` is unstable, and they matter: all
/// 31 change under `to_lowercase` (`ǅ` -> `ǆ`), while none of them is
/// `is_uppercase`, so the category test alone would let them into the byte scan.
///
/// `pub(crate)` for the same reason as the table above: the exhaustive test
/// checks the list against Unicode rather than trusting the literals.
pub(crate) const fn is_titlecase(c: char) -> bool {
    matches!(
        c,
        '\u{1c5}'
            | '\u{1c8}'
            | '\u{1cb}'
            | '\u{1f2}'
            | '\u{1f88}'..='\u{1f8f}'
            | '\u{1f98}'..='\u{1f9f}'
            | '\u{1fa8}'..='\u{1faf}'
            | '\u{1fbc}'
            | '\u{1fcc}'
            | '\u{1ffc}'
    )
}

/// Does `c` have a lowercase mapping? Yes if it is uppercase or titlecase -
/// the exhaustive test asserts that this is a *sufficient* reason to reject,
/// never a necessary one.
#[inline]
pub(crate) fn has_lowercase_mapping(c: char) -> bool {
    c.is_uppercase() || is_titlecase(c)
}

/// Is the only case folding in `s` the ASCII one?
///
/// The byte scan compares with `eq_ignore_ascii_case`, so it is exact for `s`
/// only if every character of `s` survives `to_lowercase` unchanged. That is the
/// contract, and it is a *sufficient* condition: the gate may reject characters
/// that would have been safe (it does: 549 of them, `𝐀`, `ℂ`, `ϒ` and the rest
/// of the letterlike/mathematical uppercase), it may never admit one that was
/// not. `the_gate_admits_only_characters_that_fold_to_themselves` brute-forces
/// that over all of Unicode instead of trusting it.
///
/// Two facts carry it, both cheap to test:
///   * every code point with a lowercase mapping is uppercase or one of the 31
///     titlecase letters ([`has_lowercase_mapping`]). This is a *different*
///     claim from the one §15's version of this function rested on ("case-mapped
///     code points are alphabetic", true but useless here: it made CJK - `Lo`,
///     hence alphabetic - pay for a copy even though `中` folds to itself);
///   * such a character's UTF-8 sequence starts with one of the 21 bytes in
///     [`CASE_LEAD_BYTE`], so a string that hits none of them is admitted
///     without decoding anything, which is the common case for symbol-punctuated
///     labels (`Table · heap`, and every generated label in the app uses
///     `U+00B7`).
///
/// So `中文表格`, `订单表_2024`, `αβγδεζηθικλμ`, `한`, `straße`, `µ` now qualify -
/// their characters fold to themselves - while `ΣΣΣ`, `İstanbul`, `ǅ`, `Ⅻ`, `Ａ`,
/// `МОСКВА`, `Пользователь`, `ΑΒΓαβγ` do not.
///
/// # What each leg buys, and what no rewrite was worth (round 20, stage ①)
///
/// Measured, not asserted: the three legs and five rewrites of this predicate
/// were timed over 48 corpus cells x three separate processes in
/// `target/bench_gate/` (`raw_p1.txt`..`raw_p3.txt`; bands in `RESULTS.md`
/// 4-B / 6.5-B / 6.6-B, and re-derived for these numbers in
/// `impl_recompute.txt` + `impl_bands.txt` + `impl_anchor_check.txt`).
/// Absolute nanoseconds are comparable only inside that one build
/// (`bench.exe` md5 `8433f263af410623f40b124f1ce504e6`); across rounds quote
/// ratios only. Every ratio below is the median of the three per-process
/// ratios, then the band over that family's cells. The floor of the instrument
/// is the same-code control pair `CTRLG/GATE`: 48 cells, mean 0.999, per-cell
/// band 0.812..1.313, per-process envelope 0.667..1.524. A number counts as a
/// multiple only when all three processes sit on one side of 2.3x; among the
/// 240 candidate cells (48 x 5) not one reached 2.3x on the faster side, and
/// 38 sat at or above 2.3x on the slower side - so the round kept this
/// predicate exactly as it is.
///
/// * leg 1 (`s.is_ascii()`) is the only short-circuit ASCII content has. On
///   those cells leg 1 alone costs 0.800..1.000 of the whole gate, while either
///   later leg run alone costs 4.000..23.167 (the sieve) and 4.250..29.167 (the
///   per-character predicate), and deleting leg 1 costs 5.000..26.857;
/// * leg 2 (the byte sieve) is the only short-circuit content that hits no lead
///   byte has: it *is* 1.062..1.448 / 0.933..1.438 / 1.000..1.531 of the gate on
///   CJK / symbol / mixed cells, where leg 3 alone would cost 3.467..6.241 /
///   2.467..3.781 / 3.214..4.071. Deleting it costs 3.333..5.071 / 2.278..3.844
///   / 2.929..4.196 on the same three families;
/// * leg 3 (`chars().all(..)`) is the whole gate once the sieve hits:
///   0.851..1.227 (Korean, EA segment), 1.261..1.422 (Latin-1 accents),
///   0.774..0.836 (one cased letter mid-string), 1.500..1.568 (`straße`);
///   the sieve alone is 0.003..0.095 there, 0.231..0.371 on `M` (mid-string).
///
/// Share of a whole call at the entry point (`gate_share = 1 - route ns /
/// entry ns`, the route still chosen by length - pricing the gate is not a
/// statement about which route runs): -0.250..0.245 pure ASCII, 0.073..0.463
/// CJK, 0.240..0.442 mixed, 0.519..0.677 mixed-with-Korean, 0.605..0.744
/// lead-byte-hitting-and-admitted, 0.896..0.935 rejected at the first
/// character. Only those six families were measured at the entry point; the
/// other nine have gate-level bands and nothing else.
///
/// Nothing served both halves of the corpus, so nothing was adopted. The
/// skip-by-width form (candidate (a2)), which reads this table by lead byte
/// only, was the single candidate past 1.5x and it flips sign: 0.384..0.548 /
/// 0.486..0.689 / 0.492..0.558 of the gate on the one-cased-letter, Korean and
/// mixed-Korean families, against 2.659..3.489 / 2.407..3.167 / 2.078..2.558 on
/// Latin-1, `straße` and the symbol-band families; at the entry level its best
/// cell was 0.635. The bitmap sieve cost 1.400..2.517 where the sieve misses
/// (0.75..2.22 elsewhere), and swapping the two byte legs cost 3.200..8.871
/// on CJK / symbol / mixed. Its correctness prerequisites are asserted directly rather
/// inherited from a comment: see `no_continuation_byte_is_a_case_lead_byte` and
/// `the_lowest_case_lead_byte_sits_above_the_continuation_band` at the end of
/// this file.
pub(crate) fn only_ascii_folds(s: &str) -> bool {
    if s.is_ascii() {
        return true;
    }
    if !s.bytes().any(|b| CASE_LEAD_BYTE[b as usize]) {
        return true;
    }
    s.chars().all(|c| c.is_ascii() || !has_lowercase_mapping(c))
}

/// Does folding `s` depend on position? Greek capital sigma is the one
/// conditional lowercase mapping in Unicode (`ς` at the end of a word, `σ`
/// elsewhere); everything else, including the two-code-point `U+0130` mapping,
/// folds per character exactly. Allocation-free.
fn needs_contextual_fold(s: &str) -> bool {
    s.contains('\u{3a3}')
}

/// `s.to_lowercase()` into a char buffer, in one allocation when the string is
/// ASCII or folds context-free.
///
/// The plain `s.to_lowercase().chars().collect()` path pays for a `String` copy
/// *and* a `Vec` that grows by doubling, which is three to twenty-odd
/// allocations per cell on a path that visits every cell on every keystroke.
/// Measured: 40,000 -> 10,000 allocations per 10,000 ASCII cells, 218,890 ->
/// 10,001 per 10,000 CJK cells.
pub(crate) fn fold_into_chars(s: &str) -> Vec<char> {
    if s.is_ascii() {
        let mut out = Vec::with_capacity(s.len());
        out.extend(s.bytes().map(|b| b.to_ascii_lowercase() as char));
        return out;
    }
    if needs_contextual_fold(s) {
        return s.to_lowercase().chars().collect();
    }
    // `flat_map(char::to_lowercase)` is exactly `s.to_lowercase().chars()`
    // whenever no mapping depends on position; `char::to_lowercase`'s own
    // buffer keeps a multi-code-point mapping intact.
    let mut out = Vec::with_capacity(s.len());
    out.extend(s.chars().flat_map(char::to_lowercase));
    out
}

/// Is `needle` a substring of `haystack`, ignoring case?
///
/// `needle` must already be case-folded (callers fold the query once per filter
/// pass, never per cell). Under that contract this is equivalent to
/// `haystack.to_lowercase().contains(needle)` for every input, at every length
/// and on every route here. The contract is load-bearing for the equivalence,
/// not for the answer: hand it an unfolded needle and the answer used to be a
/// function of `SCAN_MAX_BYTES`, because the window ring ASCII-folds the
/// needle it is handed and the copying path leaves it alone. Both scans here
/// now ASCII-fold it, so inside and outside the bound agree. The change is
/// monotone: on the ring nothing moves, in the reused buffer an unfolded
/// needle can only gain matches where ASCII case was the difference, and past
/// `SCRATCH_MAX_BYTES`, or in a haystack with a real lowercase mapping in it,
/// the copying path answers exactly as it answered before. No haystack loses
/// an answer; nothing here claims more than that.
///
/// Haystacks whose only case folding is ASCII's take an allocation-free scan:
/// the byte window ring up to `SCAN_MAX_BYTES`, and above it
/// [`ascii_scratch_contains`], which folds the bytes once into a reused buffer
/// and lets `str::contains` do the searching - and past what that buffer may
/// hold, [`ascii_chunk_contains`], which folds the same bytes through the same
/// buffer in windows and still lets `str::contains` do the searching. Only a
/// haystack with a real lowercase mapping in it keeps `to_lowercase`, because
/// for it folding is not a byte-wise operation. The two prices behind that split, both measured: the
/// fold inside a comparison is 1.9-2.1x the same ring comparing pre-folded
/// bytes and is flat in length, while the ring as a search is 1.70x std's at
/// 64 B and under, 2.44x at 65-128 B, 4.90x at 256 B and up. Past the bound a
/// haystack pays the first once per byte and none of the second, which is what
/// makes the buffer worth folding into; below it a short haystack cannot cover
/// either price, and the ring is the cheaper of the two allocation-free routes
/// - see the module comment for the crossover it still sits on.
pub fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    // `len()` is O(1) and both gates read it, so the length decides which scan
    // answers; what makes a scan exact is a property of the *haystack*, not of
    // the needle: see [`only_ascii_folds`] for why, and the byte-alignment
    // argument in `non_ascii_needles_are_safe_for_the_byte_scan` below. The two
    // questions are now asked in one place - the verdict is computed once here
    // and carried down, because a haystack long enough to need it is long enough
    // for the probe to cost: asking it twice, once per route, showed up on the
    // paired acceptance run as the pure-ASCII over-cap cells landing at
    // 1.089-1.573x of the copy they replaced (`verify_prehoist_p*.txt`) against
    // 0.983-1.432x with the verdict carried down (`verify_p*.txt`). A smaller
    // price, not a win: that leg buys those cells residency, not speed; the gain
    // is on cells whose characters have no lowercase mapping. Nothing about the
    // routes moved: the same three arms, the same two predicates, the same
    // answer.
    let admitted = only_ascii_folds(haystack);
    if haystack.len() <= SCAN_MAX_BYTES && admitted {
        return ascii_contains(haystack.as_bytes(), needle.as_bytes());
    }
    if haystack.len() > SCAN_MAX_BYTES && admitted {
        return ascii_scratch_contains(haystack, needle, admitted);
    }
    haystack.to_lowercase().contains(needle)
}

/// Does `haystack` start with `needle`, ignoring case? Equivalent to
/// `haystack.to_lowercase().starts_with(needle)`.
///
/// `needle` must already be case-folded, exactly as in
/// [`contains_ignore_case`]: that contract is what makes the byte scan below
/// equivalent for a haystack longer than `SCAN_MAX_BYTES`, which now takes it.
///
/// Taking the scan is decided by [`only_ascii_folds`] alone. The comparison is
/// anchored at byte 0 and makes at most `min(h, m)` checks, so no length term
/// prices it and it has no crossover of its own.
pub fn starts_with_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    // The same haystack-only judgement as `contains_ignore_case`, minus its
    // length term: that term caps the window loop, and an anchored comparison
    // needs no cap and no alignment argument either (the one at the `contains`
    // gate stands on its own).
    if only_ascii_folds(haystack) {
        let (h, n) = (haystack.as_bytes(), needle.as_bytes());
        return h.len() >= n.len() && h.iter().zip(n).all(|(hc, nc)| hc.eq_ignore_ascii_case(nc));
    }
    haystack.to_lowercase().starts_with(needle)
}

/// Is `needle` a substring of `haystack`, comparing bytes with
/// `eq_ignore_ascii_case`? Exact whenever `haystack` has no non-ASCII folding,
/// which is what [`only_ascii_folds`] guarantees at the gate.
///
/// `pub(crate)` for one reason: the path guard in `lib.rs`
/// (`the_scan_rejects_a_near_miss_in_one_comparison_per_start`) calls it
/// directly and counts the comparisons it makes. Nothing outside this crate can
/// reach it, and the counter it reads is `#[cfg(test)]` only - and it counts
/// `byte_eq` alone, one route: the prefolded scan that answers above the bound
/// never calls here, so no reading of this counter observes it, and the counter
/// that does is `scratch_folds`.
///
/// The window loop is anchored on the needle's LAST byte, and that is a measured
/// choice rather than a reflex. The adversarial shape here is "half a needle
/// shared as a prefix, mismatch at the end" - `'a' x 63` against `'a' x 31 + '·'`,
/// and the same form in CJK (`订` against `订…格`), Korean (`한` against `한…글`) and
/// Greek (`α` against `α…β`). A first-byte filter is blind to it: every start
/// matches on the leading byte, so it pays one extra comparison per start *and*
/// keeps the whole quadratic inner loop - 1.7-2.3x **slower** than the plain
/// nested loop it was meant to replace, measured. Rejecting on the last byte
/// throws out each start in one comparison (that cell: 2,972 ns -> 129 ns).
/// It costs nothing when the mismatch is at the front instead, because the inner
/// loop already leaves after one comparison there.
///
/// The anchor was re-checked against both alternatives on the guard's own cell
/// rather than taken on trust: the anchorless `windows().any()` ring and the
/// first-byte-filtered ring make 32-33 comparisons per start there, 992-1,056
/// in all against the budget of 96 this crate's guard allows, where this loop
/// makes one. That is §15's and §17's degeneration reproduced independently, on
/// an instrument that had not measured it before.
pub(crate) fn ascii_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    // Unreachable from the gate (`contains_ignore_case` answers an empty needle
    // first), but the last-byte index below must not be the one place in here
    // that panics on it.
    if needle.is_empty() {
        return true;
    }
    if needle.len() == 1 {
        let n = needle[0];
        return haystack.iter().any(|hc| byte_eq(*hc, n));
    }
    let last = needle[needle.len() - 1];
    (0..=haystack.len() - needle.len()).any(|start| {
        // One comparison at the window's right edge, then the window itself.
        byte_eq(haystack[start + needle.len() - 1], last)
            && haystack[start..start + needle.len() - 1]
                .iter()
                .zip(needle)
                .all(|(hc, nc)| byte_eq(*hc, *nc))
    })
}

/// Is `needle` a substring of `haystack`, folding both with
/// `to_ascii_lowercase` into one reused buffer first and letting
/// `str::contains` do the searching? The route [`contains_ignore_case`] takes
/// above `SCAN_MAX_BYTES`, and exact on the same condition the ring is:
/// [`only_ascii_folds`] must hold for `haystack`, which is what makes ASCII's
/// fold the only fold that is needed.
///
/// This is where the structure price of the ring is cancelled without paying
/// for a copy: the fold becomes one linear pass over the haystack instead of
/// one comparison's worth per window start, and the search is std's, which
/// skips on the first byte. Measured against the entry it replaced, past the
/// bound: 0.158-0.436 of it on multi-byte corpora and 0.30-0.87 on ASCII ones,
/// every one of 96 cross-process envelopes below 1, in four batches of 5,000
/// calls x 300 rounds. Folding the needle too, which is what makes an unfolded
/// needle answer the same way here and inside the bound, costs the length of
/// the needle and is inside the noise of that instrument (0.89-1.26 of leaving
/// it out, across 44 of the 48 cells past the bound).
///
/// What that premium is *made of* was measured on the two allocation-free routes
/// against each other, over 48-160 B: on gate-true multi-byte corpora the per-byte
/// slopes of this route and of the ring are the same price (least squares over that
/// sweep: 0.9082 against 1.0232 ns/B on the CJK-dominated family, 0.9091 against
/// 0.9067 on the mixed one), and the whole difference sits in the intercepts - 45.3
/// and 50.0 ns here against -4.3 and +4.3 for the ring. So on a multi-byte haystack
/// this route costs roughly 45-50 ns *per call* more than the ring, not more per
/// byte: handing the cell to the buffer buys a flat entry into `str::contains`
/// (taking the thread-local, the `clear`, the write, the call) and then pays the
/// same linear price the ring would have. That is a property of the route, not an
/// indictment of the bound: the bound decides *which* flat price a cell pays, and
/// above it this route's flat term is what buys std's skip, whose absence is what
/// the ring's `Theta((h-m+1) x m)` loop is charged. Read it as support for routing
/// on length rather than as a cost gate having gone wrong. The ASCII family is the
/// exception in the other direction - there this route's slope collapses to 0.0158
/// ns/B against the ring's 0.9894, because folding ASCII bytes is nearly free,
/// which is why 64 sits on a real crossover for that family alone.
///
/// Reallocating the buffer is what would make this a copy again, so it is one
/// thread-local `Vec<u8>` that is cleared, never shrunk: `reserve` grows on the
/// coldest call a thread makes and on no later one, which is the structural
/// half of the measurement above - the counting allocator that priced it was
/// left out of this crate, so the claim is argued here and not asserted by a
/// test. Nor is the buffer re-entered: nothing between taking it and returning
/// calls back into this module, so the borrow cannot be double-mutated.
///
/// `str::from_utf8` is the price of not writing `unsafe`, and it cannot fail
/// here: in valid UTF-8 a byte below 0x80 is always an ASCII character on its
/// own, while a continuation byte (0x80..=0xBF) and a lead byte (0xC0..) are
/// both outside what `to_ascii_lowercase` touches, so no sequence is broken and
/// no spurious one introduced. The `Err` arm is therefore unreachable by
/// construction, kept because a wrong-answer-or-panic choice on a branch nobody
/// measured is not worth taking: no test corpus is malformed UTF-8, and the arm
/// is unreached by the tests that exist.
///
/// Past `SCRATCH_MAX_BYTES` the buffer is not filled with the whole haystack
/// any more. For a haystack [`only_ascii_folds`] admits, [`ascii_chunk_contains`]
/// answers from a window - it is the same reused buffer, so the residency claim
/// above is replaced by a smaller one: the buffer never reaches past one window
/// plus a needle. For a haystack the gate refuses the copying path gives the
/// answer, because for it folding is not a byte-wise operation and there is no
/// window-shaped substitute (measured: the two candidate shapes for that class
/// are 1.06-1.50x *slower* than the copy and one of them does not even agree
/// with it).
///
/// `admitted` is the caller's [`only_ascii_folds`] verdict for `haystack`,
/// counted once at the dispatch in [`contains_ignore_case`] and carried down:
/// the entry reaches this route only with it true, and the route below the
/// buffer bound could take the length question alone because its buffer is sized
/// by the answer, while the one past it is chosen *by* the verdict.
fn ascii_scratch_contains(haystack: &str, needle: &str, admitted: bool) -> bool {
    if haystack.len() + needle.len() > SCRATCH_MAX_BYTES {
        if !admitted {
            return haystack.to_lowercase().contains(needle);
        }
        return ascii_chunk_contains(haystack, needle);
    }
    SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.reserve(haystack.len() + needle.len());
        buf.extend(haystack.bytes().map(scratch_fold_byte));
        let split = buf.len();
        buf.extend(needle.bytes().map(scratch_fold_byte));
        match (
            std::str::from_utf8(&buf[..split]),
            std::str::from_utf8(&buf[split..]),
        ) {
            (Ok(folded), Ok(folded_needle)) => folded.contains(folded_needle),
            _ => haystack.to_lowercase().contains(needle),
        }
    })
}

/// `p`, moved down to the nearest character boundary at or below it.
///
/// `str::floor_char_boundary` is unstable, so this is the portable form of it:
/// at most three steps back, because no UTF-8 sequence is longer than four
/// bytes and the byte before a continuation byte is a boundary.
#[inline]
fn floor_char_boundary(haystack: &str, p: usize) -> usize {
    let mut p = p.min(haystack.len());
    while p > 0 && !haystack.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Is `needle` a substring of `haystack`, folding `haystack` into a bounded
/// window of [`CHUNK_MAX_BYTES`] bytes at a time - into the same reused buffer
/// [`ascii_scratch_contains`] uses - and letting `str::contains` search each
/// window? The route [`contains_ignore_case`] takes above `SCRATCH_MAX_BYTES`
/// for a haystack [`only_ascii_folds`] admits.
///
/// Why it gives the copying path's answer, in four steps:
///   * ① the gate is the precondition. `only_ascii_folds(haystack)` means every
///     character of `haystack` survives `to_lowercase` unchanged, so Unicode's
///     fold of `haystack` is byte-for-byte `to_ascii_lowercase` of it; the whole
///     string copy and this window fold therefore produce the same string. That
///     is the same fact [`ascii_scratch_contains`] stands on, one window at a
///     time.
///   * ② every window is a legal `&str`. Both edges are [`floor_char_boundary`]
///     results, so `haystack[from..end]` is a whole number of characters; an
///     ASCII fold touches only bytes below 0x80, and in valid UTF-8 such a byte
///     is always an ASCII character alone, so the folded window is valid UTF-8
///     too. This is what makes `from_utf8` here unable to fail - and it is the
///     difference between this route and a byte-aligned one, which cuts
///     three-byte characters apart on CJK text, fails `from_utf8` on 99.7% of
///     its windows there and quietly falls back to copying the whole cell.
///   * ③ consecutive windows overlap by `needle.len() - 1` bytes (the first
///     window has no overlap). A match at position `p` of length `m` ends no
///     later than the last window's end, so some window's end is the first one
///     at or past `p + m`; that window starts at most `p + m - 1`, and stepping
///     back `m - 1` bytes from it, floored to a boundary, reaches at or below
///     `p`. So the match lies whole inside that window and straddling an edge
///     cannot hide it.
///   * ④ ①-③ together make this `haystack.to_lowercase().contains(needle)`
///     byte for byte, at every length and for every needle, with no allocation
///     per call. The legs behind that claim, with their provenance stated:
///     4,601,808 pairs of the short-full-alphabet grids and 107,401 boundary
///     placements of a real corpus needle at window sizes 4 to 4096 and 26,818
///     fuzz cases whose needle is longer than the window on purpose are ①段's
///     paired runs, recorded in `target/bench_bigcell/RESULTS.md`; they are the
///     selection evidence, not this route's own re-measurement. The grids carry
///     zero disagreements with the copying path; the 107,401 placements are all
///     positive cases, so what that leg exhausts is a missed or clipped match,
///     not a spurious one; and the candidate arms ran on the `only_ascii_folds`
///     subset of the grids, never on the whole of them. This route's own legs are
///     the five tests in `chunked_route_tests` below.
///
/// Residency is the point of the window: one thread holds at most a window plus
/// a needle (the same [`SCRATCH`] buffer the cell-sized route uses, never a
/// second one), whatever the cell's length. Measured against the copy it replaces
/// on this machine, paired, needle absent so no leg stops early, median of three
/// processes (`target/bench_bigcell/verify_p{1,2,3}.txt`): a cell of characters
/// with no lowercase mapping lands at 0.206-0.238 of the copy (4.2-4.9x), at
/// 1 MB 0.231 (4.3x); a pure-ASCII cell lands at 0.983-1.432, inside to slightly
/// above the same instrument's self-noise band, and is reported as no gain; a
/// haystack the gate refuses stays on the copy at 0.993-1.249, allocations
/// unchanged, which is the rule working and not a measurement of anything.
///
/// The `Err` arm of `from_utf8` is unreachable by construction, exactly as in
/// [`ascii_scratch_contains`]: ② says every window is valid UTF-8 before the
/// fold and the fold cannot break one. It is kept because choosing between a
/// wrong answer and a panic on a branch nobody can reach is not worth taking.
fn ascii_chunk_contains(haystack: &str, needle: &str) -> bool {
    ascii_chunk_contains_windowed(haystack, needle, CHUNK_MAX_BYTES)
}

/// The body of [`ascii_chunk_contains`] with the window as a parameter, so the
/// equivalence tests can force it below `CHUNK_MAX_BYTES` and put a match on
/// every edge shape without building a corpus the size of the real window. The
/// product only ever calls this with `CHUNK_MAX_BYTES`.
fn ascii_chunk_contains_windowed(haystack: &str, needle: &str, window: usize) -> bool {
    // A window shorter than the longest UTF-8 sequence could floor to the same
    // boundary it started from and never advance. `CHUNK_MAX_BYTES` is 4096, so
    // the clamp is a total-function guard for the test-injected windows and
    // folds away on the product path.
    let window = window.max(4);
    let m = needle.len();
    // Unreachable from the gate, which answers an empty needle first, and the
    // overlap arithmetic below is the one place in here that would panic on it.
    if m == 0 {
        return true;
    }
    if m > haystack.len() {
        return false;
    }
    SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.reserve(window + m + 8);
        let mut start = 0usize;
        loop {
            let end = if start + window >= haystack.len() {
                haystack.len()
            } else {
                floor_char_boundary(haystack, start + window)
            };
            let from = if start == 0 {
                0
            } else {
                floor_char_boundary(haystack, start.saturating_sub(m - 1))
            };
            buf.clear();
            buf.extend(haystack[from..end].bytes().map(chunk_fold_byte));
            match std::str::from_utf8(&buf[..]) {
                Ok(folded) => {
                    if folded.contains(needle) {
                        return true;
                    }
                }
                Err(_) => return haystack.to_lowercase().contains(needle),
            }
            if end == haystack.len() {
                break;
            }
            start = end;
        }
        false
    })
}

/// `b.to_ascii_lowercase()` plus, in a test build only, one increment of the
/// chunked route's fold counter. The sibling of [`scratch_fold_byte`], and the
/// reason it is a sibling and not a reuse of it: `scratch_folds` is asserted to
/// read zero above `SCRATCH_MAX_BYTES` by
/// `the_scratch_route_folds_once_per_byte_and_falls_back_above_the_cap`, where
/// the whole-corpus buffer route is what it counts. Folding the window through
/// the same counter would have moved that guard's meaning; keeping the two
/// apart keeps both honest. The `#[cfg(test)]` line compiles away in the
/// product, so the fold pays nothing for being observable.
#[inline(always)]
fn chunk_fold_byte(b: u8) -> u8 {
    #[cfg(test)]
    CHUNK_FOLDS.with(|cell| cell.set(cell.get() + 1));
    b.to_ascii_lowercase()
}

#[cfg(test)]
thread_local! {
    /// Bytes folded by `ascii_chunk_contains` on this thread, for
    /// `a_refused_haystack_never_reaches_the_chunked_route` and for the two
    /// cases of `the_final_sigma_and_dotted_capital_traps_stay_on_the_copying_route`.
    /// Thread-local for the same reason as `SCRATCH_FOLDS` below: `cargo test`
    /// runs tests on several threads at once.
    static CHUNK_FOLDS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Zero the chunked route's fold counter. Test builds only.
#[cfg(test)]
pub(crate) fn reset_chunk_folds() {
    CHUNK_FOLDS.with(|cell| cell.set(0));
}

/// Bytes folded by `ascii_chunk_contains` on this thread since the last reset.
/// Test builds only.
#[cfg(test)]
pub(crate) fn chunk_folds() -> u32 {
    CHUNK_FOLDS.with(|cell| cell.get())
}

/// The capacity of the shared scratch buffer on this thread. Test builds only:
/// it is how `the_chunked_route_keeps_the_shared_buffer_bounded` reads residency
/// without a counting allocator in this crate.
#[cfg(test)]
pub(crate) fn scratch_capacity() -> usize {
    SCRATCH.with(|cell| cell.borrow().capacity())
}

/// `b.to_ascii_lowercase()` plus, in a test build only, one increment of the
/// prefolded route's fold counter: the sibling of [`byte_eq`] for the route
/// above the bound, which never calls through it. The `#[cfg(test)]` line
/// compiles away in the product, so the fold pays nothing for being observable.
#[inline(always)]
fn scratch_fold_byte(b: u8) -> u8 {
    #[cfg(test)]
    SCRATCH_FOLDS.with(|cell| cell.set(cell.get() + 1));
    b.to_ascii_lowercase()
}

thread_local! {
    /// The buffer [`ascii_scratch_contains`] folds into: one allocation per
    /// thread, cleared per call and sized by the largest cell that thread
    /// actually prefolded. Holds the folded haystack, then the folded needle.
    static SCRATCH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
thread_local! {
    /// Bytes folded by `ascii_scratch_contains` on this thread, for
    /// `the_scratch_route_folds_once_per_byte_and_falls_back_above_the_cap`.
    /// Thread-local for the same reason as `BYTE_COMPARES` below, and it counts
    /// the fold alone: the route above the bound is what it observes, and
    /// nothing on the ring does.
    static SCRATCH_FOLDS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Zero the prefolded route's fold counter. Test builds only.
#[cfg(test)]
pub(crate) fn reset_scratch_folds() {
    SCRATCH_FOLDS.with(|cell| cell.set(0));
}

/// Bytes folded by `ascii_scratch_contains` on this thread since the last
/// reset. Test builds only.
#[cfg(test)]
pub(crate) fn scratch_folds() -> u32 {
    SCRATCH_FOLDS.with(|cell| cell.get())
}

/// `hc.eq_ignore_ascii_case(nc)` plus, in a test build only, one increment of
/// the path guard's counter. The `#[cfg(test)]` line compiles away in the
/// product, so the scan pays nothing for being observable.
#[inline(always)]
fn byte_eq(hc: u8, nc: u8) -> bool {
    #[cfg(test)]
    BYTE_COMPARES.with(|cell| cell.set(cell.get() + 1));
    hc.eq_ignore_ascii_case(&nc)
}

#[cfg(test)]
thread_local! {
    /// Byte comparisons made by `ascii_contains` on this thread, for
    /// `the_scan_rejects_a_near_miss_in_one_comparison_per_start`. Thread-local
    /// rather than one global `AtomicU32` because `cargo test` runs tests on
    /// several threads at once and the exhaustive pairing tests would add their
    /// own comparisons to the measurement while it ran.
    static BYTE_COMPARES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Zero the path guard's counter. Test builds only.
#[cfg(test)]
pub(crate) fn reset_byte_compares() {
    BYTE_COMPARES.with(|cell| cell.set(0));
}

/// Comparisons made by `ascii_contains` on this thread since the last reset.
/// Test builds only.
#[cfg(test)]
pub(crate) fn byte_compares() -> u32 {
    BYTE_COMPARES.with(|cell| cell.get())
}

#[cfg(test)]
mod chunked_route_tests {
    // Explicit imports, never `use super::*`: this crate's test modules are
    // compiled on a Windows host where the glob form overflows rustc's stack
    // (measured in an earlier round of this line).
    use crate::fold::{
        ascii_chunk_contains, ascii_chunk_contains_windowed, chunk_folds, contains_ignore_case,
        only_ascii_folds, reset_chunk_folds, reset_scratch_folds, scratch_capacity, scratch_folds,
        CHUNK_MAX_BYTES,
    };

    /// The route this round adds is `haystack.to_lowercase().contains(needle)`
    /// for an admitted haystack, so that is the reference every case below is
    /// paired against.
    fn reference(haystack: &str, needle: &str) -> bool {
        haystack.to_lowercase().contains(needle)
    }

    /// The 24-character alphabet the short grids are built from: ASCII letters
    /// in both cases, a digit, `_`, a space, the `·` separator the labels use,
    /// the two characters that fold to two-code-point or context-dependent
    /// strings, the Greek sigmas, Cyrillic in both cases, CJK, the Kelvin sign,
    /// a full-width letter, a Roman numeral and a bare combining dot.
    const ALPHA: &[char] = &[
        'a', 'B', '7', '_', ' ', '·', 'ß', 'İ', 'i', 'Σ', 'σ', 'ς', 'А', 'б', '中', 'µ', 'é', 'É',
        'K', '\u{212a}', 'Ⅻ', 'Ａ', '\u{307}', 'k',
    ];

    fn strings(max_len: usize) -> Vec<Vec<char>> {
        let mut out = Vec::new();
        for len in 1..=max_len {
            let total = ALPHA.len().pow(len as u32);
            for i in 0..total {
                let mut s = Vec::with_capacity(len);
                let mut x = i;
                for _ in 0..len {
                    s.push(ALPHA[x % ALPHA.len()]);
                    x /= ALPHA.len();
                }
                out.push(s);
            }
        }
        out
    }

    /// ① of the four-part argument, checked case by case: on every haystack the
    /// gate admits, the window fold answers exactly what folding the whole cell
    /// and searching it would. The candidate route runs on the admitted subset
    /// - a refused haystack is not this route's business, and
    ///
    /// `a_refused_haystack_never_reaches_the_chunked_route` pins that the entry
    /// never hands it one - and the needles are the haystack's own substrings
    /// plus every single character of the alphabet, so both the hits and the
    /// near misses are in here.
    #[test]
    fn the_chunked_route_agrees_with_the_copying_route_point_by_point() {
        let corpus = strings(3);
        let mut admitted = 0usize;
        let mut refused = 0usize;
        let mut checked = 0usize;
        let mut hits = 0usize;
        let mut contracted = 0usize;
        for chars in &corpus {
            let haystack: String = chars.iter().collect();
            if !only_ascii_folds(&haystack) {
                refused += 1;
                continue;
            }
            admitted += 1;
            let mut needles: Vec<String> = Vec::new();
            for i in 0..chars.len() {
                for j in i + 1..=chars.len() {
                    needles.push(chars[i..j].iter().collect());
                }
            }
            for c in ALPHA {
                needles.push(c.to_string());
            }
            for needle in &needles {
                let want = reference(&haystack, needle);
                hits += want as usize;
                checked += 1;
                for window in [4usize, 8, 16, CHUNK_MAX_BYTES] {
                    assert_eq!(
                        ascii_chunk_contains_windowed(&haystack, needle, window),
                        want,
                        "chunked(window={window})({haystack:?}, {needle:?})"
                    );
                }
                assert_eq!(
                    ascii_chunk_contains(&haystack, needle),
                    want,
                    "chunked({haystack:?}, {needle:?})"
                );
                if needle == &needle.to_lowercase() {
                    // The caller's contract, which is what makes the entry's
                    // answer the reference's: a folded needle must agree here
                    // too, at a length where the entry takes the ring.
                    contracted += 1;
                    assert_eq!(
                        contains_ignore_case(&haystack, needle),
                        want,
                        "entry({haystack:?}, {needle:?})"
                    );
                }
            }
        }
        assert_eq!(corpus.len(), 14_424, "24 + 24^2 + 24^3 strings");
        assert!(admitted > 2_000, "admitted={admitted} refused={refused}");
        assert!(
            refused > 2_000,
            "refused={refused}: the grid must cover both sides"
        );
        assert!(hits > 0 && hits < checked, "hits={hits} of {checked}");
        assert!(contracted > 1_000, "folded-needle cells={contracted}");
    }

    /// ② and ③ of the argument, on a corpus whose characters are 1, 2, 3 and 4
    /// bytes long and windows small enough to put an edge in the middle of them.
    /// A byte-aligned window would cut these characters apart and fall back to
    /// copying the cell; a boundary-aligned one changes no answer, at any needle
    /// length, because the overlap carries the match across the edge.
    #[test]
    fn a_window_edge_that_cuts_a_character_changes_no_answer() {
        // 19 bytes per unit: `ab` (1+1), `·` (2), `订` (3), `中` (3), `🙂` (4),
        // `ß` (2), `µ` (2), a space (1). No uppercase, so every slice of it is
        // already a folded needle, and no `q`, so appending one makes a needle
        // that cannot be found.
        let haystack = "ab·订中🙂ßµ ".repeat(24);
        assert!(only_ascii_folds(&haystack), "corpus must be admitted");
        assert_eq!(haystack.len(), 456, "corpus drifted");
        let mut starts: Vec<usize> = Vec::new();
        for (i, _) in haystack.char_indices() {
            starts.push(i);
        }
        let mut cases = 0usize;
        let mut edge_cut = 0usize;
        for window in [4usize, 8, 16, 64, CHUNK_MAX_BYTES] {
            // Teeth for the claim that this exercises edges at all: some
            // byte-aligned window boundary here sits inside a character, so a
            // route that did not floor it would be looking at invalid UTF-8.
            if window < haystack.len() {
                assert!(
                    (window..haystack.len())
                        .step_by(window)
                        .any(|edge| !haystack.is_char_boundary(edge)),
                    "window={window} never lands inside a character"
                );
            }
            for m in [1usize, 2, 3, 5, 8] {
                for &p in &starts {
                    let Some(end) = p.checked_add(m) else {
                        continue;
                    };
                    if end > haystack.len() || !haystack.is_char_boundary(end) {
                        continue;
                    }
                    let needle = &haystack[p..end];
                    let cut = p / window != (end - 1) / window;
                    edge_cut += cut as usize;
                    assert!(
                        reference(&haystack, needle),
                        "window={window} m={m} p={p}: the reference must find its own slice"
                    );
                    assert!(
                        ascii_chunk_contains_windowed(&haystack, needle, window),
                        "missed a match: window={window} m={m} p={p} {needle:?} (crosses an edge: {cut})"
                    );
                    // And the same slice with a byte that is nowhere in the
                    // corpus must not be found: no window may answer wider than
                    // the reference.
                    let miss = format!("{needle}q");
                    assert!(!reference(&haystack, &miss), "reference found {miss:?}");
                    assert!(
                        !ascii_chunk_contains_windowed(&haystack, &miss, window),
                        "spurious match for {miss:?} at window={window} p={p}"
                    );
                    cases += 1;
                }
            }
        }
        assert!(cases >= 1_000, "only {cases} edge cases");
        assert!(edge_cut >= 300, "{edge_cut} of {cases} straddle an edge");
        // The entry reaches this route only past `SCRATCH_MAX_BYTES`, so drive
        // the same corpus at four times its length through the entry and check
        // the answer is still the reference's.
        let long = haystack.repeat(10);
        assert!(long.len() + 8 > crate::fold::SCRATCH_MAX_BYTES);
        for m in [1usize, 2, 3, 5, 8] {
            for &p in &starts {
                let Some(end) = p.checked_add(m) else {
                    continue;
                };
                if end > haystack.len() || !haystack.is_char_boundary(end) {
                    continue;
                }
                let needle = &haystack[p..end];
                assert_eq!(
                    contains_ignore_case(&long, needle),
                    reference(&long, needle),
                    "entry over the cap, m={m} p={p} {needle:?}"
                );
            }
        }
    }

    /// The gate, seen from the route side: a haystack with a real lowercase
    /// mapping in it is answered by the copy at every length and folds nothing
    /// on either buffer, while an admitted one of the same size streams through
    /// the window and folds each byte exactly once.
    #[test]
    fn a_refused_haystack_never_reaches_the_chunked_route() {
        assert!(!only_ascii_folds("А"));
        assert!(!only_ascii_folds("Σ"));
        assert!(!only_ascii_folds("МОСКВА"));
        let refused = "МОСКВА".repeat(700); // 4,200 B, past the buffer bound
        assert!(refused.len() + 8 > crate::fold::SCRATCH_MAX_BYTES);
        assert!(!only_ascii_folds(&refused));
        reset_chunk_folds();
        reset_scratch_folds();
        let needle = "москва";
        assert_eq!(
            contains_ignore_case(&refused, needle),
            reference(&refused, needle),
            "a refused cell keeps the copying path's answer"
        );
        assert!(contains_ignore_case(&refused, needle));
        assert_eq!(
            chunk_folds(),
            0,
            "refused means the window buffer is not folded into"
        );
        assert_eq!(scratch_folds(), 0, "nor is the whole-cell buffer");

        // Same size, same shape, admitted: the window route answers, the
        // whole-cell buffer sleeps, and no byte is folded more than once beyond
        // the overlap an edge needs. The needle here is one the cell cannot
        // contain, so the answer costs a full scan and the fold counter sees
        // every byte of it.
        let admitted = "订".repeat(1400); // 4,200 B
        assert!(admitted.len() + 8 > crate::fold::SCRATCH_MAX_BYTES);
        assert!(only_ascii_folds(&admitted));
        reset_chunk_folds();
        reset_scratch_folds();
        let needle = "订订q"; // 7 B, and `q` is nowhere in the cell
        let scanned = contains_ignore_case(&admitted, needle);
        assert!(!scanned);
        assert_eq!(scanned, reference(&admitted, needle));
        assert_eq!(
            scratch_folds(),
            0,
            "past the bound the whole-cell buffer sleeps"
        );
        let folded = chunk_folds() as usize;
        assert!(
            folded >= admitted.len(),
            "{folded} folds for a {} B cell",
            admitted.len()
        );
        // Two windows at 4,096 B, so at most one re-read overlap of this needle's
        // length at the single interior edge, plus whatever the floor dropped.
        assert!(
            folded <= admitted.len() + needle.len() + 8,
            "{folded} folds for a {} B cell: the overlap is charged more than once",
            admitted.len()
        );
        // A match answers as soon as the window that holds it does, so a hitting
        // call folds strictly less than the scan above - early, never twice.
        reset_chunk_folds();
        assert!(contains_ignore_case(&admitted, "订订订"));
        assert!(chunk_folds() as usize <= folded);
    }

    /// The two shapes that decide this route's existence: `str::to_lowercase`
    /// folds a word-final `Σ` to `ς` and `İ` to two code points, and a byte-wise
    /// fold does neither. Both haystacks carry a character with a real lowercase
    /// mapping, so the gate keeps them off the window route - and calling the
    /// window route on them directly, which no entry path does, is shown to give
    /// the wrong bit. That is the argument for the gate, kept as a case.
    #[test]
    fn the_final_sigma_and_dotted_capital_traps_stay_on_the_copying_route() {
        assert!(reference("\u{391}\u{3a3}", "\u{3c2}"));
        assert!(!reference("\u{3a3}\u{3c1}\u{391}", "\u{3c2}"));
        assert!(reference("\u{130}", "\u{307}"));
        let mut diverged = 0usize;
        for (haystack, needle) in [
            ("\u{391}\u{3a3}", "\u{3c2}"),
            ("\u{3a3}\u{3c1}\u{391}", "\u{3c2}"),
            ("STRASSE\u{3a3}", "\u{3c2}"),
            ("\u{130}", "\u{307}"),
            ("\u{130}istanbul", "i\u{307}"),
        ] {
            assert!(
                !only_ascii_folds(haystack),
                "{haystack:?} must not be admitted"
            );
            let want = reference(haystack, needle);
            assert_eq!(
                contains_ignore_case(haystack, needle),
                want,
                "entry({haystack:?}, {needle:?})"
            );
            // A byte-wise fold can neither turn a word-final `Σ` into `ς` nor
            // split `İ` into `i` plus a combining dot, so the window route says
            // `false` on all five. Four of them are the reference's `true`: that
            // gap is what the gate keeps off the answer, which is why the route
            // is never handed such a cell - and why raising the buffer bound
            // instead of windowing, which would have put every long cell on it,
            // was turned down.
            assert!(
                !ascii_chunk_contains(haystack, needle),
                "the byte fold cannot produce ({haystack:?}, {needle:?})"
            );
            diverged += want as usize;
        }
        assert_eq!(
            diverged, 4,
            "four of the five cells fold differently in Unicode"
        );
        // And an admitted sigma: `σ` and `ς` fold to themselves, so a cell of
        // them is on the window route and answers as the copy does.
        let cased = format!("{}\u{3a3}", "\u{3c2}\u{3c3}".repeat(1500));
        assert!(!only_ascii_folds(&cased), "the trailing Σ is cased");
        let admitted = "\u{3c2}\u{3c3}".repeat(1500);
        assert!(only_ascii_folds(&admitted));
        assert_eq!(
            contains_ignore_case(&cased, "\u{3c2}\u{3c3}\u{3c2}"),
            reference(&cased, "\u{3c2}\u{3c3}\u{3c2}"),
            "a 6 KB cell ending in Σ is refused, so the copy answers it"
        );
        assert_eq!(
            contains_ignore_case(&admitted, "\u{3c2}\u{3c3}\u{3c2}"),
            reference(&admitted, "\u{3c2}\u{3c3}\u{3c2}")
        );
        assert!(contains_ignore_case(&admitted, "\u{3c2}\u{3c3}\u{3c2}"));
        reset_chunk_folds();
        assert!(contains_ignore_case(&admitted, "\u{3c2}\u{3c3}\u{3c2}"));
        assert!(
            chunk_folds() > 0,
            "the admitted cell answered on the window route"
        );
        reset_chunk_folds();
        assert_eq!(
            contains_ignore_case(&cased, "\u{3c2}\u{3c3}\u{3c2}"),
            reference(&cased, "\u{3c2}\u{3c3}\u{3c2}")
        );
        assert_eq!(
            chunk_folds(),
            0,
            "the refused 6 KB cell folds nothing on the window route"
        );
    }

    /// The residency claim, measured rather than argued: a 1 MB cell and a 4 KB
    /// cell go through the same buffer and it stays the size of one window plus
    /// the needle, whatever the first of them was. This is the whole difference
    /// between this route and raising `SCRATCH_MAX_BYTES`.
    #[test]
    fn the_chunked_route_keeps_the_shared_buffer_bounded() {
        let needle = "订订订"; // 9 B
        let big = "订".repeat(349_525); // 1,048,575 B, an absent needle: full scan
        assert!(big.len() >= 1_000_000, "the big cell is {} B", big.len());
        assert!(only_ascii_folds(&big));
        assert!(!contains_ignore_case(&big, &format!("{needle}q")));
        let after_big = scratch_capacity();
        let small = "订".repeat(1_400); // 4,200 B
        assert!(only_ascii_folds(&small));
        assert!(contains_ignore_case(&small, needle));
        let after_small = scratch_capacity();
        // The route reserves a window plus the needle plus a word; one doubling
        // of slack is allowed for the allocator rounding the reservation up, and
        // the cell's own length is not.
        let bound = 2 * (CHUNK_MAX_BYTES + needle.len() + 8);
        assert!(
            after_big <= bound,
            "residency after a 1 MB cell: {after_big} B, bound {bound} B"
        );
        assert!(
            after_small <= bound,
            "residency after a 4 KB cell: {after_small} B, bound {bound} B"
        );
        assert!(
            after_big < big.len() / 8,
            "{after_big} B of a {} B cell is still resident",
            big.len()
        );
        assert!(after_small <= after_big, "the small cell grew the buffer");
        // The buffer the admitted cells use is the one the cell-sized route
        // already had; no second thread-local was opened for it.
        assert!(
            after_big > 0,
            "the window route did not touch the shared buffer"
        );
    }
}

// ---------------------------------------------------------------------------
// Round 19. `SCAN_MAX_BYTES` was measured against the *other* allocation-free
// route this time and left exactly where it is (name and value unmoved, no
// proposal to move it), so what the three tests below pin is the evidence for
// leaving it, not a new bound. Layers, one test each:
//   * semantics - the five *true* near-miss shapes on gate-true corpora at the
//     three lengths the round measured (64/65/96 B), answered against the
//     copying path as reference (`target/bench_scanbound/RESULTS.md` §4.2),
//   * routing - the dispatch at the seam itself, read through the route
//     counters this crate already has (`BYTE_COMPARES`, `SCRATCH_FOLDS`,
//     `CHUNK_FOLDS`), over the whole 48-160 B span the round swept (§4.1),
//   * the §22.7-12 account carried over from round 18 - the exhaustive
//     short-string tier extended to full strings of 4 and 5 characters.
//
// The corpora are built by a transcription of the round's own
// `build(fam, len, shape, seed)` (`target/bench_scanbound/bench.rs:165-563`,
// pools from `:132-145`), on that file's seeds, so the cells asserted here are
// cells that were measured there. What a test can pin is the answer and the
// route that produced it; the *price* of a route stays on the bench side, and
// the two halves no counter in this crate can observe are stated where they
// fall (allocation counts, and the `from_utf8` `Err` fallback inside (ii)).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod scan_bound_evidence_tests {
    // Explicit imports, never `use super::*` - see the note at the head of
    // `chunked_route_tests`: the glob form overflows rustc's stack on Windows.
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

// ---------------------------------------------------------------------------
// Round 20 stage ②: the two facts the skip-by-width shape of the sieve (that
// round's candidate (a2)) rests on, stated by their own names in the module
// that owns the table. Stage ① measured the shape at 0.384..0.548 of the gate
// on the one-cased-letter family and 2.659..3.489 on Latin-1, so it was not
// adopted; a later round may only adopt it if these two assertions hold, and
// they were previously readable only as a *consequence* of another module's
// test (see the note inside the first test). Evidence on disk:
// `target/bench_gate/raw_probe.txt` (`true_in_0x80_0xBF=false`),
// `target/bench_gate/raw_eq.txt` (`mapping_no_lead_hit=0`,
// `codepoints_with_table_hit_in_continuation=0`, 1,111,935 code points of
// `0x81..=0x10FFFF`), `target/bench_gate/raw_eq2.txt` (`probe2.exe`, a binary
// separate from `bench.exe`, adding the `0x00..=0x80` leg: 16,619,585 pairs,
// zero disagreements with the gate for all five candidates).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod case_lead_table_band_tests {
    // Explicit imports, never `use super::*` - see the note at the head of
    // `chunked_route_tests`: the glob form overflows rustc's stack on Windows.
    use super::CASE_LEAD_BYTE;

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
