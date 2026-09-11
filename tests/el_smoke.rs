//! Checks that the kind of work done on RHEL and AlmaLinux is recorded:
//! a dnf transaction and an edit under /etc.
//!
//! Runs only when EXARARE_EL_TESTS=1, because it touches the system. CI sets it
//! inside throwaway EL containers.

mod common;

use common::{record, which, with_exarare_on_path};

fn enabled() -> bool {
    std::env::var("EXARARE_EL_TESTS").as_deref() == Ok("1")
}

const SCRIPT: &str = r##"exarare note Install a package
dnf -y install zsh
echo "# added by exarare smoke test" >> /etc/motd
sed -i '$d' /etc/motd
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
    let session = record(&bash, &with_exarare_on_path(SCRIPT));
    let lines = session.command_lines();

    assert!(
        lines.contains(&"dnf -y install zsh"),
        "dnf command not recorded: {lines:?}"
    );
    assert!(
        lines.contains(&r##"echo "# added by exarare smoke test" >> /etc/motd"##),
        "redirection into /etc not recorded: {lines:?}"
    );
    assert_eq!(session.notes, vec!["Install a package".to_string()]);

    let dnf = session.find("dnf -y install zsh").unwrap();
    assert_eq!(dnf.exit_code, Some(0), "dnf failed inside the container");
}
