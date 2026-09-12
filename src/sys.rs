//! Running the system commands the probes read their state from.

use std::process::Command;

use anyhow::{Context, Result, bail};

/// Runs a command and returns its standard output.
pub fn output(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {program}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            stderr.lines().next().unwrap_or("no output").trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether the command exists on PATH, so a probe can say "not installed"
/// instead of reporting an error for every host that does not have it.
pub fn has_command(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_command_that_exists() {
        assert!(has_command("sh"));
        assert!(!has_command("exarare-no-such-command"));
    }

    #[test]
    fn reports_the_first_line_of_stderr() {
        let error = output("sh", &["-c", "echo boom >&2; exit 3"]).unwrap_err();
        assert!(format!("{error:#}").contains("boom"), "{error:#}");
    }
}
