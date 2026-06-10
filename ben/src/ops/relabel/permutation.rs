use super::errors::RelabelError;
use std::collections::HashMap;
use std::io;

/// Convert a sparse permutation map into a dense index vector.
///
/// Rejects maps that are not permutations: the new-index side must be contiguous from 0, and the
/// old-index side must use each index at most once. A duplicated old index would silently copy one
/// node's value into several positions while dropping another node entirely.
pub(super) fn dense_permutation(
    new_to_old_node_map: &HashMap<usize, usize>,
) -> io::Result<Vec<usize>> {
    let Some(max_key) = new_to_old_node_map.keys().copied().max() else {
        return Ok(Vec::new());
    };

    let mut permutation = vec![usize::MAX; max_key + 1];
    for (&new_idx, &old_idx) in new_to_old_node_map {
        permutation[new_idx] = old_idx;
    }

    let missing = permutation.iter().filter(|&&x| x == usize::MAX).count();
    if missing > 0 {
        return Err(io::Error::from(RelabelError::NonContiguousMap {
            max_key,
            missing,
        }));
    }

    // Old-side injectivity. Out-of-range old indices are caught later against the actual
    // assignment length (`permute_assignment`), so only duplicates are checked here; duplicates
    // would otherwise pass every later check and silently scramble node data.
    let mut seen = std::collections::HashSet::with_capacity(permutation.len());
    for &old_idx in &permutation {
        if !seen.insert(old_idx) {
            return Err(io::Error::from(RelabelError::DuplicateOldIndex { old_idx }));
        }
    }

    Ok(permutation)
}

/// Error for an input whose distinct district ids cannot all receive a one-based `u16` label:
/// labels start at 1, so at most `u16::MAX` (65,535) distinct ids are representable.
fn too_many_labels_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "assignment has more than 65535 distinct district ids; \
         one-based u16 labels cannot represent them all",
    )
}

/// Remap an assignment vector's district labels in first-seen order, starting at 1.
///
/// Errors if the assignment holds more than `u16::MAX` distinct ids, which one-based `u16` labels
/// cannot represent — wrapping would silently alias two districts.
pub(super) fn first_seen_relabel_assignment(assignment: &[u16]) -> io::Result<Vec<u16>> {
    let mut label_map = HashMap::new();
    let mut next_label = 0u16;
    let mut out = Vec::with_capacity(assignment.len());

    for &value in assignment {
        let mapped = match label_map.get(&value) {
            Some(mapped) => *mapped,
            None => {
                next_label = next_label
                    .checked_add(1)
                    .ok_or_else(too_many_labels_error)?;
                label_map.insert(value, next_label);
                next_label
            }
        };
        out.push(mapped);
    }

    Ok(out)
}

/// Rewrite the value of each `(val, len)` RLE pair in first-seen order, in place.
///
/// Errors if the runs hold more than `u16::MAX` distinct ids; see
/// [`first_seen_relabel_assignment`]. On error, a prefix of `runs` may already be relabeled —
/// callers treat the whole operation as failed and discard.
pub(super) fn first_seen_relabel_rle(runs: &mut [(u16, u16)]) -> io::Result<()> {
    let mut label_map = HashMap::new();
    let mut label = 0u16;
    label_map.reserve(runs.len());
    for (val, _len) in runs {
        let new_val = match label_map.get(val) {
            Some(v) => *v,
            None => {
                label = label.checked_add(1).ok_or_else(too_many_labels_error)?;
                label_map.insert(*val, label);
                label
            }
        };
        *val = new_val;
    }
    Ok(())
}

