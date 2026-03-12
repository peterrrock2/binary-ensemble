//! Utility functions for run-length encoding assignment vectors.

/// Convert a vector of assignments to a run-length encoded (RLE) vector.
pub fn assign_to_rle(assign_vec: Vec<u16>) -> Vec<(u16, u16)> {
    let mut prev_assign: u16 = 0;
    let mut count: u16 = 0;
    let mut first = true;
    let mut rle_vec: Vec<(u16, u16)> = Vec::new();

    for assign in assign_vec {
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
    rle_vec
}

/// Convert a run-length encoded (RLE) vector to a vector of assignments.
pub fn rle_to_vec(rle_vec: Vec<(u16, u16)>) -> Vec<u16> {
    let mut output_vec: Vec<u16> = Vec::new();
    for (val, len) in rle_vec {
        for _ in 0..len {
            output_vec.push(val);
        }
    }
    output_vec
}

#[cfg(test)]
mod tests;
