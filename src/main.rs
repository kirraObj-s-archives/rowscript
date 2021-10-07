use clap::{AppSettings, Clap};
use rowscript_compiler as compiler;
use std::io::Read;
use std::{fs, io};

#[derive(Clap)]
#[clap(setting = AppSettings::ColoredHelp, about = "RowScript programming language")]
struct Cli {
    #[clap(subcommand)]
    sub: Cmd,
}

#[derive(Clap)]
enum Cmd {
    #[clap(about = "Build source files")]
    Build(Build),
}

#[derive(Clap)]
struct Build {
    #[clap(required = true, index = 1, about = "Input source file")]
    file: String,
}

impl Build {
    fn build_file_or_stdin(self) -> String {
        let f = &self.file;
        match f.as_str() {
            "-" => {
                let mut buf = String::new();
                io::stdin()
                    .read_to_string(&mut buf)
                    .expect("read stdin error");
                buf
            }
            _ => fs::read_to_string(f).expect(&format!("read file error: '{}'", f)),
        }
    }
}

fn main() {
    match Cli::parse().sub {
        Cmd::Build(cmd) => compiler::build(cmd.build_file_or_stdin()),
    }
}