/// Reorder an assignment vector according to a dense permutation.
pub(super) fn permute_assignment(
    assignment: &[u16],
    permutation: &[usize],
) -> io::Result<Vec<u16>> {
    if assignment.len() != permutation.len() {
        return Err(io::Error::from(RelabelError::LengthMismatch {
            map_len: permutation.len(),
            assignment_len: assignment.len(),
        }));
    }

    let mut out = vec![0u16; permutation.len()];
    for (new_idx, &old_idx) in permutation.iter().enumerate() {
        if old_idx >= assignment.len() {
            return Err(io::Error::from(RelabelError::OldIndexOutOfRange {
                old_idx,
                assignment_len: assignment.len(),
            }));
        }
        out[new_idx] = assignment[old_idx];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::rle::{assign_to_rle, rle_to_vec};

    // ── dense_permutation ───────────────────────────────────────────────

    #[test]
    fn dense_permutation_empty_map_returns_empty_vec() {
        let map = HashMap::new();
        assert!(dense_permutation(&map).unwrap().is_empty());
    }

    #[test]
    fn dense_permutation_contiguous_map_yields_dense_vec() {
        let map: HashMap<usize, usize> = [(0, 2), (1, 0), (2, 1)].into_iter().collect();
        assert_eq!(dense_permutation(&map).unwrap(), vec![2, 0, 1]);
    }

    #[test]
    fn dense_permutation_non_contiguous_below_max_errors() {
        let map: HashMap<usize, usize> = [(0, 0), (2, 1)].into_iter().collect();
        let err = dense_permutation(&map).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("contiguous"));
    }

    #[test]
    fn dense_permutation_non_zero_start_errors() {
        // {1 -> 10}: slot 0 missing; pin today's behavior.
        let map: HashMap<usize, usize> = [(1, 10)].into_iter().collect();
        let err = dense_permutation(&map).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn dense_permutation_duplicate_old_indices_rejected() {
        // {0 -> 5, 1 -> 5}: a duplicated old index is not a permutation — applying it would copy
        // one node's value into two positions and silently drop another node entirely.
        let map: HashMap<usize, usize> = [(0, 5), (1, 5)].into_iter().collect();
        let err = dense_permutation(&map).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("more than once"));
    }

    // ── first_seen_relabel_assignment ──────────────────────────────────

    #[test]
    fn first_seen_relabel_assignment_empty() {
        assert!(first_seen_relabel_assignment(&[]).unwrap().is_empty());
    }

    #[test]
    fn first_seen_relabel_assignment_all_same() {
        assert_eq!(
            first_seen_relabel_assignment(&[7, 7, 7]).unwrap(),
            vec![1, 1, 1]
        );
    }

    #[test]
    fn first_seen_relabel_assignment_monotonic() {
        assert_eq!(
            first_seen_relabel_assignment(&[2, 3, 4, 5]).unwrap(),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn first_seen_relabel_assignment_reversed() {
        assert_eq!(
            first_seen_relabel_assignment(&[5, 4, 3, 2]).unwrap(),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn first_seen_relabel_assignment_with_gaps() {
        assert_eq!(
            first_seen_relabel_assignment(&[1, 5, 9, 5, 1, 9]).unwrap(),
            vec![1, 2, 3, 2, 1, 3]
        );
    }

    #[test]
    fn first_seen_relabel_rejects_more_distinct_ids_than_labels() {
        // All 65,536 distinct u16 ids: one-based labels max out at 65,535, so the 65,536th
        // distinct id has no label. Wrapping would silently alias two districts.
        let assignment: Vec<u16> = (0..=u16::MAX).collect();
        let err = first_seen_relabel_assignment(&assignment).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("65535"));

        let mut runs: Vec<(u16, u16)> = (0..=u16::MAX).map(|v| (v, 1)).collect();
        let err = first_seen_relabel_rle(&mut runs).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    // ── first_seen_relabel_rle ─────────────────────────────────────────

    #[test]
    fn first_seen_relabel_rle_basic() {
        let mut runs = vec![(2u16, 3u16), (3, 1), (2, 2), (5, 1)];
        first_seen_relabel_rle(&mut runs).unwrap();
        assert_eq!(runs, vec![(1, 3), (2, 1), (1, 2), (3, 1)]);
    }

    /// Cross-check: assignment-level and RLE-level first-seen relabeling must agree for any input.
    /// This pins the equivalence as a property, not a coincidence (decision #6 / risk mitigation).
    #[test]
    fn first_seen_relabel_assignment_equals_rle_path() {
        let inputs: Vec<Vec<u16>> = vec![
            vec![],
            vec![7],
            vec![1, 1, 1, 1],
            vec![2, 3, 4, 5, 5, 3, 4, 2],
            vec![5, 4, 3, 2, 1],
            vec![1, 5, 9, 5, 1, 9, 9, 1],
            vec![3, 3, 1, 1, 2, 2, 3, 3, 4],
        ];
        for input in inputs {
            let from_assignment = first_seen_relabel_assignment(&input).unwrap();

            let mut runs = assign_to_rle(input.clone());
            first_seen_relabel_rle(&mut runs).unwrap();
            let from_rle = rle_to_vec(runs);

            assert_eq!(
                from_assignment, from_rle,
                "divergence on input: {:?}",
                input
            );
        }
    }

    // ── permute_assignment ─────────────────────────────────────────────

    #[test]
    fn permute_assignment_identity() {
        let assignment = vec![10u16, 20, 30];
        let perm = vec![0, 1, 2];
        assert_eq!(
            permute_assignment(&assignment, &perm).unwrap(),
            vec![10, 20, 30]
        );
    }

    #[test]
    fn permute_assignment_reversal() {
        let assignment = vec![10u16, 20, 30];
        let perm = vec![2, 1, 0];
        assert_eq!(
            permute_assignment(&assignment, &perm).unwrap(),
            vec![30, 20, 10]
        );
    }

    #[test]
    fn permute_assignment_length_mismatch() {
        let assignment = vec![1u16, 2, 3];
        let perm = vec![0, 1];
        let err = permute_assignment(&assignment, &perm).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("length"));
    }

    #[test]
    fn permute_assignment_old_index_out_of_range() {
        let assignment = vec![1u16, 2, 3];
        let perm = vec![0, 1, 99];
        let err = permute_assignment(&assignment, &perm).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("old index"));
    }
}
