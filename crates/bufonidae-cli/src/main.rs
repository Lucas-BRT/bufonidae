use clap::Parser;

/// Pool several Mega.nz accounts into one logical directory, via rclone.
#[derive(Parser)]
#[command(version, about)]
struct Cli {}

fn main() {
    Cli::parse();
}
