#![forbid(unsafe_code)]

mod cli;
mod platform;

fn main() {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
        }
        Ok(cli::Action::Run(command)) => match platform::run(command) {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("hebrew-tty: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("hebrew-tty: {error}\n{}", cli::USAGE);
            std::process::exit(2);
        }
    }
}
