mod hash;
use hash::*;
mod fmt;
use fmt::*;

use anyhow::{Context, Result};
use itertools::Itertools;
use regex::Regex;
use std::io::{stdout, Write};
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
    Initial { commit: String },
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
    // @ENV_MERGE_BASE;
    @ENV_MODE;
    GIT_SEQUENCE_EDITOR;
}

fn initial_opts() -> Opts {
    use bpaf::*;
    // let base = short('b')
    //     .long("base")
    //     .help("Merge base")
    //     .argument("BASE")
    //     .fallback("origin/master".to_string());

    let commit = positional("TARGET_COMMIT").parse(|s| rev_parse(&s));

    let parser = construct!(Opts::Initial { commit });

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
    let commit = long("commit")
        .argument("COMMIT")
        .fallback_with(|| std::env::var(ENV_TARGET_COMMIT));
    let parser = construct!(Opts::RebaseTodo { commit, todo });
    Info::default().for_parser(parser).run()
}

fn hunk_split_opts() -> Opts {
    use bpaf::*;
    let commit = long("commit").argument("COMMIT");
    let parser = construct!(Opts::HunkSplit { commit });
    Info::default().for_parser(parser).run()
}

fn opts() -> Opts {
    // let mode: Parser<Mode> = long("mode")
    //     .argument("MODE")
    //     .fallback_with(|| std::env::var(ENV_MODE))
    //     .from_str::<Mode>()
    //     .default()
    //     .hide();
    // let mode = Info::default().for_parser(mode).run();
    let mode = std::env::var(ENV_MODE)
        .map(|m| m.parse::<Mode>().with_context(|| format!("{m:?}")).unwrap())
        .unwrap_or_default();
    match mode {
        Mode::Initial => initial_opts(),
        Mode::RebaseTodo => rebase_todo_opts(),
        Mode::HunkSplit => hunk_split_opts(),
    }
}

