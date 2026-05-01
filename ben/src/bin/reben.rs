/// Entry point for the `reben` CLI binary.
fn main() {
    if let Err(err) = binary_ensemble::cli::reben::run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
