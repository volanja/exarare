//! Renders a recorded session as a Markdown runbook.
//!
//! Chapters are either generated from the record, or a template for a human to
//! fill in. Nothing is invented: where the record cannot answer a question, the
//! document says so and leaves a TODO.

use std::fmt::Write as _;
use std::path::Path;

use time::OffsetDateTime;
use time::macros::format_description;

use crate::diff::{Change, unified_body};
use crate::i18n::Messages;
use crate::session::Meta;
use crate::step::{CommandRun, Item, Runbook, Step};

/// Diff lines shown inline per file; longer diffs move to an appendix.
const MAX_INLINE_DIFF_LINES: usize = 50;

pub struct Input<'a> {
    pub meta: &'a Meta,
    pub runbook: &'a Runbook,
    /// Blob store of the session, for reading recorded file content.
    pub blobs: &'a Path,
    pub version: &'a str,
}

pub fn render(input: &Input, msg: &Messages) -> String {
    let mut out = String::new();
    let mut long_diffs: Vec<(&Change, String)> = Vec::new();

    metadata(&mut out, input, msg);
    purpose(&mut out, msg);
    overview(&mut out, input.runbook, msg);
    prerequisites(&mut out, input, msg);
    procedure(&mut out, input, msg, &mut long_diffs);
    what_changed(&mut out, input, msg, &mut long_diffs);
    verification(&mut out, input.runbook, msg);
    rollback(&mut out, input.runbook, msg);
    appendix_log(&mut out, input.runbook, msg);
    appendix_diffs(&mut out, &long_diffs, msg);
    appendix_generated(&mut out, input, msg);
    out
}

/// A step with no title of its own is either the whole session, or the work
/// that came before the first `exarare step`.
fn title_of(step: &Step, without_steps: bool, msg: &Messages) -> String {
    step.title.clone().unwrap_or_else(|| {
        if without_steps {
            msg.unnamed_step().to_string()
        } else {
            msg.before_first_step().to_string()
        }
    })
}

fn datetime(ts: OffsetDateTime) -> String {
    ts.format(format_description!(
        "[year]-[month]-[day] [hour]:[minute]:[second] UTC"
    ))
    .unwrap_or_else(|_| ts.to_string())
}

fn clock(ts: OffsetDateTime) -> String {
    ts.format(format_description!("[hour]:[minute]:[second]"))
        .unwrap_or_else(|_| ts.to_string())
}

fn metadata(out: &mut String, input: &Input, msg: &Messages) {
    let meta = input.meta;
    let title = meta.name.clone().unwrap_or_else(|| meta.id.clone());
    let _ = writeln!(out, "# {}: {title}\n", msg.runbook_title());
    let _ = writeln!(out, "| | |\n|---|---|");
    let row = |out: &mut String, label: &str, value: &str| {
        let _ = writeln!(out, "| {label} | {value} |");
    };
    row(out, msg.session_id(), &meta.id);
    row(out, msg.host(), meta.hostname.as_deref().unwrap_or("-"));
    row(out, msg.user(), meta.user.as_deref().unwrap_or("-"));
    row(out, msg.shell(), &meta.shell);
    row(out, msg.started(), &datetime(meta.started_at));
    row(
        out,
        msg.ended(),
        &meta
            .ended_at
            .map(datetime)
            .unwrap_or_else(|| "-".to_string()),
    );
    out.push('\n');
}

fn todo(out: &mut String, msg: &Messages, text: &str) {
    if text.is_empty() {
        let _ = writeln!(out, "> **{}**\n", msg.todo());
    } else {
        let _ = writeln!(out, "> **{}**: {text}\n", msg.todo());
    }
}

fn purpose(out: &mut String, msg: &Messages) {
    let _ = writeln!(out, "## 1. {}\n", msg.purpose());
    let _ = writeln!(out, "{}\n", msg.purpose_intro());
    for item in [msg.purpose_what(), msg.purpose_why(), msg.purpose_done()] {
        let _ = writeln!(out, "### {item}\n");
        todo(out, msg, "");
    }
}

