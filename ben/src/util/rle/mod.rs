//! Utility functions for run-length encoding assignment vectors.

/// Convert a vector of assignments to a run-length encoded (RLE) vector.
///
/// # Arguments
///
/// * `assign_vec` - The full assignment vector.
///
/// # Returns
///
/// Returns the assignment vector as `(value, count)` pairs.
pub fn assign_to_rle(assign_vec: Vec<u16>) -> Vec<(u16, u16)> {
    let mut rle_vec: Vec<(u16, u16)> = Vec::new();
    assign_slice_to_rle(&assign_vec, &mut rle_vec);
    rle_vec
}

/// Convert a run-length encoded (RLE) vector to a vector of assignments.
///
/// # Arguments
///
/// * `rle_vec` - The run-length encoded assignment vector.
///
/// # Returns
///
/// Returns the expanded assignment vector.
pub fn rle_to_vec(rle_vec: Vec<(u16, u16)>) -> Vec<u16> {
    let mut output_vec: Vec<u16> = Vec::new();
    rle_to_vec_in_place(&rle_vec, &mut output_vec);
    output_vec
}

/// Expand an RLE vector into a provided output buffer.
///
/// # Arguments
///
/// * `rle_vec` - The run-length encoded assignment vector.
/// * `output_vec` - The buffer that will receive the expanded assignments.
///
/// # Returns
///
/// This function does not return a value.
pub(crate) fn rle_to_vec_in_place(rle_vec: &[(u16, u16)], output_vec: &mut Vec<u16>) {
    output_vec.clear();
    let total_len: usize = rle_vec.iter().map(|(_, len)| *len as usize).sum();
    if output_vec.capacity() < total_len {
        output_vec.reserve(total_len - output_vec.capacity());
    }
    for &(val, len) in rle_vec {
        for _ in 0..len {
            output_vec.push(val);
        }
    }
}

/// Encode an assignment slice into a provided RLE output buffer.
///
/// # Arguments
///
/// * `assign_vec` - The full assignment vector.
/// * `rle_vec` - The buffer that will receive `(value, count)` pairs.
///
/// # Returns
///
/// This function does not return a value.
pub(crate) fn assign_slice_to_rle(assign_vec: &[u16], rle_vec: &mut Vec<(u16, u16)>) {
    rle_vec.clear();
    let mut prev_assign: u16 = 0;
    let mut count: u16 = 0;
    let mut first = true;

    for &assign in assign_vec {
        if first {
            prev_assign = assign;
            count = 1;
            first = false;
            continue;
        }
        if assign == prev_assign {
            count += 1;
        } else {
            rle_vec.push((prev_assign, count));
            prev_assign = assign;
            count = 1;
        }
    }

    if count > 0 {
        rle_vec.push((prev_assign, count));
    }
}

#[cfg(test)]
mod tests;
