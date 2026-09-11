mod diff;
mod event;
mod session;
mod shell;
mod snapshot;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use time::OffsetDateTime;

use event::{Event, EventKind};
use session::{ENV_SESSION_DIR, Session, Status};

#[derive(Parser)]
#[command(
    name = "exarare",
    version,
    about = "Record server build work and turn it into a Markdown runbook and an Ansible playbook"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start a recording shell. Work done in it is recorded until it exits.
    Start {
        /// A name for this session, e.g. "web01 nginx setup"
        #[arg(short, long)]
        name: Option<String>,
        /// Shell to run (bash or zsh). Defaults to $SHELL.
        #[arg(long)]
        shell: Option<PathBuf>,
        /// Directory to snapshot for file changes. Repeatable; defaults to /etc.
        #[arg(long = "watch", value_name = "PATH")]
        watch: Vec<PathBuf>,
    },
    /// Finish the current recording (same as typing `exit` in the recording shell)
    Stop,
    /// Show whether this shell is being recorded
    Status,
    /// List recorded sessions
    List,
    /// Show the file changes of a session (defaults to the most recent one)
    Diff {
        /// Session id, as shown by `exarare list`
        session: Option<String>,
    },
    /// Insert a heading into the runbook, e.g. `exarare note "Install nginx"`
    Note {
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// Called by the shell hooks
    #[command(name = "__hook", hide = true)]
    Hook {
        #[command(subcommand)]
        hook: Hook,
    },
}

