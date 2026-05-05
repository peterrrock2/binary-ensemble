/// Entry point for the `pcben` CLI binary.
fn main() {
    if let Err(err) = binary_ensemble::cli::pcben::run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
