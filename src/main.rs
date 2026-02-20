use clap::Parser;
use kitsune_livewallpaper::cli::Cli;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    kitsune_livewallpaper::run(cli)
}
