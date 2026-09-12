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

/// Whether the character encoding of the locale is UTF-8, in the order the
/// C library resolves it: LC_ALL, then LC_CTYPE, then LANG.
pub fn locale_is_utf8() -> bool {
    let value = |name: &str| std::env::var(name).ok();
    utf8_locale(&[value("LC_ALL"), value("LC_CTYPE"), value("LANG")])
}

/// Split out so it can be tested without touching the process environment.
fn utf8_locale(values: &[Option<String>]) -> bool {
    for value in values.iter().flatten() {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let lower = value.to_ascii_lowercase();
        return lower.contains("utf-8") || lower.contains("utf8");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    #[test]
    fn recognises_a_utf8_locale() {
        assert!(utf8_locale(&[var("en_US.UTF-8")]));
        assert!(utf8_locale(&[var("ja_JP.utf8")]));
        assert!(utf8_locale(&[var("C.UTF-8")]));
        assert!(!utf8_locale(&[var("C")]));
        assert!(!utf8_locale(&[var("POSIX")]));
        assert!(!utf8_locale(&[None, None, None]));
    }

    #[test]
    fn the_first_variable_that_is_set_decides() {
        // LC_ALL overrides LANG, even when LANG looks better.
        assert!(!utf8_locale(&[var("C"), None, var("en_US.UTF-8")]));
        assert!(utf8_locale(&[None, var("en_US.UTF-8"), var("C")]));
        // An empty value is not a setting.
        assert!(utf8_locale(&[var("  "), var("en_US.UTF-8")]));
    }

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
