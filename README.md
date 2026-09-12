# Exarare

> **exarāre** (Latin) — literally "to plough up, to dig out". Romans wrote by
> scratching grooves into wax tablets with a stylus, so the word also came to
> mean, poetically, "to write".

Exarare records the work you do while building a Linux server over SSH and
turns it into a **Markdown runbook** and an **Ansible playbook** — without an
LLM, using plain rules.

Target platforms: **RHEL and AlmaLinux 8, 9 and 10**. Recording also works on
macOS, which is handy for development.

> [!WARNING]
> Exarare is at an early stage. Command recording and file diffs work today;
> package detection and runbook / playbook generation are under development
> (see [Roadmap](#roadmap)).

## Why

Tools that record a terminal session capture command lines and output, but
they cannot tell you *which packages* `dnf` actually installed or *what
changed* inside the files you edited. Exarare combines:

1. **Command recording** — a recording shell with hooks captures every command
   line, its working directory, exit code and time.
2. **System state diffs** — snapshots taken before and after the session
   reveal edited files, installed packages, enabled services, firewall rules
   and users.
3. **Rule-based generation** — the recorded events and diffs are mapped to
   runbook sections and Ansible modules (`dnf`, `copy`, `systemd`,
   `firewalld`, `user`, ...).

## Installation

Requires Rust 1.85 or later.

```sh
cargo install --git https://github.com/volanja/exarare
```

Prebuilt static binaries and RPM packages will be published on the
[Releases](https://github.com/volanja/exarare/releases) page.

## Usage

Start a recording shell. Run it as root when you are going to build a server,
so that everything you do is captured in one session:

```sh
sudo exarare start --name "web01 nginx setup"
```

The prompt gets a `[rec]` prefix. Work as usual:

```sh
[rec] # exarare note "Install nginx"
[rec] # dnf install -y nginx
[rec] # vi /etc/nginx/nginx.conf
[rec] # systemctl enable --now nginx
[rec] # exit
```

`exit` (or `exarare stop`) finishes the recording.

### Commands

| Command | Description |
|---|---|
| `exarare start [-n NAME] [--shell PATH] [--watch PATH]...` | Start a recording shell (bash or zsh; defaults to `$SHELL`) |
| `exarare stop` | Finish the current recording |
| `exarare status` | Show whether the current shell is being recorded |
| `exarare list` | List recorded sessions |
| `exarare diff [ID]` | Show the file changes of a session (default: the most recent) |
| `exarare gen md [ID] [-o FILE] [--lang en\|ja]` | Write a Markdown runbook for a session |
| `exarare note TEXT` | Insert a heading into the runbook |

### The runbook

```sh
exarare gen md -o runbook.md
```

The document has a fixed shape. Some of it is generated, some of it is a
template only a human can fill in — Exarare does not guess at intent:

| Chapter | Source |
|---|---|
| 1. Purpose | template: what, why, definition of done |
| 2. Overview of the work | generated: the steps in order, with counts |
| 3. Prerequisites | generated host and watched paths, plus a template |
| 4. Procedure | generated: one section per `exarare note`, with commands and diffs |
| 5. What changed | generated: every changed file |
| 6. Verification | generated candidates (`systemctl status`, `curl`, ...), expected results left blank |
| 7. Rollback | generated candidates, behind a warning that they are not a verified procedure |
| Appendix A | generated: every command that ran, including failures and retries |
| Appendix B | generated: how the document was produced |

Chapter 4 shows only the commands that worked, so trial and error does not
become an instruction; nothing is lost, because appendix A keeps everything.
Each `exarare note` starts a new step, and a session without notes is a single
step. A file is placed in the step whose commands name its path.

The fixed text is available in English (default) and Japanese, selected with
`--lang` or the `EXARARE_LANG` environment variable. Recorded commands, paths
and diffs are never translated.

### File changes

`exarare start` snapshots the watched directories — `/etc` unless you pass
`--watch` — and snapshots them again when the session ends. `exarare diff`
then reports added, removed, modified and re-permissioned files, with a
unified diff for text files:

```console
$ exarare diff
modified     /etc/nginx/nginx.conf
  --- a/etc/nginx/nginx.conf
  +++ b/etc/nginx/nginx.conf
  @@ -34,7 +34,7 @@
  -        listen       80;
  +        listen       8080;
permissions  /etc/nginx/conf.d/tls.conf
  mode 644 -> 600, owner 0:0 -> 0:0
```

Files that change on their own (`/etc/ld.so.cache`, `/etc/mtab`, lock files,
editor backups) are skipped. For secrets (`/etc/shadow`, `*.key`, `*.pem`,
anything under a `private/` directory) the change is reported but the content
is never stored. Content is stored only for text files up to 1 MiB.

### Where data is stored

Sessions are saved under `~/.local/share/exarare/sessions/<id>/`
(`$XDG_DATA_HOME/exarare` if set, or `$EXARARE_HOME`). The directory is
readable only by its owner, because recordings can contain secrets.

| File | Content |
|---|---|
| `meta.json` | Session name, host, user, shell, watched directories, start and end time |
| `events.jsonl` | One JSON event per line: commands, exit codes, notes |
| `snapshots/before.jsonl`, `snapshots/after.jsonl` | One entry per file: hash, size, mode, owner |
| `blobs/` | Content of the text files, addressed by hash |

### Limitations

- Supported shells are bash and zsh.
- While recording, `HISTCONTROL` and `HISTIGNORE` are cleared so that no
  command escapes the record.
- The bash hook uses the `DEBUG` trap, so it replaces other tools that also
  use it (e.g. bash-preexec).
- Commands run in a shell started inside the recording shell (e.g. `sudo -i`)
  are not recorded. Start the recording as root instead.
- Command output is not recorded yet.
- File changes are found by comparing snapshots of the watched directories, so
  a file edited outside them is not noticed. Pass `--watch` for other paths.

## Roadmap

- [x] Recording shell (bash / zsh hooks)
- [x] File snapshots and diffs
- [ ] Markdown runbook generation
- [ ] Probes for RPM / dnf, systemd, firewalld and users
- [ ] Ansible playbook generation
- [ ] Optional auditd backend
- [ ] CI on AlmaLinux / UBI 8, 9 and 10, static binaries and RPM packages

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
