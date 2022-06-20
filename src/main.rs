use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use std::process::Command;

// yes n | git reset -p | luajit -e 'a = io.read"*a"; for x in a:gmatch [[@@ %-%d+,%d+ %+%d+,%d+ @@(.-)%(%d+/%d+%) Unstage this hunk [^?]+%?]] do print(("%q"):format(x)) end'

#[derive(Clone, Debug)]
struct Opts {
    base: String,
    commit: String,
}

fn opts() -> Opts {
    use bpaf::*;
    let base = short('b')
        .long("base")
        .help("Merge base")
        .argument("BASE")
        .fallback("origin/master".to_string());

    let commit = positional("TARGET_COMMIT").guard(
        |s| s.chars().all(|c| c.is_ascii_hexdigit()) && s.chars().count() > 9,
        "Commit must be a hexadecimal hash of len >= 9",
    );

    let parser = construct!(Opts { base, commit });

    Info::default()
        .descr("Split a commit")
        .for_parser(parser)
        .run()
}

fn main() -> Result<()> {
    let exe = std::env::current_exe()?.display().to_string();
    let opts = opts();
    anyhow::ensure!(Command::new("git")
        .args(&["rebase", "-i", &opts.base])
        .env("GIT_SEQUENCE_EDITOR", &exe)
        .status()?
        .success());
    // git revert --no-commit HEAD
    // Command::new("sh")
    //     .arg("yes n | git reset -p")
    println!("Hello, world! {opts:?}");
    Ok(())
}
