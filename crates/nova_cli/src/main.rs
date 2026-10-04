fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(nova_cli::run_cli(&args));
}