fn overview(out: &mut String, runbook: &Runbook, msg: &Messages) {
    let _ = writeln!(out, "## 2. {}\n", msg.overview());
    let _ = writeln!(out, "{}\n", msg.overview_intro());
    for (i, step) in runbook.steps.iter().enumerate() {
        let _ = writeln!(
            out,
            "{}. **{}** — {}{sep}{}{sep}{}",
            i + 1,
            title_of(step, runbook.without_steps, msg),
            msg.count_commands(step.command_count()),
            msg.count_packages(step.packages.len()),
            msg.count_files(step.files.len()),
            sep = msg.list_separator()
        );
    }
    out.push('\n');
    if runbook.without_steps {
        let _ = writeln!(out, "{}\n", msg.no_steps_hint());
    }
}

fn prerequisites(out: &mut String, input: &Input, msg: &Messages) {
    let meta = input.meta;
    let _ = writeln!(out, "## 3. {}\n", msg.prerequisites());
    let _ = writeln!(out, "### {}\n", msg.prereq_recorded());
    let _ = writeln!(out, "| | |\n|---|---|");
    let _ = writeln!(
        out,
        "| {} | {} |",
        msg.host(),
        meta.hostname.as_deref().unwrap_or("-")
    );
    let _ = writeln!(
        out,
        "| {} | {} |",
        msg.user(),
        meta.user.as_deref().unwrap_or("-")
    );
    let _ = writeln!(
        out,
        "| {} | {} |",
        msg.watched_directories(),
        paths(&meta.watch_roots)
    );
    out.push('\n');
    todo(out, msg, msg.prereq_todo());
}

