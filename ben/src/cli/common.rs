use std::io::{self, Result};
use std::path::Path;

pub fn set_verbose(verbose: bool) {
    if verbose {
        std::env::set_var("RUST_LOG", "trace");
    }
}

pub fn check_overwrite(file_name: &str, overwrite: bool) -> Result<()> {
    if Path::new(file_name).exists() && !overwrite {
        eprint!(
            "File {:?} already exists, do you want to overwrite it? (y/[n]): ",
            file_name
        );
        let mut user_input = String::new();
        io::stdin().read_line(&mut user_input).unwrap();
        eprintln!();
        if user_input.trim().to_lowercase() != "y" {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
    }
    Ok(())
}
