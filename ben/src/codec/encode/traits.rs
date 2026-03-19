pub trait FromRLE {
    fn from_rle(runs: Vec<(u16, u16)>, count: Option<u16>) -> Self;
}

pub trait FromAssign {
    fn from_assignment(assignments: impl AsRef<[u16]>, count: Option<u16>) -> Self;
}
