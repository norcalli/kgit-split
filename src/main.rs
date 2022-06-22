mod hash;
use hash::*;
mod fmt;
use fmt::*;

use anyhow::{Context, Result};
use itertools::Itertools;
use regex::Regex;
use std::fmt::Write as _;
use std::io::{stdout, Write as _};
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
    @ENV_CONTEXT_SIZE;
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

fn git() -> Command {
    Command::new("git")
}

fn sh(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.args(&["-c", command]);
    cmd
}

fn get_output(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    anyhow::ensure!(
        output.status.success(),
        "Failed to run {command:?}:\n{}",
        vec_to_utf8(output.stderr)
    );
    Ok(vec_to_utf8(output.stdout))
}

fn get_output_with_input(
    command: &mut Command,
    input: impl FnOnce(&mut std::process::ChildStdin) -> Result<()>,
) -> Result<String> {
    command.stderr(std::process::Stdio::piped());
    command.stdout(std::process::Stdio::piped());
    let child = spawn_with_input(command, input)?;
    let output = child.wait_with_output()?;
    anyhow::ensure!(
        output.status.success(),
        "Failed to run {command:?}:\n{}",
        vec_to_utf8(output.stderr)
    );
    Ok(vec_to_utf8(output.stdout))
}

fn spawn_with_input(
    command: &mut Command,
    input: impl FnOnce(&mut std::process::ChildStdin) -> Result<()>,
) -> Result<std::process::Child> {
    command.stdin(std::process::Stdio::piped());
    let mut child = command.spawn()?;
    {
        let child_stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No stdin?"))?;
        input(child_stdin)?;
    }
    Ok(child)
}

trait StringExt {
    fn truncate_end(&mut self) -> &mut Self;
}
impl StringExt for String {
    fn truncate_end(&mut self) -> &mut Self {
        self.truncate(self.trim_end().len());
        self
    }
}

fn rev_parse(input: impl AsRef<str>) -> Result<String> {
    let mut s = get_output(git().arg("rev-parse").arg(input.as_ref()))?;
    s.truncate_end();
    Ok(s)
}

fn render_hunk(hunk: &str, max_lines: usize) -> impl std::fmt::Display + '_ {
    FmtFn(move |f| {
        for line in hunk.split('\n').take(max_lines) {
            if let Some(line) = line.strip_prefix("+") {
                writeln!(
                    f,
                    "{color}+{line}{reset}",
                    color = termion::color::Fg(termion::color::Green),
                    reset = termion::color::Fg(termion::color::Reset),
                )?;
            } else if let Some(line) = line.strip_prefix("-") {
                writeln!(
                    f,
                    "{color}-{line}{reset}",
                    color = termion::color::Fg(termion::color::Red),
                    reset = termion::color::Fg(termion::color::Reset),
                )?;
            } else {
                writeln!(f, "{line}")?;
            }
        }
        Ok(())
    })
}

fn edit(content: impl AsRef<[u8]>) -> Result<String> {
    let editor = std::env::var("EDITOR").or_else(|_| std::env::var("VISUAL"))?;
    let path: PathBuf = std::env::temp_dir().join(".gitsplit_edit");
    std::fs::write(&path, content)?;
    let mut child = sh(&format!("{editor} {path:?}", path = path.display())).spawn()?;
    child.wait()?;
    Ok(std::fs::read_to_string(&path)?)
}

// fn edit_or_original(content: impl std::fmt::Display) -> String {
//     let content = content.to_string();
//     edit(&content).unwrap_or(content)
// }

