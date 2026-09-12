mod ansible;
mod diff;
mod event;
mod i18n;
mod markdown;
mod packages;
mod session;
mod shell;
mod snapshot;
mod state;
mod step;
mod sys;
mod watcher;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use time::OffsetDateTime;

use event::{Event, EventKind};
use i18n::Locale;
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
        /// Directory whose content is snapshotted and diffed. Repeatable; defaults to /etc.
        #[arg(long = "snapshot", value_name = "PATH")]
        snapshot: Vec<PathBuf>,
        /// Directory watched for touched files, with no content recorded. Repeatable;
        /// defaults to /opt, /usr/local, /srv, /var/www, /root and /home when they exist.
        #[arg(long = "watch", value_name = "PATH")]
        watch: Vec<PathBuf>,
        /// Do not watch the filesystem
        #[arg(long)]
        no_watcher: bool,
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
    /// Generate a document from a recorded session
    Gen {
        #[command(subcommand)]
        target: GenTarget,
    },
    /// Start a step of the runbook, e.g. `exarare step "Install nginx"`
    Step {
        #[arg(required = true)]
        title: Vec<String>,
    },
    /// Add a remark to the step being worked on, e.g. `exarare note "needs EPEL"`
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
enum GenTarget {
    /// A Markdown runbook
    Md {
        /// Session id, as shown by `exarare list` (defaults to the most recent)
        session: Option<String>,
        /// Write to this file instead of standard output
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Language of the generated text: en or ja. Defaults to EXARARE_LANG, then en.
        #[arg(long)]
        lang: Option<String>,
        /// Draw the overview as a Mermaid flowchart instead of a numbered list
        #[arg(long)]
        mermaid: bool,
    },
    /// An Ansible playbook, written into a directory with its files
    Ansible {
        /// Session id, as shown by `exarare list` (defaults to the most recent)
        session: Option<String>,
        /// Directory to write playbook.yml and files/ into
        #[arg(short, long, default_value = "ansible")]
        output: PathBuf,
        /// Language of the generated comments: en or ja
        #[arg(long)]
        lang: Option<String>,
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
        Cmd::Start {
            name,
            shell,
            snapshot,
            watch,
            no_watcher,
        } => start(name, shell, snapshot, watch, no_watcher),
        Cmd::Stop => stop(),
        Cmd::Status => status(),
        Cmd::List => list(),
        Cmd::Diff { session } => show_diff(session),
        Cmd::Gen { target } => match target {
            GenTarget::Md {
                session,
                output,
                lang,
                mermaid,
            } => gen_md(session, output, lang, mermaid),
            GenTarget::Ansible {
                session,
                output,
                lang,
            } => gen_ansible(session, output, lang),
        },
        Cmd::Step { title } => mark(EventKind::Step {
            title: title.join(" "),
        }),
        Cmd::Note { text } => mark(EventKind::Note {
            text: text.join(" "),
        }),
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

fn start(
    name: Option<String>,
    shell: Option<PathBuf>,
    snapshot: Vec<PathBuf>,
    watch: Vec<PathBuf>,
    no_watcher: bool,
) -> Result<ExitCode> {
    if std::env::var_os(ENV_SESSION_DIR).is_some() {
        bail!("already recording in this shell (run `exarare status`)");
    }
    let shell = shell::resolve(shell);
    let kind = shell::Kind::detect(&shell)?;
    let snapshot_roots = if snapshot.is_empty() {
        session::default_snapshot_roots()
    } else {
        snapshot
    };
    let watch_roots = if no_watcher {
        Vec::new()
    } else if watch.is_empty() {
        watcher::default_roots()
    } else {
        watch
    };

    if !sys::locale_is_utf8() {
        // Old shells mangle a multibyte argument when the locale says the
        // encoding is single-byte: bash 3.2 recorded a step title as an
        // unrelated string. The locale is deliberately not changed here, since
        // that would also change the behaviour of the commands being recorded.
        eprintln!(
            "exarare: warning: the locale is not UTF-8, so a non-ASCII step title or note may be recorded incorrectly by an older shell. Set LC_ALL to a UTF-8 locale before starting."
        );
    }
    let mut session = Session::create(name, &shell, snapshot_roots, watch_roots)?;
    session.append(EventKind::SessionStart)?;
    take_snapshot(&session, "before")?;
    take_packages(&session, "before");
    take_state(&session, "before");
    let fs_watcher = start_watcher(&session);
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

    // Stopped before the `after` snapshot, so that exarare's own reads and
    // writes cannot end up in the list of touched files.
    if let Some(watcher) = fs_watcher {
        let touched = watcher.finish();
        for gap in &touched.incomplete_roots {
            eprintln!("exarare: could not watch {gap}");
        }
        if touched.truncated {
            eprintln!(
                "exarare: more than {} paths were touched, so the list is incomplete",
                watcher::MAX_PATHS
            );
        }
        if let Err(e) = watcher::save(&touched, &session.touched_path()) {
            eprintln!("exarare: {e:#}");
        }
    }

    session.append(EventKind::SessionEnd)?;
    session.meta.ended_at = Some(OffsetDateTime::now_utc());
    session.save_meta()?;
    result?;

    take_packages(&session, "after");
    take_state(&session, "after");
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
    eprintln!(
        "exarare: run `exarare gen md {}` for a runbook",
        session.meta.id
    );
    Ok(ExitCode::SUCCESS)
}

/// Walks the watched directories and stores the `before` or `after` snapshot.
fn take_snapshot(session: &Session, which: &str) -> Result<()> {
    let out = session.snapshot_path(which);
    std::fs::create_dir_all(out.parent().expect("snapshot path has a parent"))?;
    let rules = snapshot::Rules::defaults()?;
    let stats = snapshot::capture(
        &session.meta.snapshot_roots,
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

/// Reads the package state. A host without rpm records an empty snapshot, so
/// that a later comparison reports nothing rather than a wipe.
fn take_packages(session: &Session, which: &str) {
    let snapshot = packages::capture();
    for error in &snapshot.errors {
        eprintln!("exarare: {error}");
    }
    if let Err(e) = packages::save(&snapshot, &session.packages_path(which)) {
        eprintln!("exarare: {e:#}");
    }
}

fn package_changes(session: &Session) -> packages::Diff {
    let before = packages::load(&session.packages_path("before")).unwrap_or_default();
    let after = packages::load(&session.packages_path("after")).unwrap_or_default();
    packages::compare(&before, &after)
}

/// Starts the filesystem watcher. It adds to the record rather than carrying
/// it, so a failure to start is reported and the session goes on without it.
fn start_watcher(session: &Session) -> Option<watcher::Watcher> {
    if session.meta.watch_roots.is_empty() {
        return None;
    }
    let rules = match watcher::Rules::new(session::data_dir().ok()) {
        Ok(rules) => rules,
        Err(e) => {
            eprintln!("exarare: {e:#}");
            return None;
        }
    };
    match watcher::start(&session.meta.watch_roots, rules) {
        Ok(watcher) => Some(watcher),
        Err(e) => {
            eprintln!("exarare: {e:#}");
            None
        }
    }
}

fn touched_files(session: &Session) -> watcher::Touched {
    watcher::load(&session.touched_path()).unwrap_or_default()
}

/// Reads service, firewall and account state. Subsystems that do not answer —
/// systemd inside a container, a stopped firewalld — are recorded as unknown.
fn take_state(session: &Session, which: &str) {
    let snapshot = state::capture();
    for error in &snapshot.errors {
        eprintln!("exarare: {error}");
    }
    if let Err(e) = state::save(&snapshot, &session.state_path(which)) {
        eprintln!("exarare: {e:#}");
    }
}

fn state_changes(session: &Session) -> state::Diff {
    let before = state::load(&session.state_path("before")).unwrap_or_default();
    let after = state::load(&session.state_path("after")).unwrap_or_default();
    state::compare(&before, &after)
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
    let list = |paths: &[PathBuf]| {
        if paths.is_empty() {
            "-".to_string()
        } else {
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    println!("  snapshots: {}", list(&meta.snapshot_roots));
    println!("  watching:  {}", list(&meta.watch_roots));
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

/// The session named by `id`, or the most recent one.
fn resolve_session(id: Option<String>) -> Result<Session> {
    let sessions = Session::list()?;
    match &id {
        Some(id) => sessions
            .into_iter()
            .find(|s| &s.meta.id == id)
            .with_context(|| format!("no session {id}")),
        None => sessions.into_iter().next_back().context("no sessions"),
    }
}

fn require_finished_snapshots(session: &Session) -> Result<()> {
    if !session.snapshot_path("after").exists() {
        bail!(
            "session {} has no `after` snapshot (still recording, or it was aborted)",
            session.meta.id
        );
    }
    Ok(())
}

fn show_diff(id: Option<String>) -> Result<ExitCode> {
    let session = resolve_session(id)?;
    require_finished_snapshots(&session)?;
    let changes = file_changes(&session)?;
    print!("{}", diff::render(&changes, &session.blobs_dir()));
    print!("{}", packages::render(&package_changes(&session)));
    print!("{}", state::render(&state_changes(&session)));

    // Touched paths are listed after the changes, since they say less: a write
    // happened, and nothing about what the content became.
    let touched = touched_files(&session);
    for path in touched.paths.keys() {
        println!("touched      {}", path.display());
    }
    if touched.truncated {
        println!(
            "warning      more than {} paths were touched, list is incomplete",
            watcher::MAX_PATHS
        );
    }
    for gap in &touched.incomplete_roots {
        println!("warning      could not watch {gap}");
    }
    Ok(ExitCode::SUCCESS)
}

fn gen_md(
    id: Option<String>,
    output: Option<PathBuf>,
    lang: Option<String>,
    mermaid: bool,
) -> Result<ExitCode> {
    let session = resolve_session(id)?;
    require_finished_snapshots(&session)?;
    let locale = Locale::resolve(lang.as_deref())?;
    let changes = file_changes(&session)?;
    let mut runbook = step::build(&session.events()?, &changes);
    step::attribute_packages(&mut runbook, package_changes(&session));
    step::attribute_state(&mut runbook, state_changes(&session));
    let blobs = session.blobs_dir();
    let document = markdown::render(
        &markdown::Input {
            meta: &session.meta,
            runbook: &runbook,
            blobs: &blobs,
            version: env!("CARGO_PKG_VERSION"),
            touched: touched_files(&session),
            mermaid,
        },
        &locale.messages(),
    );
    match output {
        Some(path) => {
            std::fs::write(&path, &document)
                .with_context(|| format!("failed to write {}", path.display()))?;
            eprintln!("exarare: wrote {}", path.display());
        }
        None => print!("{document}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn gen_ansible(id: Option<String>, output: PathBuf, lang: Option<String>) -> Result<ExitCode> {
    let session = resolve_session(id)?;
    require_finished_snapshots(&session)?;
    let locale = Locale::resolve(lang.as_deref())?;
    let changes = file_changes(&session)?;
    let mut runbook = step::build(&session.events()?, &changes);
    step::attribute_packages(&mut runbook, package_changes(&session));
    step::attribute_state(&mut runbook, state_changes(&session));

    let blobs = session.blobs_dir();
    let playbook = ansible::render(
        &ansible::Input {
            meta: &session.meta,
            runbook: &runbook,
            blobs: &blobs,
            version: env!("CARGO_PKG_VERSION"),
        },
        &locale.messages(),
    );
    ansible::write(&playbook, &output)?;
    eprintln!(
        "exarare: wrote {} and {} file(s)",
        output.join("playbook.yml").display(),
        playbook.files.len()
    );
    // The warnings are what a human has to finish, so they are not left buried
    // in the playbook alone.
    for warning in &playbook.warnings {
        eprintln!("exarare: {warning}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Records a step heading or a remark against the session of this shell.
fn mark(kind: EventKind) -> Result<ExitCode> {
    let session = Session::current()?.context("not recording")?;
    if session.status() != Status::Recording {
        bail!("session {} is not recording", session.meta.id);
    }
    session.append(kind)?;
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
