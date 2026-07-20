use pyo3::prelude::*;
use pyo3::wrap_pyfunction;

pub mod common;
pub mod compact;
pub mod decode;
pub mod encode;
pub mod graph;
pub mod recompress;
pub mod relabel;

#[pymodule(gil_used = true)]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<crate::encode::PyBenEncoder>()?;
    m.add_class::<crate::encode::PyBendlEncoder>()?;
    m.add_class::<crate::encode::PyBendlStreamSession>()?;
    m.add_class::<crate::decode::PyBenDecoder>()?;
    m.add_class::<crate::decode::PyBendlDecoder>()?;
    m.add_function(wrap_pyfunction!(crate::decode::decode_ben_to_jsonl, m)?)?;
    m.add_function(wrap_pyfunction!(crate::decode::decode_xben_to_ben, m)?)?;
    m.add_function(wrap_pyfunction!(crate::decode::decode_xben_to_jsonl, m)?)?;
    m.add_function(wrap_pyfunction!(crate::encode::encode_jsonl_to_ben, m)?)?;
    m.add_function(wrap_pyfunction!(crate::encode::encode_jsonl_to_xben, m)?)?;
    m.add_function(wrap_pyfunction!(crate::encode::encode_ben_to_xben, m)?)?;
    m.add_function(wrap_pyfunction!(crate::graph::graph_reorder, m)?)?;
    m.add_function(wrap_pyfunction!(crate::compact::compact_bundle, m)?)?;
    m.add_function(wrap_pyfunction!(
        crate::compact::compact_bundle_in_place,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(crate::recompress::recompress_bundle, m)?)?;
    m.add_function(wrap_pyfunction!(crate::recompress::decompress_bundle, m)?)?;
    m.add_function(wrap_pyfunction!(crate::relabel::relabel_bundle, m)?)?;

    Ok(())
}