#[derive(Subcommand)]
enum Hook {
    Pre {
        #[arg(long)]
        cwd: String,
        #[arg(last = true, default_value = "")]
        cmd: String,
    },
    Post {
        #[arg(long = "exit", allow_negative_numbers = true)]
        exit_code: i32,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Start { name, shell, watch } => start(name, shell, watch),
        Cmd::Stop => stop(),
        Cmd::Status => status(),
        Cmd::List => list(),
        Cmd::Diff { session } => show_diff(session),
        Cmd::Note { text } => note(text.join(" ")),
        Cmd::Hook { hook } => {
            // Never disturb the user's shell: a lost event is better than an error on every prompt.
            let _ = run_hook(hook);
            Ok(ExitCode::SUCCESS)
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("exarare: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn start(name: Option<String>, shell: Option<PathBuf>, watch: Vec<PathBuf>) -> Result<ExitCode> {
    if std::env::var_os(ENV_SESSION_DIR).is_some() {
        bail!("already recording in this shell (run `exarare status`)");
    }
    let shell = shell::resolve(shell);
    let kind = shell::Kind::detect(&shell)?;
    let watch_roots = if watch.is_empty() {
        session::default_watch_roots()
    } else {
        watch
    };

    let mut session = Session::create(name, &shell, watch_roots)?;
    session.append(EventKind::SessionStart)?;
    take_snapshot(&session, "before")?;
    eprintln!(
        "exarare: recording session {} — type `exit` or run `exarare stop` to finish",
        session.meta.id
    );

    let dir = session.dir.clone();
    let result = shell::run(kind, &shell, &dir, |pid| {
        session.meta.shell_pid = Some(pid);
        if let Err(e) = session.save_meta() {
            eprintln!("exarare: {e:#}");
        }
    });

    session.append(EventKind::SessionEnd)?;
    session.meta.ended_at = Some(OffsetDateTime::now_utc());
    session.save_meta()?;
    result?;

    let changes = match take_snapshot(&session, "after") {
        Ok(_) => file_changes(&session).map(|c| c.len()).unwrap_or(0),
        Err(e) => {
            eprintln!("exarare: {e:#}");
            0
        }
    };
    let commands = count_commands(&session.events()?);
    eprintln!(
        "exarare: finished session {} ({commands} commands, {changes} changed files) — saved in {}",
        session.meta.id,
        session.dir.display()
    );
    if changes > 0 {
        eprintln!(
            "exarare: run `exarare diff {}` to see them",
            session.meta.id
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Walks the watched directories and stores the `before` or `after` snapshot.
fn take_snapshot(session: &Session, which: &str) -> Result<()> {
    let out = session.snapshot_path(which);
    std::fs::create_dir_all(out.parent().expect("snapshot path has a parent"))?;
    let rules = snapshot::Rules::defaults()?;
    let stats = snapshot::capture(
        &session.meta.watch_roots,
        &out,
        &session.blobs_dir(),
        &rules,
    )
    .with_context(|| format!("failed to take the {which} snapshot"))?;
    if stats.unreadable > 0 {
        eprintln!(
            "exarare: {} path(s) could not be read while snapshotting",
            stats.unreadable
        );
    }
    Ok(())
}

fn file_changes(session: &Session) -> Result<Vec<diff::Change>> {
    let before = snapshot::load(&session.snapshot_path("before"))?;
    let after = snapshot::load(&session.snapshot_path("after"))?;
    Ok(diff::compare(&before, &after))
}

fn stop() -> Result<ExitCode> {
    let session = Session::current()?.context("not recording")?;
    let pid = session
        .meta
        .shell_pid
        .context("the recording shell's pid is unknown")?;
    // An interactive shell ignores SIGTERM but exits on SIGHUP.
    kill(Pid::from_raw(pid as i32), Signal::SIGHUP)
        .with_context(|| format!("failed to signal the recording shell (pid {pid})"))?;
    Ok(ExitCode::SUCCESS)
}

fn status() -> Result<ExitCode> {
    let Some(session) = Session::current()? else {
        println!("not recording");
        return Ok(ExitCode::from(1));
    };
    let meta = &session.meta;
    println!("recording session {}", meta.id);
    if let Some(name) = &meta.name {
        println!("  name:     {name}");
    }
    println!("  started:  {}", meta.started_at);
    println!("  commands: {}", count_commands(&session.events()?));
    println!(
        "  watching: {}",
        meta.watch_roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("  data:     {}", session.dir.display());
    Ok(ExitCode::SUCCESS)
}

fn list() -> Result<ExitCode> {
    let sessions = Session::list()?;
    if sessions.is_empty() {
        println!("no sessions");
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "{:<22} {:<10} {:>5}  {:<20} NAME",
        "ID", "STATUS", "CMDS", "HOST"
    );
    for s in &sessions {
        let commands = s.events().map(|e| count_commands(&e)).unwrap_or(0);
        println!(
            "{:<22} {:<10} {:>5}  {:<20} {}",
            s.meta.id,
            s.status(),
            commands,
            s.meta.hostname.as_deref().unwrap_or("-"),
            s.meta.name.as_deref().unwrap_or("")
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn show_diff(id: Option<String>) -> Result<ExitCode> {
    let sessions = Session::list()?;
    let session = match &id {
        Some(id) => sessions
            .into_iter()
            .find(|s| &s.meta.id == id)
            .with_context(|| format!("no session {id}"))?,
        None => sessions.into_iter().next_back().context("no sessions")?,
    };
    if !session.snapshot_path("after").exists() {
        bail!(
            "session {} has no `after` snapshot (still recording, or it was aborted)",
            session.meta.id
        );
    }
    let changes = file_changes(&session)?;
    print!("{}", diff::render(&changes, &session.blobs_dir()));
    Ok(ExitCode::SUCCESS)
}

fn note(text: String) -> Result<ExitCode> {
    let session = Session::current()?.context("not recording")?;
    if session.status() != Status::Recording {
        bail!("session {} is not recording", session.meta.id);
    }
    session.append(EventKind::Note { text })?;
    Ok(ExitCode::SUCCESS)
}

fn run_hook(hook: Hook) -> Result<()> {
    let dir = std::env::var_os(ENV_SESSION_DIR).context("not recording")?;
    let kind = match hook {
        Hook::Pre { cwd, cmd } => EventKind::CmdStart { cmd, cwd },
        Hook::Post { exit_code } => EventKind::CmdEnd { exit_code },
    };
    session::append_event(Path::new(&dir), &Event::now(kind))
}

fn count_commands(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::CmdStart { .. }))
        .count()
}
