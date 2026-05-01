/// Entry point for the `pben` CLI binary.
fn main() {
    if let Err(err) = binary_ensemble::cli::pben::run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
