use std::env;
use std::process::ExitCode;

const USAGE: &str = "Usage: aethel [OPTIONS]

Options:
    -h, --help       Print help
    -V, --version    Print version";

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    let Some(argument) = arguments.next() else {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    };

    match argument.as_str() {
        "-h" | "--help" if arguments.next().is_none() => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        "-V" | "--version" if arguments.next().is_none() => {
            println!("aethel {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        value => {
            eprintln!("error: unexpected argument '{value}'");
            eprintln!("Try 'aethel --help' for usage information.");
            ExitCode::from(2)
        }
    }
}
