use clap::Subcommand;

#[derive(clap::Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Generate,
}

fn main() {
    let _args = <Args as clap::Parser>::parse();
}
