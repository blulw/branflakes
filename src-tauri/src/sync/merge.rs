//! Pure reconciliation logic for Spotify playlist sync. No I/O.
//!
//! # Three-way merge
//! Items are Spotify URIs. Duplicates are allowed, so each occurrence is
//! treated as a distinct *token* `(uri, n)` = the n-th occurrence of `uri`.
//! For every token we know whether it is in the base (last agreed state),
//! in Branflakes (local) and on Spotify (remote):
//!
//! | local vs base | remote vs base | result               |
//! |---------------|----------------|----------------------|
//! | same          | any            | remote               |
//! | changed       | same           | local                |
//! | changed       | changed        | keep if either has it|
//!
//! The last row means a conflicting removal never wins over an addition,
//! and the same track added on both sides is kept once (no duplicates).
//!
//! Spotify items Branflakes can't hold (Spotify local files, podcast
//! episodes, unavailable placeholders) are "foreign": Branflakes never
//! changes them, so for them local == base by definition.
//!
//! Order: if only one side changed, its order is used. If both changed,
//! Spotify's order is kept and Branflakes additions are inserted after the
//! item they followed in Branflakes. A Branflakes-side *reorder* that
//! conflicts with Spotify changes is reported as a conflict.
//!
//! # Push planning
//! `plan_push` turns the current Spotify list into the merged list with the
//! documented `/items` operations: remove by URI, move a range, add at a
//! position, or (rarely) replace. `apply` simulates each op with Spotify's
//! semantics so the plan can be verified before anything is sent.

use std::collections::{HashMap, HashSet};

/// Max URIs per add/replace request (Spotify limit).
pub const MAX_URIS_PER_REQUEST: usize = 100;
/// Above this many single-item moves, a full replace is cheaper (when safe).
pub const MAX_MOVES: usize = 25;

pub fn is_track(uri: &str) -> bool {
    uri.starts_with("spotify:track:")
}

/// URIs that the add/remove endpoints accept.
pub fn is_pushable(uri: &str) -> bool {
    uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:")
}

type Token = (String, usize);

fn tokens(seq: &[String]) -> Vec<Token> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    seq.iter()
        .map(|k| {
            let n = seen.entry(k.as_str()).or_insert(0);
            let t = (k.clone(), *n);
            *n += 1;
            t
        })
        .collect()
}

/// Insert every token of `secondary` that `keep` accepts and `result` lacks,
/// directly after the nearest preceding `secondary` token already in
/// `result` (or at the front).
fn insert_missing(result: &mut Vec<Token>, secondary: &[Token], keep: &dyn Fn(&Token) -> bool) {
    let mut present: HashSet<Token> = result.iter().cloned().collect();
    for (i, t) in secondary.iter().enumerate() {
        if present.contains(t) || !keep(t) {
            continue;
        }
        let anchor = secondary[..i].iter().rev().find(|p| present.contains(*p));
        let at = match anchor {
            Some(a) => result.iter().position(|x| x == a).map(|p| p + 1).unwrap_or(result.len()),
            None => 0,
        };
        result.insert(at, t.clone());
        present.insert(t.clone());
    }
}

