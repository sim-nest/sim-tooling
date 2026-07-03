//! Binary entry point for the xtask tool.

#![forbid(unsafe_code)]

fn main() {
    if let Err(err) = xtask::run(std::env::args().collect()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
