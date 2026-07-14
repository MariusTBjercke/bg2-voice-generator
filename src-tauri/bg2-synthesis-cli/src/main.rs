use std::process::ExitCode;

fn main() -> ExitCode {
    bg2_voice_generator_lib::cli::run(std::env::args().skip(1).collect())
}
