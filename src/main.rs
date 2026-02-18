use clap::Parser;
use kitsune_wallpaperengine::cli::Cli;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    kitsune_wallpaperengine::run(cli)
}
