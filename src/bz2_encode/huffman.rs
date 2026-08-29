//! Grouped Huffman coding: 2–6 tables, one selector per 50 symbols.
//!
//! ## Code-length cap
//!
//! The decoder in `lib.rs` reads code lengths as a delta walk and rejects any
//! value outside `1..=20` (`invalid bzip2 Huffman delta length`), while its
//! table builder tolerates up to `MAX_CODE_LEN` = 23. So 20 is the hard ceiling
//! that our own decoder — and the format's delta encoding — will accept.
//!
//! We cap at 17 rather than 20 because that is what libbz2's encoder emits, and
//! staying inside the reference encoder's window keeps us safely interoperable
//! with third-party decoders that assume the narrower range. 17 bits is far
//! past the point where the length limit costs anything measurable: the
//! alphabet is at most 258 symbols, so an unconstrained Huffman code only
//! reaches 17 bits on extremely skewed distributions.

use std::vec;
use std::vec::Vec;

use super::bitwriter::BitWriter;

/// Longest code we will emit. See the module note — the decoder accepts 20.
pub const MAX_CODE_LEN: u8 = 17;

/// The decoder's delta reader rejects any length outside `1..=20`, so our cap
/// has to stay inside that window. Enforced at compile time.
const _: () = assert!(MAX_CODE_LEN >= 1 && MAX_CODE_LEN <= 20);

/// Symbols per selector group.
pub const GROUP_SIZE: usize = 50;

/// Refinement passes over the group→table assignment, as in libbz2.
const N_ITERS: usize = 4;

/// The Huffman coding chosen for one block.
pub struct Coding {
    /// Number of tables actually used (2..=6).
    pub n_groups: usize,
    /// Table index for each group of `GROUP_SIZE` symbols.
    pub selectors: Vec<u8>,
    /// Code lengths per table, each `alpha_size` long.
    pub lens: Vec<Vec<u8>>,
    /// Canonical codes per table.
    pub codes: Vec<Vec<u32>>,
}

/// Choose the table count the way bzip2 does, from the symbol count.
fn group_count(n_syms: usize) -> usize { match n_syms { 0..=199 => 2, 200..=599 => 3, 600..=1199 => 4, 1200..=2399 => 5, _ => 6 } }

/// Build the grouped Huffman coding for one block's symbol stream.
pub fn build(syms: &[u16], alpha_size: usize) -> Coding {
    assert!(!syms.is_empty(), "empty symbol stream");
    assert!(alpha_size >= 3);

    let n_groups = group_count(syms.len());
    let mut lens = initial_lengths(syms, alpha_size, n_groups);

    for _ in 0..N_ITERS {
        let mut rfreq = vec![vec![0u32; alpha_size]; n_groups];
        let selectors = assign_selectors(syms, &lens);
        for (g, &t) in selectors.iter().enumerate() {
            let start = g * GROUP_SIZE;
            let end = (start + GROUP_SIZE).min(syms.len());
            for &s in &syms[start..end] { rfreq[t as usize][s as usize] += 1; }
        }
        for (t, len) in lens.iter_mut().enumerate() {
            // A zero-frequency symbol still needs a code: the delta encoding
            // has no way to say "unused", so every symbol gets weight >= 1.
            let weights: Vec<u64> = rfreq[t].iter().map(|&f| f.max(1) as u64).collect();
            *len = package_merge(&weights, MAX_CODE_LEN);
        }
    }

    // One last assignment against the final code lengths. For fixed lengths the
    // per-group argmin is optimal, so this can only shrink the output.
    let selectors = assign_selectors(syms, &lens);

    let codes = lens.iter().map(|l| canonical_codes(l)).collect();

    Coding { n_groups, selectors, lens, codes }
}

/// Pick, for each group of 50 symbols, the table that codes it in fewest bits.
fn assign_selectors(syms: &[u16], lens: &[Vec<u8>]) -> Vec<u8> {
    let mut selectors = Vec::with_capacity(syms.len() / GROUP_SIZE + 1);
    let mut start = 0usize;
    while start < syms.len() {
        let end = (start + GROUP_SIZE).min(syms.len());
        let mut best = 0usize;
        let mut best_cost = u64::MAX;
        for (t, len) in lens.iter().enumerate() {
            let cost: u64 = syms[start..end].iter().map(|&s| len[s as usize] as u64).sum();
            if cost < best_cost {
                best_cost = cost;
                best = t;
            }
        }
        selectors.push(best as u8);
        start = end;
    }
    selectors
}

/// Seed the tables by splitting the alphabet into runs of roughly equal total
/// frequency — libbz2's starting point, which the refinement passes then move.
fn initial_lengths(syms: &[u16], alpha_size: usize, n_groups: usize) -> Vec<Vec<u8>> {
    let mut freq = vec![0u32; alpha_size];
    for &s in syms { freq[s as usize] += 1; }

    let mut lens = vec![vec![0u8; alpha_size]; n_groups];
    let mut remaining: u32 = syms.len() as u32;
    let mut gs: usize = 0;
    let mut n_part = n_groups;

    while n_part > 0 {
        let target = remaining / n_part as u32;
        // `ge` is an inclusive upper bound that starts just below `gs`, so an
        // empty slice is representable.
        let mut ge: isize = gs as isize - 1;
        let mut acc: u32 = 0;
        while acc < target && ge < alpha_size as isize - 1 {
            ge += 1;
            acc += freq[ge as usize];
        }
        if ge > gs as isize && n_part != n_groups && n_part != 1 && (n_groups - n_part) % 2 == 1 {
            acc -= freq[ge as usize];
            ge -= 1;
        }
        for (v, slot) in lens[n_part - 1].iter_mut().enumerate() { *slot = if v as isize >= gs as isize && v as isize <= ge { 1 } else { 15 }; }
        n_part -= 1;
        gs = (ge + 1) as usize;
        remaining -= acc;
    }

    lens
}

