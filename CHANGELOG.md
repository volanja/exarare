# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the entries are drawn from the pull requests that landed in each release.
`taiki-e/create-gh-release-action` reads the section for the tag being released
and uses it as the release notes, so a missing section fails the release rather
than publishing without notes.

## [Unreleased]

## [0.1.0] - 2026-09-13

First release. Exarare records the work done while building a Linux server and
turns it into a Markdown runbook and an Ansible playbook, using rules rather
than a language model.

### Added

- Recording shell for bash and zsh, with `start`, `stop`, `status` and `list`.
  `exarare step` starts a step of the runbook and `exarare note` records a
  remark about the step in progress ([#19])
- File snapshots of the `--snapshot` directories, compared before and after, and
  reported by `exarare diff` with a unified diff for text files ([#11])
- Watching the `--watch` directories for files that were touched, recording
  paths but never content, which covers work outside the snapshots and files
  that were edited and put back ([#32])
- Markdown runbook via `exarare gen md`, in English or Japanese, with an
  optional Mermaid flowchart for the overview ([#16], [#29])
- Package changes read from the rpm database rather than from dnf's output,
  separating explicit installs from dependencies ([#20])
- Service, firewall, user and group changes, from systemd, firewalld,
  `/etc/passwd` and `/etc/group` ([#21])
- Ansible playbook via `exarare gen ansible`, checked against `ansible-lint`'s
  production profile in CI ([#23])
- Releases carrying static musl binaries and RPMs for x86_64 and aarch64
  ([#27]), and macOS binaries for development ([#38])
- CI across AlmaLinux and UBI 8, 9 and 10 ([#10]), coverage reporting ([#14]),
  and a documented set of local checks ([#25])

### Changed

- `--watch` now names the directories watched for touches; the directories whose
  content is snapshotted and diffed moved to `--snapshot` ([#32])
- Each appendix of the runbook has its own letter, after two shared one ([#33])
- The code-to-test ratio is no longer reported: it fell as tests were added,
  because most of them live beside the code they cover ([#28])

### Fixed

- An RPM checksum named a path inside the build tree, so verifying a downloaded
  file failed ([#36])
- A file touched during a step is placed in that step even when the watcher
  reports late, which FSEvents does ([#38])
- `exarare start` warns when no UTF-8 locale is set, because bash 3.2 mangles a
  non-ASCII step title in that case ([#26])
- CI clears the dnf cache before installing rather than after, so a point
  release no longer breaks the EL jobs ([#31])

[Unreleased]: https://github.com/volanja/exarare/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/volanja/exarare/releases/tag/v0.1.0
[#10]: https://github.com/volanja/exarare/pull/10
[#11]: https://github.com/volanja/exarare/pull/11
[#14]: https://github.com/volanja/exarare/pull/14
[#16]: https://github.com/volanja/exarare/pull/16
[#19]: https://github.com/volanja/exarare/pull/19
[#20]: https://github.com/volanja/exarare/pull/20
[#21]: https://github.com/volanja/exarare/pull/21
[#23]: https://github.com/volanja/exarare/pull/23
[#25]: https://github.com/volanja/exarare/pull/25
[#26]: https://github.com/volanja/exarare/pull/26
[#27]: https://github.com/volanja/exarare/pull/27
[#28]: https://github.com/volanja/exarare/pull/28
[#29]: https://github.com/volanja/exarare/pull/29
[#31]: https://github.com/volanja/exarare/pull/31
[#32]: https://github.com/volanja/exarare/pull/32
[#33]: https://github.com/volanja/exarare/pull/33
[#36]: https://github.com/volanja/exarare/pull/36
[#38]: https://github.com/volanja/exarare/pull/38
