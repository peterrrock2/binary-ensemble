use pyo3::prelude::*;
use pyo3::wrap_pyfunction; // <-- needed for wrap_pyfunction!

pub mod decode;
pub mod encode;

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Export classes
    m.add_class::<crate::encode::PyBenEncoder>()?;
    m.add_class::<crate::decode::PyBenDecoder>()?;
    m.add_function(wrap_pyfunction!(crate::decode::decompress_ben_to_jsonl, m)?)?;
    m.add_function(wrap_pyfunction!(crate::decode::decompress_xben_to_ben, m)?)?;
    m.add_function(wrap_pyfunction!(
        crate::decode::decompress_xben_to_jsonl,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(crate::encode::compress_jsonl_to_ben, m)?)?;
    m.add_function(wrap_pyfunction!(crate::encode::compress_jsonl_to_xben, m)?)?;
    m.add_function(wrap_pyfunction!(crate::encode::compress_ben_to_xben, m)?)?;

    Ok(())
}