fn paths(paths: &[std::path::PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!("`{}`", p.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Writes the commands collected so far as one console block.
fn flush_commands(out: &mut String, pending: &mut Vec<&CommandRun>) {
    if pending.is_empty() {
        return;
    }
    let _ = writeln!(out, "```console");
    for run in pending.iter() {
        let _ = writeln!(out, "$ {}", run.cmd);
    }
    let _ = writeln!(out, "```\n");
    pending.clear();
}

/// Commands and notes in the order they were recorded: a remark splits the
/// console block, so that it stays next to the command it is about.
fn step_body(out: &mut String, step: &Step, msg: &Messages) {
    let mut pending: Vec<&CommandRun> = Vec::new();
    for item in &step.items {
        match item {
            Item::Command(run) => pending.push(run),
            Item::Note(text) => {
                flush_commands(out, &mut pending);
                let _ = writeln!(out, "> **{}**: {text}\n", msg.note_label());
            }
        }
    }
    flush_commands(out, &mut pending);
}

/// The directories the commands of a step ran in, in order of appearance.
fn directories(step: &Step) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for run in step.commands() {
        if !seen.contains(&run.cwd.as_str()) {
            seen.push(&run.cwd);
        }
    }
    seen
}

fn diff_block<'a>(
    out: &mut String,
    change: &'a Change,
    blobs: &Path,
    msg: &Messages,
    long_diffs: &mut Vec<(&'a Change, String)>,
) {
    let Some(body) = unified_body(change, blobs) else {
        return;
    };
    let lines: Vec<&str> = body.lines().collect();
    let _ = writeln!(out, "```diff");
    for line in lines.iter().take(MAX_INLINE_DIFF_LINES) {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out, "```\n");
    if lines.len() > MAX_INLINE_DIFF_LINES {
        let _ = writeln!(
            out,
            "{}\n",
            msg.diff_truncated(MAX_INLINE_DIFF_LINES, lines.len())
        );
        long_diffs.push((change, body));
    }
}

fn change_line(change: &Change, msg: &Messages) -> String {
    format!(
        "- {} `{}`",
        msg.change_kind(change.label()),
        change.path().display()
    )
}

fn procedure<'a>(
    out: &mut String,
    input: &'a Input,
    msg: &Messages,
    long_diffs: &mut Vec<(&'a Change, String)>,
) {
    let runbook = input.runbook;
    let _ = writeln!(out, "## 4. {}\n", msg.procedure());
    let _ = writeln!(out, "{}\n", msg.procedure_intro());
    for (i, step) in runbook.steps.iter().enumerate() {
        let _ = writeln!(
            out,
            "### 4.{}. {}\n",
            i + 1,
            title_of(step, runbook.without_steps, msg)
        );
        step_body(out, step, msg);
        let dirs = directories(step);
        if !dirs.is_empty() {
            let _ = writeln!(
                out,
                "{}: {}\n",
                msg.working_directory(),
                dirs.iter()
                    .map(|d| format!("`{d}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if !step.packages.is_empty() {
            let _ = writeln!(out, "#### {}\n", msg.packages());
            for package in &step.packages {
                let _ = writeln!(out, "- `{}` {}", package.name, package.version);
            }
            out.push('\n');
        }
        if step.files.is_empty() {
            continue;
        }
        let _ = writeln!(out, "#### {}\n", msg.step_files());
        for change in &step.files {
            let _ = writeln!(out, "{}\n", change_line(change, msg));
            diff_block(out, change, input.blobs, msg, long_diffs);
        }
    }
}

fn what_changed<'a>(
    out: &mut String,
    input: &'a Input,
    msg: &Messages,
    long_diffs: &mut Vec<(&'a Change, String)>,
) {
    let runbook = input.runbook;
    let _ = writeln!(out, "## 5. {}\n", msg.what_changed());
    packages_section(out, runbook, msg);

    let total =
        runbook.steps.iter().map(|s| s.files.len()).sum::<usize>() + runbook.other_files.len();
    if total == 0 {
        let _ = writeln!(out, "{}\n", msg.no_changes());
        return;
    }
    let _ = writeln!(out, "### {}\n", msg.changed_files());
    let _ = writeln!(out, "| {} | {} |\n|---|---|", msg.path(), msg.change());
    for change in runbook
        .steps
        .iter()
        .flat_map(|s| s.files.iter())
        .chain(runbook.other_files.iter())
    {
        let _ = writeln!(
            out,
            "| `{}` | {} |",
            change.path().display(),
            msg.change_kind(change.label())
        );
    }
    out.push('\n');
    let _ = writeln!(out, "{}\n", msg.attribution_note());
    for change in &runbook.other_files {
        let _ = writeln!(out, "{}\n", change_line(change, msg));
        diff_block(out, change, input.blobs, msg, long_diffs);
    }
}

/// Packages, repositories and module streams, from the rpm database rather
/// than from what the commands printed.
fn packages_section(out: &mut String, runbook: &Runbook, msg: &Messages) {
    let diff = &runbook.packages;
    let _ = writeln!(out, "### {}\n", msg.packages());

    if !diff.errors.is_empty() {
        let _ = writeln!(out, "> {}\n", msg.packages_incomplete());
        for error in &diff.errors {
            let _ = writeln!(out, "> - {error}");
        }
        out.push('\n');
    }
    if !diff.available {
        let _ = writeln!(out, "{}\n", msg.packages_unavailable());
        return;
    }
    if diff.is_empty() {
        let _ = writeln!(out, "{}\n", msg.no_package_changes());
        return;
    }

    let explicit = diff.explicit_installs();
    if !explicit.is_empty() {
        let _ = writeln!(out, "**{}**\n", msg.packages_installed());
        let _ = writeln!(out, "| {} | {} |\n|---|---|", msg.name(), msg.version());
        for package in &explicit {
            let _ = writeln!(out, "| `{}` | {} |", package.name, package.version);
        }
        out.push('\n');
        let dependencies = diff.dependency_installs().len();
        if dependencies > 0 {
            let _ = writeln!(out, "{}\n", msg.packages_dependencies(dependencies));
        }
    }
    if !diff.removed.is_empty() {
        let _ = writeln!(out, "**{}**\n", msg.packages_removed());
        for package in &diff.removed {
            let _ = writeln!(out, "- `{}` {}", package.name, package.version);
        }
        out.push('\n');
    }
    if !diff.upgraded.is_empty() {
        let _ = writeln!(out, "**{}**\n", msg.packages_upgraded());
        for upgrade in &diff.upgraded {
            let _ = writeln!(
                out,
                "- `{}` {} → {}",
                upgrade.name, upgrade.from, upgrade.to
            );
        }
        out.push('\n');
    }
    for (label, values) in [
        (msg.repositories_added(), &diff.repositories_added),
        (msg.repositories_removed(), &diff.repositories_removed),
        (msg.modules_added(), &diff.modules_added),
    ] {
        if values.is_empty() {
            continue;
        }
        let _ = writeln!(out, "**{label}**\n");
        for value in values {
            let _ = writeln!(out, "- `{value}`");
        }
        out.push('\n');
    }
}

fn verification(out: &mut String, runbook: &Runbook, msg: &Messages) {
    let _ = writeln!(out, "## 6. {}\n", msg.verification());
    if runbook.verifications.is_empty() {
        let _ = writeln!(out, "{}\n", msg.verification_none());
        todo(out, msg, "");
        return;
    }
    let _ = writeln!(out, "{}\n", msg.verification_intro());
    for run in &runbook.verifications {
        let _ = writeln!(out, "- `{}`\n", run.cmd);
        let _ = writeln!(out, "  > **{}**: {}\n", msg.todo(), msg.expected_result());
    }
}

fn rollback(out: &mut String, runbook: &Runbook, msg: &Messages) {
    let _ = writeln!(out, "## 7. {}\n", msg.rollback());
    let _ = writeln!(out, "> {}\n", msg.rollback_disclaimer());

    let files: Vec<&Change> = runbook
        .steps
        .iter()
        .flat_map(|s| s.files.iter())
        .chain(runbook.other_files.iter())
        .collect();
    let installed = runbook.packages.explicit_installs();
    if files.is_empty() && installed.is_empty() {
        let _ = writeln!(out, "{}\n", msg.rollback_none());
        return;
    }
    if !installed.is_empty() {
        let _ = writeln!(out, "### {}\n", msg.rollback_packages());
        for package in &installed {
            let _ = writeln!(out, "- `{}` {}", package.name, package.version);
        }
        out.push('\n');
    }
    if !files.is_empty() {
        let _ = writeln!(out, "### {}\n", msg.rollback_files());
        for change in files {
            let _ = writeln!(out, "{}", change_line(change, msg));
        }
        out.push('\n');
    }
    todo(out, msg, "");
}

fn appendix_log(out: &mut String, runbook: &Runbook, msg: &Messages) {
    let _ = writeln!(out, "## {}\n", msg.appendix_log());
    let _ = writeln!(out, "{}\n", msg.appendix_log_intro());
    let _ = writeln!(
        out,
        "| {} | {} | {} | {} |\n|---|---|---|---|",
        msg.time(),
        msg.command(),
        msg.working_directory(),
        msg.exit_code()
    );
    for run in &runbook.log {
        let exit = run
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| msg.still_running().to_string());
        let _ = writeln!(
            out,
            "| {} | `{}` | `{}` | {exit} |",
            clock(run.ts),
            run.cmd.replace('|', "\\|"),
            run.cwd
        );
    }
    out.push('\n');
}

fn appendix_diffs(out: &mut String, long_diffs: &[(&Change, String)], msg: &Messages) {
    if long_diffs.is_empty() {
        return;
    }
    let _ = writeln!(out, "## {}\n", msg.appendix_diffs());
    for (change, body) in long_diffs {
        let _ = writeln!(out, "### `{}`\n", change.path().display());
        let _ = writeln!(out, "```diff");
        out.push_str(body);
        let _ = writeln!(out, "```\n");
    }
}

fn appendix_generated(out: &mut String, input: &Input, msg: &Messages) {
    let _ = writeln!(out, "## {}\n", msg.appendix_generated());
    let _ = writeln!(out, "| | |\n|---|---|");
    let _ = writeln!(
        out,
        "| {} | exarare {} |",
        msg.generated_by(),
        input.version
    );
    let _ = writeln!(
        out,
        "| {} | {} |",
        msg.watched_directories(),
        paths(&input.meta.watch_roots)
    );
    out.push('\n');
    let _ = writeln!(out, "{}", msg.generated_note());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, EventKind};
    use crate::i18n::Locale;
    use crate::session::default_watch_roots;
    use crate::step;

    fn meta() -> Meta {
        Meta {
            id: "20260912-101500-0001".into(),
            name: Some("web01 nginx setup".into()),
            hostname: Some("web01".into()),
            user: Some("root".into()),
            shell: "/bin/bash".into(),
            watch_roots: default_watch_roots(),
            started_at: OffsetDateTime::UNIX_EPOCH,
            ended_at: Some(OffsetDateTime::UNIX_EPOCH),
            shell_pid: Some(1234),
        }
    }

    fn events() -> Vec<Event> {
        let at = |kind| Event {
            ts: OffsetDateTime::UNIX_EPOCH,
            kind,
        };
        vec![
            at(EventKind::SessionStart),
            at(EventKind::Step {
                title: "Install nginx".into(),
            }),
            at(EventKind::CmdStart {
                cmd: "dnf install -y nginx".into(),
                cwd: "/root".into(),
            }),
            at(EventKind::CmdEnd { exit_code: 0 }),
            at(EventKind::Note {
                text: "needs EPEL enabled".into(),
            }),
            at(EventKind::CmdStart {
                cmd: "systemctl enable --now nginx".into(),
                cwd: "/root".into(),
            }),
            at(EventKind::CmdEnd { exit_code: 0 }),
            at(EventKind::CmdStart {
                cmd: "systemctl status nginx".into(),
                cwd: "/root".into(),
            }),
            at(EventKind::CmdEnd { exit_code: 0 }),
            at(EventKind::SessionEnd),
        ]
    }

    fn rendered(locale: Locale) -> String {
        let runbook = step::build(&events(), &[]);
        let meta = meta();
        let input = Input {
            meta: &meta,
            runbook: &runbook,
            blobs: Path::new("/nonexistent"),
            version: "0.1.0",
        };
        render(&input, &locale.messages())
    }

    #[test]
    fn renders_every_chapter_in_order() {
        let doc = rendered(Locale::En);
        let chapters: Vec<&str> = doc
            .lines()
            .filter(|l| l.starts_with("## "))
            .map(|l| l.trim_start_matches("## "))
            .collect();
        assert_eq!(
            chapters,
            vec![
                "1. Purpose",
                "2. Overview of the work",
                "3. Prerequisites",
                "4. Procedure",
                "5. What changed",
                "6. Verification",
                "7. Rollback",
                "Appendix A: full command log",
                "Appendix B: how this was generated",
            ]
        );
        assert!(doc.starts_with("# Build runbook: web01 nginx setup"));
        assert!(doc.contains("### 4.1. Install nginx"));
    }

    #[test]
    fn leaves_the_human_parts_as_todo() {
        let doc = rendered(Locale::En);
        assert!(doc.contains("### What was done"));
        assert!(doc.contains("### Why it was needed"));
        assert!(doc.contains("### Definition of done"));
        assert!(doc.contains("> **TODO**"));
    }

    #[test]
    fn a_note_is_rendered_between_the_commands() {
        let doc = rendered(Locale::En);
        let step = doc.split("### 4.1. Install nginx").nth(1).unwrap();
        let step = step.split("## 5.").next().unwrap();
        let note_at = step.find("> **Note**: needs EPEL enabled").unwrap();
        let before = step.find("dnf install -y nginx").unwrap();
        let after = step.find("systemctl enable --now nginx").unwrap();
        assert!(
            before < note_at && note_at < after,
            "the note should sit between the two commands: {step}"
        );
        // The console block is split, rather than the note being moved away.
        assert_eq!(step.matches("```console").count(), 2, "{step}");
    }

    #[test]
    fn rollback_opens_with_a_disclaimer() {
        let doc = rendered(Locale::En);
        let rollback = doc.split("## 7. Rollback").nth(1).unwrap();
        let disclaimer = rollback.lines().find(|l| l.starts_with("> ")).unwrap();
        assert!(disclaimer.contains("not a verified rollback procedure"));
    }

    #[test]
    fn separates_instructions_from_the_log() {
        let doc = rendered(Locale::En);
        let procedure = doc.split("## 4. Procedure").nth(1).unwrap();
        let procedure = procedure.split("## 5.").next().unwrap();
        assert!(procedure.contains("dnf install -y nginx"));
        // A check belongs in chapter 6, not in the instructions.
        assert!(!procedure.contains("systemctl status nginx"));
        assert!(doc.contains("- `systemctl status nginx`"));
        // The appendix keeps everything.
        let appendix = doc.split("Appendix A").nth(1).unwrap();
        assert!(appendix.contains("systemctl status nginx"));
    }

    #[test]
    fn renders_package_changes() {
        use crate::packages::{Diff, Package, Upgrade};

        let package = |name: &str| Package {
            name: name.into(),
            version: "1:1.20.1-14.el9".into(),
            arch: "x86_64".into(),
        };
        let diff = Diff {
            available: true,
            installed: vec![package("nginx"), package("nginx-core")],
            upgraded: vec![Upgrade {
                name: "bash".into(),
                arch: "x86_64".into(),
                from: "5.1.8-6.el9".into(),
                to: "5.1.8-9.el9".into(),
            }],
            explicit: ["nginx".to_string()].into_iter().collect(),
            explicit_known: true,
            repositories_added: vec!["epel".into()],
            ..Default::default()
        };

        let mut runbook = step::build(&events(), &[]);
        step::attribute_packages(&mut runbook, diff);
        let meta = meta();
        let doc = render(
            &Input {
                meta: &meta,
                runbook: &runbook,
                blobs: Path::new("/nonexistent"),
                version: "0.1.0",
            },
            &Locale::En.messages(),
        );

        // `dnf install -y nginx` names nginx, so the step claims it.
        let step_section = doc.split("### 4.1. Install nginx").nth(1).unwrap();
        let step_section = step_section.split("## 5.").next().unwrap();
        assert!(step_section.contains("#### Packages"), "{step_section}");
        assert!(
            step_section.contains("- `nginx` 1:1.20.1-14.el9"),
            "{step_section}"
        );
        // A dependency is not presented as something the operator asked for.
        assert!(!step_section.contains("nginx-core"), "{step_section}");

        let changed = doc.split("## 5. What changed").nth(1).unwrap();
        let changed = changed.split("## 6.").next().unwrap();
        assert!(changed.contains("**Installed**"), "{changed}");
        assert!(
            changed.contains("| `nginx` | 1:1.20.1-14.el9 |"),
            "{changed}"
        );
        assert!(
            changed.contains("1 more were installed as dependencies"),
            "{changed}"
        );
        assert!(changed.contains("**Upgraded**"), "{changed}");
        assert!(
            changed.contains("`bash` 5.1.8-6.el9 → 5.1.8-9.el9"),
            "{changed}"
        );
        assert!(changed.contains("Repositories enabled"), "{changed}");

        // Rollback offers to remove what was installed, behind its disclaimer.
        let rollback = doc.split("## 7. Rollback").nth(1).unwrap();
        assert!(rollback.contains("Packages to remove"), "{rollback}");
        assert!(rollback.contains("- `nginx`"), "{rollback}");
    }

    #[test]
    fn distinguishes_no_packages_from_no_package_state() {
        // A host without rpm must not be reported as one where nothing changed.
        let unknown = rendered(Locale::En);
        let changed = unknown.split("## 5. What changed").nth(1).unwrap();
        assert!(changed.contains("this host has no rpm"), "{changed}");

        let mut runbook = step::build(&events(), &[]);
        step::attribute_packages(
            &mut runbook,
            crate::packages::Diff {
                available: true,
                ..Default::default()
            },
        );
        let meta = meta();
        let known = render(
            &Input {
                meta: &meta,
                runbook: &runbook,
                blobs: Path::new("/nonexistent"),
                version: "0.1.0",
            },
            &Locale::En.messages(),
        );
        let changed = known.split("## 5. What changed").nth(1).unwrap();
        assert!(
            changed.contains("No package changes were recorded."),
            "{changed}"
        );
    }

    #[test]
    fn renders_in_japanese() {
        let doc = rendered(Locale::Ja);
        assert!(doc.starts_with("# 構築手順書: web01 nginx setup"));
        assert!(doc.contains("## 1. 目的"));
        assert!(doc.contains("## 7. 切り戻し"));
        assert!(doc.contains("> **要記入**"));
        assert!(doc.contains("> **補足**: needs EPEL enabled"));
        // Recorded data stays as it was recorded.
        assert!(doc.contains("dnf install -y nginx"));
    }
}
