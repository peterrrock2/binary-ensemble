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

    // Create submodule "read"
    let read = pyo3::types::PyModule::new(m.py(), "read")?; // <-- new()
    read.add_function(wrap_pyfunction!(
        crate::decode::read::read_single_assignment,
        &read
    )?)?;

    // Attach as attribute and submodule so both `pyben.read` and `from pyben.read ...` work
    m.add_submodule(&read)?; // <-- pass by reference
    m.py()
        .import("sys")?
        .getattr("modules")?
        .set_item("pyben.read", read)?;

    Ok(())
}
