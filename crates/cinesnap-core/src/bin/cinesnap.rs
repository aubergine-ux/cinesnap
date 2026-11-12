//! `cinesnap-cli`: the Cinesnap command line without the desktop app.

fn main() -> anyhow::Result<()> {
    cinesnap_core::cli::run(std::env::args_os())
}
