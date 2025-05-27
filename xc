warning: virtual workspace defaulting to `resolver = "1"` despite one or more workspace members being on edition 2021 which implies `resolver = "2"`
note: to keep the current resolver, specify `workspace.resolver = "1"` in the workspace root's manifest
note: to use the edition 2021 resolver, specify `workspace.resolver = "2"` in the workspace root's manifest
note: for more details see https://doc.rust-lang.org/cargo/reference/resolver.html#resolver-versions
   Compiling pyo3 v0.25.0
   Compiling binary-ensemble-python v0.1.0 (/mnt/efs/h/Dropbox/MADLAB/Git_Repos/peter/binary-ensemble/pyben)
error[E0277]: the trait bound `Result<File, PyErr>: std::io::Read` is not satisfied
  --> pyben/src/lib.rs:19:37
   |
19 |         let reader = BufReader::new(file);
   |                      -------------- ^^^^ the trait `std::io::Read` is not implemented for `Result<File, PyErr>`
   |                      |
   |                      required by a bound introduced by this call
   |
note: required by a bound in `BufReader::<R>::new`
  --> /usr/src/debug/rust/rustc-1.86.0-src/library/std/src/io/buffered/bufreader.rs:73:5

error[E0308]: mismatched types
  --> pyben/src/lib.rs:20:36
   |
20 |         Ok(PyBenDecoder { decoder: reader })
   |                                    ^^^^^^ expected `BenDecoder<BufReader<File>>`, found `BufReader<Result<File, PyErr>>`
   |
   = note: expected struct `BenDecoder<BufReader<File>>`
              found struct `BufReader<Result<File, PyErr>>`

error[E0599]: no method named `add_class` found for reference `&PyModule` in the current scope
  --> pyben/src/lib.rs:41:7
   |
41 |     m.add_class::<PyBenDecoder>()?;
   |       ^^^^^^^^^ method not found in `&PyModule`

error[E0277]: the trait bound `&PyModule: From<BoundRef<'_, '_, PyModule>>` is not satisfied
  --> pyben/src/lib.rs:39:1
   |
39 | #[pymodule]
   | ^^^^^^^^^^^ the trait `From<BoundRef<'_, '_, PyModule>>` is not implemented for `&PyModule`
   |
   = note: required for `BoundRef<'_, '_, PyModule>` to implement `Into<&PyModule>`
   = note: this error originates in the attribute macro `pymodule` (in Nightly builds, run with -Z macro-backtrace for more info)

Some errors have detailed explanations: E0277, E0308, E0599.
For more information about an error, try `rustc --explain E0277`.
error: could not compile `binary-ensemble-python` (lib) due to 4 previous errors