// TODO git log the commit directly and do the parsing of the hunks before
// initiating a rebase at all.
// Then do the revert commit and then create all the hunks afterwards, except
// without the reverse flag.
fn main() -> Result<()> {
    env_logger::init();
    let opts = opts();
    match opts {
        Opts::Initial { commit } => {
            // let prev_commit = rev_parse(&format!("{commit}~"))
            //     .with_context(|| format!("Failed to find previous commit of {commit:?}. \
            //                              This can happen if the previous commit is the first commit."))?;
            // let rebase_commit = &prev_commit;
            let rebase_commit = &commit;
            let exe = std::env::current_exe()?.display().to_string();
            anyhow::ensure!(git()
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
            let diff_context_size = std::env::var(ENV_CONTEXT_SIZE)
                .map_err(|err| anyhow::anyhow!("{err:?}"))
                .and_then(|s| Ok(s.parse::<usize>()?))
                .unwrap_or(1);
            let raw_hunks = get_output(git().args(&[
                "diff",
                "-p",
                &format!("-U{diff_context_size}"),
                &format!("{commit}~"),
                &commit,
            ]))?;
            let original_commit_message = get_output(git().args(&[
                "log",
                "--reverse",
                "--pretty=format:%B",
                &format!("{commit}~..{commit}"),
            ]))?;
            log::debug!("{raw_hunks:?}\n\n");
            let hunk_start_pat = Regex::new(r#"@@ \-\d+(?:,\d+)? \+\d+(?:,\d+)? @@"#)?;
            let diff_start_pat = Regex::new(r#"diff .*"#)?;
            let mut files = Vec::new();
            {
                enum ParseMode {
                    Hunk,
                    Header,
                }
                let mut lines = raw_hunks.split('\n');
                let mut header = diff_start_pat
                    .captures(lines.next().expect("diff was empty"))
                    .expect("First line should be diff in diff")[0]
                    .to_string();
                let mut hunks = Vec::new();
                let mut parse_mode = ParseMode::Header;

                for line in lines {
                    match parse_mode {
                        ParseMode::Hunk => {
                            if let Some(_cap) = diff_start_pat.captures(line) {
                                files.push((
                                    std::mem::take(&mut header),
                                    std::mem::take(&mut hunks),
                                ));
                                header.push_str(line);
                                parse_mode = ParseMode::Header;
                            } else if let Some(_cap) = hunk_start_pat.captures(line) {
                                hunks.push(line.to_string());
                                parse_mode = ParseMode::Hunk;
                            } else {
                                write!(hunks.last_mut().unwrap(), "\n{line}").unwrap();
                                parse_mode = ParseMode::Hunk;
                            }
                        }
                        ParseMode::Header => {
                            if let Some(_cap) = diff_start_pat.captures(line) {
                                files.push((
                                    std::mem::take(&mut header),
                                    std::mem::take(&mut hunks),
                                ));
                                header.push_str(line);
                                parse_mode = ParseMode::Header;
                            } else if let Some(_cap) = hunk_start_pat.captures(line) {
                                hunks.push(line.to_string());
                                parse_mode = ParseMode::Hunk;
                            } else {
                                write!(header, "\n{line}").unwrap();
                                parse_mode = ParseMode::Header;
                            }
                        }
                    }
                }
                if !header.is_empty() {
                    files.push((std::mem::take(&mut header), std::mem::take(&mut hunks)));
                }
            }
            log::debug!("After parsing {files:#?}");
            // for (header, hunks) in files.iter_mut() {
            //     hunks.extend(std::mem::take(&mut hunks).into_iter()
            //                  .flat_map(|hunk| {
            //                      if hunk.split('\n').any(|line| line.starts_with("+")) &&
            //                      hunk.split('\n').any(|line| line.starts_with("-")) {
            //                          let mut add = Vec::new();
            //                          let mut rem = Vec::new()kkkkkk
            //                          for line in hunk.split('\n') {
            //                          }
            //                      }
            //                  }))
            // }
            get_output(git().args(&["revert", "--no-commit", &commit]))?;
            get_output(git().args(&[
                "commit",
                "-m",
                &format!("@split Revert {commit}: {original_commit_message}"),
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

            #[derive(Hash, Default)]
            struct UiState {
                dont_save: bool,
                files: Vec<(String, Vec<String>)>,
                force_redraw_gen: u64,
                force_redraw_terminal_size: (u16, u16),
                allow_partial: bool,
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

                pub fn all_hunks_assigned(&self) -> bool {
                    self.hunk_commits.len() >= self.hunk_count()
                        && self.hunk_commits.iter().all(|c| c.is_some())
                }

                pub fn should_save_commits(&self) -> bool {
                    !self.dont_save && (self.allow_partial || self.all_hunks_assigned())
                }

                pub fn set_hunk_commit(&mut self, hunk_idx: usize, commit_id: CommitId) {
                    if self.hunk_commits.len() <= hunk_idx {
                        self.hunk_commits
                            .resize_with(hunk_idx + 1, Default::default);
                    }
                    self.hunk_commits[hunk_idx] = Some(commit_id);
                }

                pub fn hunks(&self) -> impl Iterator<Item = ((usize, usize), (&str, &str))> {
                    self.files
                        .iter()
                        .enumerate()
                        .flat_map(|(file_id, (header, hunks))| {
                            hunks.iter().enumerate().map(move |(hunk_id, hunk)| {
                                ((file_id, hunk_id), (header.as_str(), hunk.as_str()))
                            })
                        })
                }

                pub fn get_hunk(&self, idx: usize) -> Option<((usize, usize), (&str, &str))> {
                    self.hunks().nth(idx)
                }

                pub fn hunk_count(&self) -> usize {
                    self.files.iter().map(|(_, hunks)| hunks.len()).sum()
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
            let ui_state = {
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
                ui_state.files = files;
                ui_state.messages.insert(
                    '0',
                    CommitInfo {
                        commit_message: original_commit_message.clone(),
                    },
                );
                ui_state
                    .hunk_commits
                    .extend((0..ui_state.hunk_count()).map(|_| Some('0')));
                let mut prev_hash = 0;
                let stdout = stdout();
                let stdout = stdout.into_raw_mode()?;
                let mut screen = termion::screen::AlternateScreen::from(stdout);
                // let mut screen = stdout;
                let mut keys = stdin.keys();
                let mut draw_buffer = String::new();
                let mut out_buffer = String::new();
                'ui_loop: for gen in 1.. {
                    ui_state.force_redraw_terminal_size =
                        termion::terminal_size().unwrap_or_default();
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
                        let terminal_height = ui_state.force_redraw_terminal_size.1;
                        write!(
                            draw_buffer,
                            "{}{}",
                            termion::cursor::Goto(1, 1),
                            termion::clear::All
                        )?;
                        writeln!(
                            draw_buffer,
                            "{x}/{n} hunks assigned{partial}",
                            x = ui_state.hunk_commits.iter().filter(|h| h.is_some()).count(),
                            n = ui_state.hunk_count(),
                            partial = ui_state
                                .allow_partial
                                .then(|| FmtFn(|f| {
                                    write!(
                                        f,
                                        " {color}ALLOW PARTIAL{reset}",
                                        color = termion::color::Fg(termion::color::Yellow),
                                        reset = termion::color::Fg(termion::color::Reset),
                                    )
                                }))
                                .or_display("")
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
                                write!(draw_buffer, "Enter commit id to edit: ",)?;
                            }
                            UiMode::Editing {
                                commit,
                                message,
                                assign_to_hunk: _,
                            } => {
                                write!(
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
                                let ((_file_id, _hunk_id), (header, hunk)) =
                                    ui_state.get_hunk(*active_hunk).unwrap();
                                let commit =
                                    ui_state.hunk_commits.get(*active_hunk).copied().flatten();
                                let commit_message = commit
                                    .map(|commit| {
                                        let messages = &ui_state.messages;
                                        FmtFn(move |f| {
                                            write!(
                                                f,
                                                "[{commit}] {}",
                                                messages[&commit]
                                                    .commit_message
                                                    .split('\n')
                                                    .next()
                                                    .unwrap()
                                            )
                                        })
                                    })
                                    .into_or_display("---");
                                let header_line = header.split('\n').next().unwrap();
                                writeln!(
                                    draw_buffer,
                                    "{active_hunk}/{n}: {color}{commit_message}{reset}\n{header_line}",
                                    n = ui_state.hunk_count(),
                                    color = commit.and_then(|commit| commit_colors
                                        .entry(commit)
                                        .or_insert_with(|| commit_colors_seq.next()).as_ref())
                                        .into_or_display(""),
                                    reset = termion::color::Fg(termion::color::Reset),
                                )?;
                                writeln!(
                                    draw_buffer,
                                    "{}",
                                    render_hunk(
                                        hunk,
                                        (terminal_height as usize)
                                            .saturating_sub(ui_state.messages.len() + 5)
                                    )
                                )?;
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
                                termion::event::Key::Char(commit @ '0'..='9') => {
                                    let message = ui_state
                                        .messages
                                        .get(&commit)
                                        .as_ref()
                                        .map(|info| info.commit_message.as_str())
                                        .unwrap_or("");
                                    if let Ok(new_message) = edit(message) {
                                        ui_state.messages.insert(
                                            commit,
                                            CommitInfo {
                                                commit_message: new_message,
                                            },
                                        );
                                    }
                                    // let mode = UiMode::Editing {
                                    //     commit: c,
                                    //     message: ui_state
                                    //         .messages
                                    //         .get(&c)
                                    //         .map(|info| info.commit_message.chars().collect_vec())
                                    //         .unwrap_or_default(),
                                    //     assign_to_hunk: None,
                                    // };
                                    // ui_state.push_mode(mode);
                                }
                                _ => (),
                            }
                        }
                        UiMode::Editing { message, .. } => match key {
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
                            termion::event::Key::Ctrl('s') if !message.is_empty() => {
                                match ui_state.pop_mode() {
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
                                }
                            }
                            _ => (),
                        },
                        UiMode::Viewing { active_hunk } => {
                            let active_hunk = *active_hunk;
                            match key {
                                termion::event::Key::Ctrl('f') => {
                                    ui_state.allow_partial = !ui_state.allow_partial;
                                }
                                termion::event::Key::Ctrl('l') => {
                                    ui_state.force_redraw_gen = gen;
                                }
                                // https://github.com/twaugh/patchutils/blob/master/src/rediff.c
                                termion::event::Key::Ctrl('e') => {
                                    let launched_editor = loop {
                                        let ((file_id, _), (_, hunk_body)) =
                                            ui_state.get_hunk(active_hunk).unwrap();
                                        match edit(hunk_body) {
                                            Ok(output) if output.trim().is_empty() => break true,
                                            Ok(output) => {
                                                ui_state.files[file_id].1.push(output);
                                            }
                                            Err(err) => {
                                                log::error!("Failed to edit hunk body {err:?}");
                                                break false;
                                            }
                                        }
                                    };
                                    if launched_editor {
                                        // Works because we're always appending the
                                        // hunks after the original, so the hunk id
                                        // is preserved up to active_hunk.
                                        let ((file_id, hunk_id), _) =
                                            ui_state.get_hunk(active_hunk).unwrap();

                                        ui_state.files[file_id].1.remove(hunk_id);
                                        ui_state.hunk_commits.truncate(ui_state.hunk_count());
                                        let mode = UiMode::Viewing {
                                            active_hunk: active_hunk
                                                .min(ui_state.hunk_count().saturating_sub(1)),
                                        };
                                        ui_state.set_mode(mode);
                                    }
                                }
                                termion::event::Key::Char('p') => {
                                    let pager = std::env::var("PAGER").ok();
                                    let _ = spawn_with_input(
                                        &mut sh(pager
                                            .as_ref()
                                            .map(|s| s.as_str())
                                            .unwrap_or_else(|| "less")),
                                        |stdin| {
                                            let (_, (header, hunk_body)) =
                                                ui_state.get_hunk(active_hunk).unwrap();
                                            writeln!(stdin, "{header}")?;
                                            write!(
                                                stdin,
                                                "{}",
                                                render_hunk(hunk_body, usize::MAX,)
                                            )?;
                                            Ok(())
                                        },
                                    )
                                    .and_then(|mut child| {
                                        child.wait()?;
                                        ui_state.force_redraw_gen = gen;
                                        Ok(())
                                    });
                                }
                                // TODO for editor?
                                // TODO allow splitting a hunk?
                                // termion::event::Key::Ctrl('e') => {}
                                termion::event::Key::Ctrl('s')
                                    if ui_state.should_save_commits() =>
                                {
                                    break 'ui_loop;
                                }
                                termion::event::Key::Char('q') => {
                                    ui_state.dont_save = true;
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
                                    if (active_hunk + 1) < ui_state.hunk_count() {
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
                ui_state
            };
            // TODO repeatedly edit a hunk and produce diffs
            // until the user edits an empty file.
            // That way can split a hunk even further
            if ui_state.should_save_commits() {
                let file_lookup: Vec<usize> = ui_state
                    .hunks()
                    .map(|((file_id, _), _)| file_id)
                    .collect_vec();
                let hunks = ui_state
                    .hunks()
                    .map(|(_, (_, b))| b.to_string())
                    .collect_vec();
                let hunks_for_commit: BTreeMap<CommitId, Vec<usize>> = ui_state
                    .hunk_commits
                    .iter()
                    .copied()
                    .enumerate()
                    .flat_map(|(hunk_id, commit_id)| Some((commit_id?, hunk_id)))
                    .into_group_map()
                    .into_iter()
                    .collect();
                for (commit_id, hunk_ids) in hunks_for_commit.into_iter() {
                    let CommitInfo { commit_message } = &ui_state.messages[&commit_id];
                    log::debug!("Writing commit {commit_message}");
                    for (file_id, hunk_ids) in &hunk_ids
                        .into_iter()
                        .group_by(|hunk_id| file_lookup[dbg!(*hunk_id)])
                    {
                        let output = get_output_with_input(
                            &mut {
                                let mut cmd = git();
                                cmd.args(&["apply", "--reject"]);
                                if diff_context_size == 0 {
                                    cmd.arg("--unidiff-zero");
                                }
                                cmd
                            },
                            // git().args(&["apply", "--reject", "--recount"]),
                            |stdin| {
                                let (header, _) = &ui_state.files[file_id];
                                writeln!(stdin, "{header}")?;
                                log::debug!("HEADER: {header:?}");
                                // TODO sort by line numbers?
                                for hunk_id in hunk_ids {
                                    let hunk_body = &hunks[hunk_id];
                                    log::debug!("BODY: {header:?}");
                                    writeln!(stdin, "{hunk_body}")?;
                                }
                                stdin.flush()?;
                                Ok(())
                            },
                        )?;
                        for line in output.split('\n') {
                            if let Some(file) = line.strip_prefix("patching file ") {
                                let file = file.trim_end();
                                dbg!(file);
                                get_output(git().args(&["add", file]))?;
                            }
                        }
                        get_output(git().args(&["add", "-u"]))?;
                    }
                    get_output(git().args(&["commit", "-m", commit_message]))?;
                }
                // Sanity check
                if ui_state.all_hunks_assigned() {
                    get_output(git().args(&["diff", "--quiet", &commit]))?;
                }
                // get_output(
                //     git().args(&["rebase", "--continue"]),
                // )?;
            } else {
                get_output(git().args(&["rebase", "--abort"]))?;
                // anyhow::bail!("Not all hunks assigned");
            }
        }
    }
    Ok(())
}