fn vec_to_utf8(s: Vec<u8>) -> String {
    String::from_utf8(s).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn get_output(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    anyhow::ensure!(
        output.status.success(),
        "Failed to run {command:?}:\n{}",
        vec_to_utf8(output.stderr)
    );
    let mut s = vec_to_utf8(output.stdout);
    s.truncate(s.trim_end().len());
    Ok(s)
}

fn get_output_with_input(
    command: &mut Command,
    input: impl FnOnce(&mut std::process::ChildStdin) -> Result<()>,
) -> Result<String> {
    command.stdin(std::process::Stdio::piped());
    let mut child = command.spawn()?;
    {
        let child_stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No stdin?"))?;
        input(child_stdin)?;
    }
    let output = child.wait_with_output()?;
    anyhow::ensure!(
        output.status.success(),
        "Failed to run {command:?}:\n{}",
        vec_to_utf8(output.stderr)
    );
    let mut s = vec_to_utf8(output.stdout);
    s.truncate(s.trim_end().len());
    Ok(s)
}

fn rev_parse(input: impl AsRef<str>) -> Result<String> {
    get_output(Command::new("git").arg("rev-parse").arg(input.as_ref()))

    //     let output = Command::new("git")
    //         .arg("rev-parse")
    //         .arg(input.as_ref())
    //         .output()?;
    //     anyhow::ensure!(output.status.success(), "{}", vec_to_utf8(output.stderr));
    //     let mut s = vec_to_utf8(output.stdout);
    //     s.truncate(s.trim_end().len());
    //     Ok(s)
}

// TODO git log the commit directly and do the parsing of the hunks before
// initiating a rebase at all.
// Then do the revert commit and then create all the hunks afterwards, except
// without the reverse flag.
fn main() -> Result<()> {
    let opts = opts();
    match opts {
        Opts::Initial { commit } => {
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
            let raw_todo = std::fs::read_to_string(&todo)?;
            let rebase_commands = raw_todo
                .split('\n')
                .filter(|line| !(line.starts_with('#') || line.is_empty()))
                .collect::<Vec<_>>();
            let exe = std::env::current_exe()?.display().to_string();
            log::debug!("{commit:?} {rebase_commands:#?}");
            let hunk_mode = Mode::HunkSplit;
            std::fs::write(
                &todo,
                &format!(
                    "x env {ENV_MODE}={hunk_mode} {exe:?} --commit {commit:?}\n{}",
                    rebase_commands.iter().format("\n")
                ),
            )
            .with_context(|| format!("Failed to write to {todo:?}", todo = todo.display()))?;
            // std::fs::write(&format!("x sh {ENV_MODE}={hunk_mode:?} {exe:?} {}",
            // assert!(commit.starts_with(rebase_commands[0].split(' ').nth(1).unwrap()));
        }
        Opts::HunkSplit { commit } => {
            log::debug!("hunk splitting {commit:?}");
            get_output(Command::new("git").args(&["revert", "--no-commit", &commit]))?;
            // Command::new("git")
            //     .args(&["revert", "--no-commit", "HEAD"])
            //     .output()?
            //     .success()
            //     .then(|| )
            //     .ok_or_else(|| anyhow!(
            //     ;
            let raw_hunks = get_output(Command::new("sh").arg("-c").arg("yes n | git reset -p"))?;
            log::debug!("{raw_hunks}\n\n");
            let regex = Regex::new(
                // r#"(diff .+\nindex.+\n\-\-\-.+\n\+\+\+.+)(?:(?s)(@@ \-\d+,\d+ \+\d+,\d+ @@(.+?)\(\d+/\d+\) Unstage [^.]+?\? )+)"#,
                // r#"(diff .+\nindex.+\n\-\-\-.+\n\+\+\+.+)|(?s)(@@ \-\d+,\d+ \+\d+,\d+ @@.+?)\(\d+/\d+\) Unstage [^.]+?\? "#,
                // r#"(diff .+\nindex.+(?:\n[^@][^@].+)+)|(?s)(@@ \-\d+,\d+ \+\d+,\d+ @@)(.+?)\(\d+/\d+\) Unstage [^.]+?\? "#,
                r#"(diff .+(?:\n[^@][^@].+)+)|(?s)(@@ \-\d+,\d+ \+\d+,\d+ @@.+?)\(\d+/\d+\) Unstage [^.]+?\? "#,
            )?;
            let mut files = Vec::new();
            for cap in regex.captures_iter(&raw_hunks) {
                if let Some(header) = cap.get(1) {
                    let header = header.as_str();
                    log::debug!("File: {header:?}");
                    files.push((header, Vec::new()));
                }
                if let Some(hunk) = cap.get(2) {
                    let hunk = hunk.as_str();
                    files.last_mut().unwrap().1.push(hunk);
                    // files.last_mut().unwrap().1.push((hunk, cap.get(3).unwrap().as_str()));
                }
            }
            // for (header, hunks) in files.iter() {
            //     println!("{header}");
            //     for hunk_body in hunks.iter() {
            //         print!("{hunk_body}");
            //     }
            //     // for (hunk_header, hunk_body) in hunks.iter() {
            //     //     println!("{hunk_header}");
            //     //     println!("{hunk_body}");
            //     // }
            // }
            get_output(Command::new("git").args(&[
                "commit",
                "-m",
                &format!("Revert {commit} for split"),
            ]))?;
            type CommitId = char;
            #[derive(Hash, Debug)]
            enum UiMode {
                Editing {
                    commit: CommitId,
                    message: Vec<char>,
                    assign_to_hunk: Option<usize>,
                },
                Viewing {
                    active_hunk: usize,
                },
                WaitingToEdit,
            }

            impl Default for UiMode {
                fn default() -> Self {
                    Self::Viewing { active_hunk: 0 }
                }
            }

            use std::collections::BTreeMap;
            #[derive(Hash)]
            struct CommitInfo {
                commit_message: String,
            }

            let hunks = files
                .iter()
                .flat_map(|(header, hunks)| hunks.iter().map(move |hunk| (header, hunk)))
                .collect_vec();
            assert!(!hunks.is_empty());

            #[derive(Hash, Default)]
            struct UiState {
                active_mode: UiMode,
                previous_modes: Vec<UiMode>,
                messages: BTreeMap<CommitId, CommitInfo>,
                hunk_commits: Vec<Option<CommitId>>,
            }

            impl UiState {
                pub fn push_mode(&mut self, mode: UiMode) {
                    self.previous_modes
                        .push(std::mem::replace(&mut self.active_mode, mode));
                }

                pub fn set_mode(&mut self, mode: UiMode) -> UiMode {
                    std::mem::replace(&mut self.active_mode, mode)
                }

                pub fn pop_mode(&mut self) -> Option<UiMode> {
                    if let Some(mode) = self.previous_modes.pop() {
                        Some(self.set_mode(mode))
                    } else {
                        None
                    }
                }

                pub fn all_hunks_assigned(&self, hunk_count: usize) -> bool {
                    self.hunk_commits.len() == hunk_count
                        && self.hunk_commits.iter().all(|c| c.is_some())
                }

                pub fn set_hunk_commit(&mut self, hunk_idx: usize, commit_id: CommitId) {
                    if self.hunk_commits.len() <= hunk_idx {
                        self.hunk_commits
                            .resize_with(hunk_idx + 1, Default::default);
                    }
                    self.hunk_commits[hunk_idx] = Some(commit_id);
                }
            }
            use std::hash::Hash;
            use termion::{input::TermRead, raw::IntoRawMode};
            trait GetHash: Hash {
                fn meow_hash(&self) -> u128 {
                    meow_hash(None, &self)
                }
            }
            impl<H: Hash> GetHash for H {}
            {
                let mut commit_colors_seq = [
                    termion::color::Fg(termion::color::LightRed).to_string(),
                    termion::color::Fg(termion::color::LightYellow).to_string(),
                    termion::color::Fg(termion::color::LightBlue).to_string(),
                    termion::color::Fg(termion::color::LightGreen).to_string(),
                    termion::color::Fg(termion::color::LightCyan).to_string(),
                    termion::color::Fg(termion::color::LightMagenta).to_string(),
                    termion::color::Fg(termion::color::LightBlack).to_string(),
                    termion::color::Fg(termion::color::LightWhite).to_string(),
                    termion::color::Fg(termion::color::Blue).to_string(),
                    termion::color::Fg(termion::color::Red).to_string(),
                ]
                .into_iter();
                let mut commit_colors: BTreeMap<CommitId, Option<String>> = BTreeMap::new();
                let stdin = std::io::stdin();
                // let mut stdin = termion::async_stdin();
                let mut ui_state: UiState = Default::default();
                let mut prev_hash = 0;
                let stdout = stdout();
                let stdout = stdout.into_raw_mode()?;
                let mut screen = termion::screen::AlternateScreen::from(stdout);
                // let mut screen = stdout;
                let mut keys = stdin.keys();
                let mut draw_buffer = String::new();
                let mut out_buffer = String::new();
                use std::fmt::Write;
                'ui_loop: loop {
                    let should_redraw = {
                        let hash = ui_state.meow_hash();
                        if hash != prev_hash {
                            prev_hash = hash;
                            true
                        } else {
                            false
                        }
                    };
                    if should_redraw {
                        let terminal_height = termion::terminal_size().unwrap_or_default().1;
                        write!(
                            draw_buffer,
                            "{}{}",
                            termion::cursor::Goto(1, 1),
                            termion::clear::All
                        )?;
                        for (id, CommitInfo { commit_message }) in ui_state.messages.iter() {
                            writeln!(
                                draw_buffer,
                                "{color}{id}: {commit_message}{reset}",
                                commit_message = commit_message.split('\n').next().unwrap(),
                                color = commit_colors
                                    .entry(*id)
                                    .or_insert_with(|| commit_colors_seq.next())
                                    .or_display(""),
                                reset = termion::color::Fg(termion::color::Reset),
                            )?;
                        }
                        match &ui_state.active_mode {
                            UiMode::WaitingToEdit => {
                                writeln!(draw_buffer, "Enter mode id to edit...",)?;
                            }
                            UiMode::Editing {
                                commit,
                                message,
                                assign_to_hunk: _,
                            } => {
                                writeln!(
                                    draw_buffer,
                                    "For {color}{commit}{reset}:\n{message}",
                                    message = message.iter().format(""),
                                    color = commit_colors
                                        .entry(*commit)
                                        .or_insert_with(|| commit_colors_seq.next())
                                        .or_display(""),
                                    reset = termion::color::Fg(termion::color::Reset),
                                )?;
                            }
                            UiMode::Viewing { active_hunk } => {
                                let (header, hunk) = &hunks[*active_hunk];
                                let commit =
                                    ui_state.hunk_commits.get(*active_hunk).copied().flatten();
                                let commit_message = commit
                                    .map(|commit| {
                                        ui_state.messages[&commit]
                                            .commit_message
                                            .split('\n')
                                            .next()
                                            .unwrap()
                                    })
                                    .into_or_display("---");
                                let header_line = header.split('\n').next().unwrap();
                                writeln!(
                                    draw_buffer,
                                    "{active_hunk}/{n}: {color}{commit_message}{reset}\n{header_line}",
                                    n = hunks.len(),
                                    color = commit.and_then(|commit| commit_colors
                                        .entry(commit)
                                        .or_insert_with(|| commit_colors_seq.next()).as_ref())
                                        .into_or_display(""),
                                    reset = termion::color::Fg(termion::color::Reset),
                                )?;
                                for line in hunk.split('\n').take(
                                    (terminal_height as usize)
                                        .saturating_sub(ui_state.messages.len() + 5),
                                ) {
                                    if let Some(line) = line.strip_prefix("+") {
                                        writeln!(
                                            draw_buffer,
                                            "{color}-{line}{reset}",
                                            color = termion::color::Fg(termion::color::Red),
                                            reset = termion::color::Fg(termion::color::Reset),
                                        )?;
                                    } else if let Some(line) = line.strip_prefix("-") {
                                        writeln!(
                                            draw_buffer,
                                            "{color}+{line}{reset}",
                                            color = termion::color::Fg(termion::color::Green),
                                            reset = termion::color::Fg(termion::color::Reset),
                                        )?;
                                    } else {
                                        writeln!(draw_buffer, "{line}")?;
                                    }
                                }
                            }
                        }

                        for c in draw_buffer.drain(..) {
                            if c == '\n' {
                                out_buffer.push_str("\r\n");
                            } else {
                                out_buffer.push(c);
                            }
                        }

                        screen.write_all(out_buffer.as_bytes())?;
                        screen.flush()?;
                    }
                    let key = if let Some(key) = keys.next() {
                        key?
                    } else {
                        break 'ui_loop;
                    };
                    match &ui_state.active_mode {
                        UiMode::WaitingToEdit => {
                            ui_state.pop_mode();
                            match key {
                                termion::event::Key::Char(c @ '0'..='9') => {
                                    let mode = UiMode::Editing {
                                        commit: c,
                                        message: ui_state
                                            .messages
                                            .get(&c)
                                            .map(|info| info.commit_message.chars().collect_vec())
                                            .unwrap_or_default(),
                                        assign_to_hunk: None,
                                    };
                                    ui_state.push_mode(mode);
                                }
                                _ => (),
                            }
                        }
                        UiMode::Editing { .. } => match key {
                            termion::event::Key::Char(c) => match &mut ui_state.active_mode {
                                UiMode::Editing {
                                    ref mut message, ..
                                } => {
                                    message.push(c);
                                }
                                _ => unreachable!(),
                            },
                            termion::event::Key::Backspace => match &mut ui_state.active_mode {
                                UiMode::Editing {
                                    ref mut message, ..
                                } => {
                                    message.pop();
                                }
                                _ => unreachable!(),
                            },
                            termion::event::Key::Esc => {
                                ui_state.pop_mode();
                            }
                            termion::event::Key::Ctrl('s') => match ui_state.pop_mode() {
                                Some(UiMode::Editing {
                                    commit,
                                    message,
                                    assign_to_hunk,
                                }) => {
                                    ui_state.messages.insert(
                                        commit,
                                        CommitInfo {
                                            commit_message: String::from_iter(message),
                                        },
                                    );
                                    if let Some(assign_to_hunk) = assign_to_hunk {
                                        ui_state.set_hunk_commit(assign_to_hunk, commit);
                                    }
                                }
                                _ => unreachable!(),
                            },
                            _ => (),
                        },
                        UiMode::Viewing { active_hunk } => {
                            let active_hunk = *active_hunk;
                            match key {
                                termion::event::Key::Ctrl('s')
                                    if ui_state.all_hunks_assigned(hunks.len()) =>
                                {
                                    break 'ui_loop;
                                }
                                termion::event::Key::Char('q') => {
                                    break 'ui_loop;
                                }
                                termion::event::Key::Char(c @ '0'..='9') => {
                                    if !ui_state.messages.contains_key(&c) {
                                        let mode = UiMode::Editing {
                                            commit: c,
                                            message: Default::default(),
                                            assign_to_hunk: Some(active_hunk),
                                        };
                                        ui_state.push_mode(mode);
                                    } else {
                                        ui_state.set_hunk_commit(active_hunk, c);
                                    }
                                }
                                termion::event::Key::Backspace => {
                                    ui_state.push_mode(UiMode::WaitingToEdit);
                                }
                                termion::event::Key::Left => {
                                    let mode = UiMode::Viewing {
                                        active_hunk: active_hunk.saturating_sub(1),
                                    };
                                    ui_state.set_mode(mode);
                                }
                                termion::event::Key::Right => {
                                    if (active_hunk + 1) < hunks.len() {
                                        let mode = UiMode::Viewing {
                                            active_hunk: active_hunk + 1,
                                        };
                                        ui_state.set_mode(mode);
                                    }
                                }
                                _ => (),
                            }
                        }
                    }
                } // 'ui_loop
                if ui_state.all_hunks_assigned(hunks.len()) {
                    let hunks_for_commit: BTreeMap<CommitId, Vec<usize>> = ui_state
                        .hunk_commits
                        .into_iter()
                        .map(|id| id.expect("Already checked in all_hunks_assigned"))
                        .enumerate()
                        .map(|(hunk_id, commit_id)| (commit_id, hunk_id))
                        .into_group_map()
                        .into_iter()
                        .collect();
                    for (commit_id, hunk_ids) in hunks_for_commit.into_iter() {
                        let commit_info = &ui_state.messages[&commit_id];
                        let output = get_output_with_input(
                            Command::new("patch")
                                .args(&["-p1", "-R"])
                                .stderr(std::process::Stdio::piped())
                                .stdout(std::process::Stdio::piped()),
                            |stdin| {
                                for hunk_id in hunk_ids.into_iter() {
                                    let (header, hunk_body) = &hunks[hunk_id];
                                    write!(stdin, "{header}\n{hunk_body}")?;
                                }
                                Ok(())
                            },
                        )?;
                        for line in output.split('\n') {
                            if let Some(file) = line.strip_prefix("patching file ") {
                                let file = file.trim_end();
                                dbg!(file);
                                get_output(Command::new("git").args(&["add", file]))?;
                            }
                        }
                        get_output(Command::new("git").args(&["add", "-u"]))?;
                        get_output(Command::new("git").args(&[
                            "commit",
                            "-m",
                            &commit_info.commit_message,
                        ]))?;
                    }
                    // get_output(
                    //     Command::new("git").args(&["rebase", "--continue"]),
                    // )?;
                } else {
                    get_output(Command::new("git").args(&["rebase", "--abort"]))?;
                    // anyhow::bail!("Not all hunks assigned");
                }
            }
        }
    }
    Ok(())
}
