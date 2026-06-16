use super::*;

#[test]
fn test_assign_to_rle() {
    let assign_vec: Vec<u16> = vec![1, 1, 1, 2, 2, 3];

    let result: Vec<(u16, u16)> = vec![(1, 3), (2, 2), (3, 1)];

    assert_eq!(assign_to_rle(assign_vec), result);
}

#[test]
fn test_rle_to_vec() {
    let rle_vec: Vec<(u16, u16)> = vec![(1, 3), (2, 2), (3, 1)];

    let result: Vec<u16> = vec![1, 1, 1, 2, 2, 3];

    assert_eq!(rle_to_vec(rle_vec), result);
}