/// Length-limited optimal prefix code lengths by package-merge.
///
/// Every weight must be at least 1, so every symbol gets a code. `limit` must
/// satisfy `2^limit >= weights.len()`.
pub fn package_merge(weights: &[u64], limit: u8) -> Vec<u8> {
    let m = weights.len();
    assert!(m >= 2, "package-merge needs at least two symbols");
    assert!(limit >= 1 && (limit >= 32 || (1u64 << limit) >= m as u64), "code-length limit too small for the alphabet");

    const NONE: u32 = u32::MAX;
    // Parallel arrays instead of a struct so the arena stays compact.
    let mut weight: Vec<u64> = Vec::with_capacity(m * 2);
    let mut left: Vec<u32> = Vec::with_capacity(m * 2);
    let mut right: Vec<u32> = Vec::with_capacity(m * 2);

    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by_key(|&i| (weights[i], i));

    // The original coins, ascending by weight; the same list is re-merged at
    // every denomination.
    let mut leaves: Vec<u32> = Vec::with_capacity(m);
    for &sym in &order {
        weight.push(weights[sym]);
        left.push(NONE);
        right.push(sym as u32);
        leaves.push((weight.len() - 1) as u32);
    }

    let mut level = leaves.clone();
    for _ in 1..limit {
        let mut packaged: Vec<u32> = Vec::with_capacity(level.len() / 2);
        let mut i = 0;
        while i + 1 < level.len() {
            let (a, b) = (level[i], level[i + 1]);
            weight.push(weight[a as usize] + weight[b as usize]);
            left.push(a);
            right.push(b);
            packaged.push((weight.len() - 1) as u32);
            i += 2;
        }

        // Merge the fresh packages back in with the original coins.
        let mut merged = Vec::with_capacity(leaves.len() + packaged.len());
        let (mut x, mut y) = (0usize, 0usize);
        while x < leaves.len() || y < packaged.len() {
            let take_leaf = y >= packaged.len() || (x < leaves.len() && weight[leaves[x] as usize] <= weight[packaged[y] as usize]);
            if take_leaf {
                merged.push(leaves[x]);
                x += 1;
            } else {
                merged.push(packaged[y]);
                y += 1;
            }
        }
        level = merged;
    }

    // The cheapest 2m-2 coins form the solution; a symbol's code length is the
    // number of selected coins it appears in.
    let mut lens = vec![0u8; m];
    let mut stack: Vec<u32> = Vec::new();
    for &node in level.iter().take(2 * m - 2) {
        stack.push(node);
        while let Some(nd) = stack.pop() {
            if left[nd as usize] == NONE { lens[right[nd as usize] as usize] += 1; } else {
                stack.push(left[nd as usize]);
                stack.push(right[nd as usize]);
            }
        }
    }

    debug_assert!(lens.iter().all(|&l| l >= 1 && l <= limit));
    lens
}

/// Canonical code assignment, matching the decoder's `limit`/`base`/`perm`
/// construction: codes are handed out in ascending length, and within a length
/// in ascending symbol order.
pub fn canonical_codes(lens: &[u8]) -> Vec<u32> {
    let min = *lens.iter().min().unwrap();
    let max = *lens.iter().max().unwrap();
    let mut codes = vec![0u32; lens.len()];
    let mut next = 0u32;
    for l in min..=max {
        for (i, &ln) in lens.iter().enumerate() {
            if ln == l {
                codes[i] = next;
                next += 1;
            }
        }
        next <<= 1;
    }
    codes
}

/// Emit the selector list, move-to-front coded over the table indices and then
/// written in unary.
pub fn write_selectors(w: &mut BitWriter, selectors: &[u8], n_groups: usize) {
    let mut pos: Vec<u8> = (0..n_groups as u8).collect();
    for &sel in selectors {
        let j = pos.iter().position(|&p| p == sel).expect("unknown selector");
        for _ in 0..j { w.write_bit(1); }
        w.write_bit(0);
        let v = pos[j];
        pos.copy_within(0..j, 1);
        pos[0] = v;
    }
}

/// Emit each table's code lengths as a delta walk from the previous length.
pub fn write_tables(w: &mut BitWriter, lens: &[Vec<u8>]) {
    for table in lens {
        let mut curr = table[0] as i32;
        w.write_bits(5, curr as u32);
        for &len in table {
            let target = len as i32;
            while curr < target {
                w.write_bits(2, 0b10); // "1 then 0" -> increment
                curr += 1;
            }
            while curr > target {
                w.write_bits(2, 0b11); // "1 then 1" -> decrement
                curr -= 1;
            }
            w.write_bit(0);
        }
    }
}
