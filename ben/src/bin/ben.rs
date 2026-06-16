/// Entry point for the `ben` CLI binary.
fn main() {
    if let Err(err) = binary_ensemble::cli::ben::run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
