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
> Exarare is at an early stage. Command recording works today; file diffs,
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
| `exarare start [-n NAME] [--shell PATH]` | Start a recording shell (bash or zsh; defaults to `$SHELL`) |
| `exarare stop` | Finish the current recording |
| `exarare status` | Show whether the current shell is being recorded |
| `exarare list` | List recorded sessions |
| `exarare note TEXT` | Insert a heading into the runbook |

### Where data is stored

Sessions are saved under `~/.local/share/exarare/sessions/<id>/`
(`$XDG_DATA_HOME/exarare` if set, or `$EXARARE_HOME`). The directory is
readable only by its owner, because recordings can contain secrets.

| File | Content |
|---|---|
| `meta.json` | Session name, host, user, shell, start and end time |
| `events.jsonl` | One JSON event per line: commands, exit codes, notes |

### Limitations

- Supported shells are bash and zsh.
- While recording, `HISTCONTROL` and `HISTIGNORE` are cleared so that no
  command escapes the record.
- The bash hook uses the `DEBUG` trap, so it replaces other tools that also
  use it (e.g. bash-preexec).
- Commands run in a shell started inside the recording shell (e.g. `sudo -i`)
  are not recorded. Start the recording as root instead.
- Command output is not recorded yet.

## Roadmap

- [x] Recording shell (bash / zsh hooks)
- [ ] File snapshots and diffs
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
