//! Checks that the kind of work done on RHEL and AlmaLinux is recorded:
//! a dnf transaction and an edit under /etc.
//!
//! Runs only when EXARARE_EL_TESTS=1, because it touches the system. CI sets it
//! inside throwaway EL containers.

mod common;

use std::ffi::OsStr;

use common::{record_with, which, with_exarare_on_path};

fn enabled() -> bool {
    std::env::var("EXARARE_EL_TESTS").as_deref() == Ok("1")
}

/// In BaseOS on every supported release, and not part of the base image, so
/// installing it is a real change.
const PACKAGE: &str = "zip";

const SCRIPT: &str = r##"exarare step Install a package
dnf -y install zip
echo "# added by exarare smoke test" >> /etc/motd
systemctl --version >/dev/null
exit
"##;

#[test]
fn records_dnf_and_etc_edits() {
    if !enabled() {
        eprintln!("EXARARE_EL_TESTS is not 1, skipping");
        return;
    }
    let bash = which("bash").expect("bash not found");
    let recording = record_with(
        &bash,
        &with_exarare_on_path(SCRIPT),
        &[OsStr::new("--snapshot"), OsStr::new("/etc")],
    );
    let lines = recording.command_lines();

    assert!(
        lines.contains(&"dnf -y install zip"),
        "dnf command not recorded: {lines:?}"
    );
    assert!(
        lines.contains(&r##"echo "# added by exarare smoke test" >> /etc/motd"##),
        "redirection into /etc not recorded: {lines:?}"
    );
    assert_eq!(recording.steps, vec!["Install a package".to_string()]);

    let dnf = recording.find("dnf -y install zip").unwrap();
    assert_eq!(dnf.exit_code, Some(0), "dnf failed inside the container");

    // The snapshot of /etc must show the edit, with its diff, and the rpm
    // database must show the package.
    let report = recording.exarare(&["diff", &recording.session_id]);
    assert!(
        report
            .lines()
            .any(|l| l.starts_with("modified") && l.ends_with("/etc/motd")),
        "/etc/motd change not reported: {report}"
    );
    assert!(
        report.contains("+# added by exarare smoke test"),
        "diff body missing: {report}"
    );
    assert!(
        report
            .lines()
            .any(|l| l.starts_with("installed") && l.contains(PACKAGE)),
        "installed package not reported: {report}"
    );

    // The runbook shows it as an install of the step that asked for it. The
    // section is found by title: the harness prepends a PATH line, which
    // becomes the step before the first heading and shifts the numbering.
    let doc = recording.exarare(&["gen", "md", &recording.session_id]);
    let step = doc
        .split(". Install a package")
        .nth(1)
        .unwrap_or_else(|| panic!("step section missing: {doc}"));
    let step = step.split("## 5.").next().unwrap();
    assert!(step.contains("#### Packages"), "{step}");
    assert!(step.contains(&format!("- `{PACKAGE}`")), "{step}");

    let changed = doc.split("## 5. What changed").nth(1).unwrap();
    let changed = changed.split("## 6.").next().unwrap();
    assert!(changed.contains("**Installed**"), "{changed}");
    assert!(changed.contains(&format!("| `{PACKAGE}` |")), "{changed}");

    // Service state is reported either way: a container has no running systemd,
    // and the chapter must say that rather than imply nothing changed.
    let services = changed.split("### Services and firewall").nth(1).unwrap();
    assert!(
        services.contains("systemd did not answer")
            || services.contains("No service, firewall")
            || services.contains("**Enabled at boot**"),
        "{services}"
    );
}