/// Relative order of the tokens common to both sequences.
fn common_order(a: &[Token], b: &[Token]) -> (Vec<Token>, Vec<Token>) {
    let sa: HashSet<&Token> = a.iter().collect();
    let sb: HashSet<&Token> = b.iter().collect();
    (a.iter().filter(|t| sb.contains(t)).cloned().collect(), b.iter().filter(|t| sa.contains(t)).cloned().collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeOutcome {
    pub merged: Vec<String>,
    pub local_changed: bool,
    pub remote_changed: bool,
    /// Both sides changed in incompatible ways; data was kept, but a
    /// Branflakes reorder or removal could not be applied.
    pub conflict: bool,
}

/// Three-way merge. `local` holds only `spotify:track:` URIs (Branflakes
/// can't hold anything else); `base` and `remote` may hold foreign items.
pub fn merge(base: &[String], local: &[String], remote: &[String]) -> MergeOutcome {
    let base_t = tokens(base);
    let base_tracks: Vec<String> = base.iter().filter(|k| is_track(k)).cloned().collect();
    let local_changed = local != base_tracks.as_slice();
    let remote_changed = remote != base;

    // Branflakes doesn't hold foreign items: put base's back where they were.
    let local_t: Vec<Token> = if local_changed {
        let mut t = tokens(local);
        insert_missing(&mut t, &base_t, &|tok| !is_track(&tok.0));
        t
    } else {
        base_t.clone()
    };
    let remote_t = tokens(remote);

    let out = |t: Vec<Token>| t.into_iter().map(|(k, _)| k).collect::<Vec<_>>();
    if !local_changed {
        return MergeOutcome { merged: remote.to_vec(), local_changed, remote_changed, conflict: false };
    }
    if !remote_changed {
        return MergeOutcome { merged: out(local_t), local_changed, remote_changed, conflict: false };
    }

    let in_base: HashSet<&Token> = base_t.iter().collect();
    let in_local: HashSet<&Token> = local_t.iter().collect();
    let in_remote: HashSet<&Token> = remote_t.iter().collect();
    let mut conflict = false;
    let keep = |t: &Token| -> bool {
        let (b, l, r) = (in_base.contains(t), in_local.contains(t), in_remote.contains(t));
        if l == b {
            r
        } else if r == b {
            l
        } else {
            l || r
        }
    };
    // Removed locally but changed (re-added) remotely, or vice versa.
    for t in base_t.iter().chain(local_t.iter()).chain(remote_t.iter()) {
        let (b, l, r) = (in_base.contains(t), in_local.contains(t), in_remote.contains(t));
        if l != b && r != b && l != r {
            conflict = true;
        }
    }
    let (lo, bo) = common_order(&local_t, &base_t);
    if lo != bo {
        conflict = true; // Branflakes reorder; Spotify's order is kept
    }

    let mut result: Vec<Token> = remote_t.iter().filter(|t| keep(t)).cloned().collect();
    insert_missing(&mut result, &local_t, &keep);
    MergeOutcome { merged: out(result), local_changed, remote_changed, conflict }
}

// ------------------------------------------------------------- push plan

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// DELETE /items — removes every occurrence of each URI.
    Remove(Vec<String>),
    /// PUT /items reorder — move one item.
    Move { range_start: usize, insert_before: usize },
    /// POST /items at a position.
    Add { uris: Vec<String>, position: usize },
    /// PUT /items replace (≤ 100 URIs).
    Replace(Vec<String>),
}

/// Apply one op to a simulated list with Spotify's documented semantics.
pub fn apply(list: &mut Vec<String>, op: &Op) {
    match op {
        Op::Remove(uris) => list.retain(|u| !uris.contains(u)),
        Op::Move { range_start, insert_before } => {
            let item = list.remove(*range_start);
            let at = if insert_before > range_start { insert_before - 1 } else { *insert_before };
            list.insert(at.min(list.len()), item);
        }
        Op::Add { uris, position } => {
            let at = (*position).min(list.len());
            list.splice(at..at, uris.iter().cloned());
        }
        Op::Replace(uris) => *list = uris.clone(),
    }
}

fn add_ops(target: &[String], is_addition: &[bool]) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut i = 0;
    while i < target.len() {
        if !is_addition[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < target.len() && is_addition[i] {
            i += 1;
        }
        for (c, chunk) in target[start..i].chunks(MAX_URIS_PER_REQUEST).enumerate() {
            ops.push(Op::Add { uris: chunk.to_vec(), position: start + c * MAX_URIS_PER_REQUEST });
        }
    }
    ops
}

/// Longest increasing subsequence (indices into `v`).
fn lis(v: &[usize]) -> HashSet<usize> {
    let mut tails: Vec<usize> = Vec::new(); // indices into v
    let mut prev = vec![usize::MAX; v.len()];
    for i in 0..v.len() {
        let pos = tails.partition_point(|&t| v[t] < v[i]);
        if pos > 0 {
            prev[i] = tails[pos - 1];
        }
        if pos == tails.len() {
            tails.push(i);
        } else {
            tails[pos] = i;
        }
    }
    let mut out = HashSet::new();
    let mut cur = tails.last().copied();
    while let Some(i) = cur {
        out.insert(i);
        cur = if prev[i] == usize::MAX { None } else { Some(prev[i]) };
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPlanError(pub String);

/// Operations that turn `remote` into `target`. The plan is verified by
/// simulation; an error means it could not be planned safely.
pub fn plan_push(remote: &[String], target: &[String]) -> Result<Vec<Op>, PushPlanError> {
    if remote == target {
        return Ok(Vec::new());
    }
    let r_t = tokens(remote);
    let m_t = tokens(target);
    let m_set: HashSet<&Token> = m_t.iter().collect();

    // 1. Removals. DELETE removes every occurrence of a URI, so any surviving
    //    occurrences of that URI are re-added in step 3.
    let mut removed: Vec<String> = Vec::new();
    for t in &r_t {
        if !m_set.contains(t) && !removed.contains(&t.0) {
            if !is_pushable(&t.0) {
                return Err(PushPlanError(format!("cannot remove {} through the Web API", t.0)));
            }
            removed.push(t.0.clone());
        }
    }
    let mut ops: Vec<Op> = removed.chunks(MAX_URIS_PER_REQUEST).map(|c| Op::Remove(c.to_vec())).collect();
    let cur: Vec<String> = remote.iter().filter(|u| !removed.contains(u)).cloned().collect();

    // 2. Reorder what remains into target order (fewest moves via LIS).
    let cur_count: HashMap<&str, usize> = cur.iter().fold(HashMap::new(), |mut m, u| {
        *m.entry(u.as_str()).or_insert(0) += 1;
        m
    });
    let is_addition: Vec<bool> = m_t.iter().map(|(k, n)| *n >= *cur_count.get(k.as_str()).unwrap_or(&0)).collect();
    for (t, add) in m_t.iter().zip(&is_addition) {
        if *add && !is_pushable(&t.0) {
            return Err(PushPlanError(format!("cannot add {} through the Web API", t.0)));
        }
    }
    let core: Vec<Token> = m_t.iter().zip(&is_addition).filter(|(_, a)| !**a).map(|(t, _)| t.clone()).collect();
    let core_pos: HashMap<&Token, usize> = core.iter().enumerate().map(|(i, t)| (t, i)).collect();
    let cur_t = tokens(&cur);
    let seq: Vec<usize> = cur_t.iter().map(|t| core_pos[t]).collect();
    let keep = lis(&seq);
    let stay: HashSet<Token> = keep.iter().map(|&i| cur_t[i].clone()).collect();

    let mut moves = Vec::new();
    let mut sim = cur_t.clone();
    for (ci, t) in core.iter().enumerate() {
        if stay.contains(t) {
            continue;
        }
        let j = sim.iter().position(|x| x == t).expect("token present");
        // Final position: right after its predecessor in target order.
        let f = if ci == 0 {
            0
        } else {
            let p = sim.iter().position(|x| x == &core[ci - 1]).expect("predecessor present");
            if p < j { p + 1 } else { p }
        };
        let insert_before = if f <= j { f } else { f + 1 };
        if insert_before == j || insert_before == j + 1 {
            continue;
        }
        let item = sim.remove(j);
        sim.insert(f, item);
        moves.push(Op::Move { range_start: j, insert_before });
    }

    let all_pushable = remote.iter().chain(target.iter()).all(|u| is_pushable(u));
    if moves.len() > MAX_MOVES && all_pushable {
        // Cheaper and still exact: replace the first 100, append the rest.
        let mut ops = vec![Op::Replace(target[..target.len().min(MAX_URIS_PER_REQUEST)].to_vec())];
        let rest = &target[target.len().min(MAX_URIS_PER_REQUEST)..];
        for (c, chunk) in rest.chunks(MAX_URIS_PER_REQUEST).enumerate() {
            ops.push(Op::Add { uris: chunk.to_vec(), position: MAX_URIS_PER_REQUEST + c * MAX_URIS_PER_REQUEST });
        }
        return verify(remote, target, ops);
    }
    ops.extend(moves);
    // 3. Additions, ascending by final position.
    ops.extend(add_ops(target, &is_addition));
    verify(remote, target, ops)
}

fn verify(remote: &[String], target: &[String], ops: Vec<Op>) -> Result<Vec<Op>, PushPlanError> {
    let mut sim = remote.to_vec();
    for op in &ops {
        apply(&mut sim, op);
    }
    if sim != target {
        return Err(PushPlanError("internal: push plan does not reproduce the merged playlist".into()));
    }
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace()
            .map(|x| if x.contains(':') { x.to_string() } else { format!("spotify:track:{x}") })
            .collect()
    }

    #[test]
    fn unchanged_is_noop() {
        let m = merge(&v("a b c"), &v("a b c"), &v("a b c"));
        assert_eq!(m.merged, v("a b c"));
        assert!(!m.local_changed && !m.remote_changed && !m.conflict);
        assert!(plan_push(&v("a b c"), &m.merged).unwrap().is_empty());
    }

    #[test]
    fn local_addition_goes_to_spotify_at_intended_position() {
        let m = merge(&v("a b c"), &v("a x b c"), &v("a b c"));
        assert_eq!(m.merged, v("a x b c"));
        let ops = plan_push(&v("a b c"), &m.merged).unwrap();
        assert_eq!(ops, vec![Op::Add { uris: v("x"), position: 1 }]);
    }

    #[test]
    fn remote_addition_is_imported_in_spotify_order() {
        let m = merge(&v("a b"), &v("a b"), &v("a y b"));
        assert_eq!(m.merged, v("a y b"));
        assert!(plan_push(&v("a y b"), &m.merged).unwrap().is_empty(), "nothing to push back: no loop");
    }

    #[test]
    fn second_sync_after_push_is_a_noop_no_loop() {
        // Sync 1: local adds x; base becomes merged.
        let m1 = merge(&v("a b"), &v("a b x"), &v("a b"));
        let base = m1.merged.clone();
        // Sync 2: Spotify now reports what we pushed.
        let m2 = merge(&base, &v("a b x"), &v("a b x"));
        assert!(!m2.local_changed && !m2.remote_changed);
        assert!(plan_push(&v("a b x"), &m2.merged).unwrap().is_empty());
    }

    #[test]
    fn same_track_added_on_both_sides_is_kept_once() {
        let m = merge(&v("a"), &v("a x"), &v("a x"));
        assert_eq!(m.merged, v("a x"));
        assert!(!m.conflict);
    }

    #[test]
    fn both_sides_add_different_tracks() {
        let m = merge(&v("a b"), &v("a b x"), &v("y a b"));
        assert_eq!(m.merged, v("y a b x"));
        let ops = plan_push(&v("y a b"), &m.merged).unwrap();
        assert_eq!(ops, vec![Op::Add { uris: v("x"), position: 3 }]);
    }

    #[test]
    fn removals_propagate_only_when_one_side_removed() {
        // Removed in Branflakes, unchanged on Spotify -> removed on Spotify.
        let m = merge(&v("a b c"), &v("a c"), &v("a b c"));
        assert_eq!(m.merged, v("a c"));
        assert_eq!(plan_push(&v("a b c"), &m.merged).unwrap(), vec![Op::Remove(v("b"))]);
        // Removed on Spotify -> removed in Branflakes.
        assert_eq!(merge(&v("a b c"), &v("a b c"), &v("a c")).merged, v("a c"));
    }

    #[test]
    fn conflicting_removal_never_loses_an_item() {
        // Branflakes removed b; Spotify reordered and added z. Spotify still
        // has b exactly as in base, so Branflakes' removal applies.
        let m = merge(&v("a b"), &v("a"), &v("b a z"));
        assert_eq!(m.merged, v("a z"));
        // Removed in Branflakes while Spotify removed and re-added it:
        // the addition wins (never lose an item on conflict).
        let m = merge(&v("a b"), &v("a"), &v("a b b"));
        assert!(m.merged.contains(&"spotify:track:b".to_string()));
        // Branflakes removed one copy of a duplicate, Spotify added another copy.
        let m = merge(&v("a a"), &v("a"), &v("a a a"));
        assert_eq!(m.merged, v("a a"), "one removal + one addition");
    }

    #[test]
    fn empty_or_partial_remote_with_base_is_trusted_only_from_complete_fetches() {
        // The engine never calls merge with an incomplete remote (it aborts).
        // An empty base (fresh/corrupt) never removes anything.
        let m = merge(&[], &v("a b"), &v("c"));
        assert_eq!(m.merged.len(), 3);
    }

    #[test]
    fn foreign_items_are_preserved_and_never_pushed() {
        let base = v("a spotify:local:x:y:z:1 b");
        // Branflakes can't hold the local file; it added c.
        let local = v("a b c");
        let remote = base.clone();
        let m = merge(&base, &local, &remote);
        assert_eq!(m.merged, v("a spotify:local:x:y:z:1 b c"));
        let ops = plan_push(&remote, &m.merged).unwrap();
        assert_eq!(ops, vec![Op::Add { uris: v("c"), position: 3 }]);
        // Unchanged local (only lacking the foreign item) is not a change.
        assert!(!merge(&base, &v("a b"), &remote).local_changed);
    }

    #[test]
    fn local_reorder_is_pushed_as_a_move() {
        let m = merge(&v("a b c d"), &v("d a b c"), &v("a b c d"));
        assert_eq!(m.merged, v("d a b c"));
        let ops = plan_push(&v("a b c d"), &m.merged).unwrap();
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert!(matches!(ops[0], Op::Move { .. }));
    }

    #[test]
    fn concurrent_reorder_keeps_spotify_order_and_flags_conflict() {
        let m = merge(&v("a b c"), &v("c a b x"), &v("b a c"));
        assert!(m.conflict);
        // Spotify's order wins; x goes after b, which it followed in Branflakes.
        assert_eq!(m.merged, v("b x a c"));
    }

    #[test]
    fn duplicate_removal_of_one_copy_is_planned_exactly() {
        let ops = plan_push(&v("a b a"), &v("a b")).unwrap();
        assert_eq!(ops, vec![Op::Remove(v("a")), Op::Add { uris: v("a"), position: 0 }]);
    }

    #[test]
    fn large_additions_are_chunked_to_100() {
        let remote = v("a");
        let mut target = remote.clone();
        target.extend((0..250).map(|i| format!("spotify:track:n{i}")));
        let ops = plan_push(&remote, &target).unwrap();
        assert_eq!(ops.len(), 3);
        assert!(ops.iter().all(|o| matches!(o, Op::Add { uris, .. } if uris.len() <= 100)));
    }

    #[test]
    fn many_moves_fall_back_to_replace_only_when_safe() {
        let remote: Vec<String> = (0..60).map(|i| format!("spotify:track:t{i}")).collect();
        let target: Vec<String> = remote.iter().rev().cloned().collect();
        let ops = plan_push(&remote, &target).unwrap();
        assert!(matches!(ops[0], Op::Replace(_)));
        // With a Spotify local file present, replace would drop it: moves only.
        let mut r2 = remote.clone();
        r2.push("spotify:local:a:b:c:1".into());
        let mut t2 = target.clone();
        t2.push("spotify:local:a:b:c:1".into());
        let ops = plan_push(&r2, &t2).unwrap();
        assert!(ops.iter().all(|o| matches!(o, Op::Move { .. })));
    }

    /// Randomised check: for many random merges the plan reproduces the target.
    #[test]
    fn random_plans_always_reproduce_target() {
        let mut seed: u64 = 0x5eed;
        let mut rnd = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..2000 {
            let len = rnd(9) as usize;
            let mk = |len: usize, rnd: &mut dyn FnMut(u64) -> u64| -> Vec<String> {
                (0..len).map(|_| format!("spotify:track:{}", rnd(6))).collect()
            };
            let base = mk(len, &mut rnd);
            let local = mk(rnd(9) as usize, &mut rnd);
            let remote = mk(rnd(9) as usize, &mut rnd);
            let m = merge(&base, &local, &remote);
            let ops = plan_push(&remote, &m.merged).expect("plannable");
            let mut sim = remote.clone();
            for op in &ops {
                apply(&mut sim, op);
            }
            assert_eq!(sim, m.merged);
            // Merging again with the result as base is stable (no loop).
            let again = merge(&m.merged, &m.merged.iter().filter(|u| is_track(u)).cloned().collect::<Vec<_>>(), &m.merged);
            assert_eq!(again.merged, m.merged);
            assert!(!again.local_changed && !again.remote_changed);
        }
    }
}
