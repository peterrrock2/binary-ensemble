/// Entry point for the `bendl` CLI binary.
fn main() {
    if let Err(err) = binary_ensemble::cli::bendl::run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
