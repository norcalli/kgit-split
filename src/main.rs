use anyhow::{Context, Result};
use itertools::Itertools;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::PathBuf;
use std::process::Command;

// yes n | git reset -p | luajit -e 'a = io.read"*a"; for x in a:gmatch [[@@ %-%d+,%d+ %+%d+,%d+ @@(.-)%(%d+/%d+%) Unstage this hunk [^?]+%?]] do print(("%q"):format(x)) end'

#[derive(Copy, Clone, Debug, parse_display::Display, parse_display::FromStr)]
#[display(style = "snake_case")]
enum Mode {
    Initial,
    RebaseTodo,
    HunkSplit,
}

impl Default for Mode {
    fn default() -> Self {
        Self::Initial
    }
}

#[derive(Clone, Debug)]
enum Opts {
    Initial { base: String, commit: String },
    RebaseTodo { commit: String, todo: PathBuf },
    HunkSplit { commit: String },
}

#[macro_export]
macro_rules! env_vars {
    (@munch ();) => {};
    (@munch (@$k:ident; $($it:tt)*);) => {
        const $k: &str = concat!("KGIT_SPLIT_", stringify!($k));
        $crate::env_vars! {
            @munch ($($it)*);
        }
    };
    (@munch ($k:ident; $($it:tt)*);) => {
        const $k: &str = stringify!($k);
        $crate::env_vars! {
            @munch ($($it)*);
        }
    };
    (@munch (; $($it:tt)*);) => {
        $crate::env_vars! {
            @munch ($($it)*);
        };
    };

    ($($it:tt)*) => {
        $crate::env_vars! {
            @munch ($($it)*);
        }
    };
}

env_vars! {
    @ENV_TARGET_COMMIT;
    @ENV_MERGE_BASE;
    @ENV_MODE;
    GIT_SEQUENCE_EDITOR;
}

fn initial_opts() -> Opts {
    use bpaf::*;
    let base = short('b')
        .long("base")
        .help("Merge base")
        .argument("BASE")
        .fallback("origin/master".to_string());

    let commit = positional("TARGET_COMMIT").parse(|s| rev_parse(&s));
    // let commit = positional("TARGET_COMMIT").guard(
    //     |s| s.chars().count() >= 7,
    //     // |s| s.chars().all(|c| c.is_ascii_hexdigit()) && s.chars().count() >= 7,
    //     "Commit must be a hexadecimal hash of len >= 7",
    // );

    let parser = construct!(Opts::Initial { base, commit });

    Info::default()
        .descr("Split a commit")
        .for_parser(parser)
        .run()
}

fn rebase_todo_opts() -> Opts {
    use bpaf::*;
    let todo = positional("REBASE_TODO")
        .from_str::<PathBuf>()
        .guard(|f| f.exists(), "Path must exist");
    let commit = long("--commit")
        .argument("COMMIT")
        .fallback_with(|| std::env::var(ENV_TARGET_COMMIT));
    let parser = construct!(Opts::RebaseTodo { commit, todo });
    Info::default().for_parser(parser).run()
}

fn hunk_split_opts() -> Opts {
    use bpaf::*;
    let commit = long("--commit").argument("COMMIT");
    let parser = construct!(Opts::HunkSplit { commit });
    Info::default().for_parser(parser).run()
}

fn opts() -> Opts {
    use bpaf::*;
    let mode = long("--mode")
        .argument("MODE")
        .fallback_with(|| std::env::var(ENV_MODE))
        .from_str::<Mode>()
        .default()
        .hide();
    let mode = Info::default().for_parser(mode).run();
    // let mode = std::env::var(ENV_MODE)
    //     .map(|m| m.parse::<Mode>().with_context(|| format!("{m:?}")).unwrap())
    //     .unwrap_or_default();
    match mode {
        Mode::Initial => initial_opts(),
        Mode::RebaseTodo => rebase_todo_opts(),
        Mode::HunkSplit => hunk_split_opts(),
    }
}

fn vec_to_utf8(s: Vec<u8>) -> String {
    String::from_utf8(s).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn rev_parse(input: impl AsRef<str>) -> Result<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg(input.as_ref())
        .output()?;
    anyhow::ensure!(output.status.success(), "{}", vec_to_utf8(output.stderr));
    let mut s = vec_to_utf8(output.stdout);
    s.truncate(s.trim_end().len());
    Ok(s)
}

fn main() -> Result<()> {
    let opts = opts();
    match opts {
        Opts::Initial { base, commit } => {
            // let prev_commit = rev_parse(&format!("{commit}~"))
            //     .with_context(|| format!("Failed to find previous commit of {commit:?}. \
            //                              This can happen if the previous commit is the first commit."))?;
            // let rebase_commit = &prev_commit;
            let rebase_commit = &commit;
            let exe = std::env::current_exe()?.display().to_string();
            anyhow::ensure!(Command::new("git")
                .args(&["rebase", "-i", rebase_commit])
                .env(GIT_SEQUENCE_EDITOR, &exe)
                .env(ENV_TARGET_COMMIT, &commit)
                // .env(ENV_MERGE_BASE, &opts.base)
                .env(ENV_MODE, Mode::RebaseTodo.to_string())
                .status()?
                .success());
        }
        Opts::RebaseTodo { todo, commit } => {
            let todo = std::fs::read_to_string(&todo)?;
            let rebase_commands = todo
                .split('\n')
                .filter(|line| !(line.starts_with('#') || line.is_empty()))
                .collect::<Vec<_>>();
            let exe = std::env::current_exe()?.display().to_string();
            eprintln!("{commit:?} {rebase_commands:#?}");
            let hunk_mode = Mode::HunkSplit;
            std::fs::write(
                &todo,
                &format!(
                    "x {exe:?} --mode {hunk_mode:?} --commit {commit:?}\n{}",
                    rebase_commands.iter().format("\n")
                ),
            )?;
            // std::fs::write(&format!("x sh {ENV_MODE}={hunk_mode:?} {exe:?} {}",
            // assert!(commit.starts_with(rebase_commands[0].split(' ').nth(1).unwrap()));
        }
        Opts::HunkSplit { commit } => {
            // git revert --no-commit HEAD
            // Command::new("sh")
            //     .arg("yes n | git reset -p")
        }
    }
    Ok(())
}
