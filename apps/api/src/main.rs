use std::process::ExitCode;

fn main() -> ExitCode {
    // `std::env::args` panics on an argument that is not Unicode; read lossily, it is just a
    // command line the binary does not know.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    barras_api::cli::main_with_args(&args)
}
