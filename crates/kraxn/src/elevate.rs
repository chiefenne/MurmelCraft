//! Administrator rights for the steps that need them, and only for those:
//! the steps go as a file to `kraxn admin-run`, started with the system's
//! own prompt (sudo in a terminal, pkexec on a Linux desktop, the macOS
//! password dialog, Windows UAC). The rest of Kraxn runs as you.

use std::path::{Path, PathBuf};
use std::process::Command;

use kraxn_core::actions::{Elevator, Outcome, Step, run_steps};
use kraxn_core::diag::{Res, diag};
use kraxn_core::platform::Os;
use kraxn_core::tools::is_admin;

pub struct SystemElevator {
    pub os: Os,
    /// A window is asking (pkexec / dialog), not a terminal (sudo).
    pub gui: bool,
}

/// A folder only this user can read (the steps file names what root will
/// copy, so nobody else may change it).
fn private_dir() -> Res<PathBuf> {
    let d = std::env::temp_dir().join(format!(
        "kraxn-admin-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(&d)
        .map_err(|e| diag("KRX007", format!("{}: {e}", d.display())))?;
    Ok(d)
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
        .unwrap_or(false)
}

/// An AppleScript string literal.
fn applescript(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn command(os: Os, gui: bool, exe: &Path, input: &Path, output: &Path) -> Command {
    let (e, i, o) = (
        exe.display().to_string(),
        input.display().to_string(),
        output.display().to_string(),
    );
    match os {
        Os::Windows => {
            let mut c = Command::new("powershell");
            c.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Process -FilePath $env:KRAXN_EXE -ArgumentList $env:KRAXN_ARGS -Verb RunAs -Wait -WindowStyle Hidden",
            ]);
            c.env("KRAXN_EXE", &e);
            c.env("KRAXN_ARGS", format!("admin-run \"{i}\" \"{o}\""));
            c
        }
        Os::Macos if gui => {
            let script = format!(
                "do shell script quoted form of {} & \" admin-run \" & quoted form of {} & \" \" & quoted form of {} with administrator privileges",
                applescript(&e),
                applescript(&i),
                applescript(&o)
            );
            let mut c = Command::new("osascript");
            c.args(["-e", &script]);
            c
        }
        Os::Linux if gui && which("pkexec") => {
            let mut c = Command::new("pkexec");
            c.args([&e, "admin-run", &i, &o]);
            c
        }
        _ => {
            let mut c = Command::new("sudo");
            c.args([&e, "admin-run", &i, &o]);
            c
        }
    }
}

impl Elevator for SystemElevator {
    fn run_admin(&mut self, steps: &[Step]) -> Res<Vec<Outcome>> {
        if is_admin() {
            return Ok(run_steps(steps));
        }
        let exe = std::env::current_exe()
            .map_err(|e| diag("KRX008", format!("cannot find Kraxn itself: {e}")))?;
        let dir = private_dir()?;
        let input = dir.join("steps.json");
        let output = dir.join("outcomes.json");
        let text =
            serde_json::to_string(steps).map_err(|e| diag("KRX007", format!("steps: {e}")))?;
        std::fs::write(&input, text)
            .map_err(|e| diag("KRX007", format!("{}: {e}", input.display())))?;
        let status = command(self.os, self.gui, &exe, &input, &output).status();
        let read = std::fs::read_to_string(&output)
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<Outcome>>(&t).ok());
        let _ = std::fs::remove_dir_all(&dir);
        match (read, status) {
            (Some(o), _) => Ok(o),
            (None, Ok(s)) => Err(diag(
                "KRX008",
                format!(
                    "the steps that need administrator rights did not run (refused or cancelled, {s})"
                ),
            )),
            (None, Err(e)) => Err(diag(
                "KRX008",
                format!("could not ask for administrator rights: {e}"),
            )),
        }
    }
}

/// `kraxn admin-run STEPS OUTCOMES`: runs the steps (started elevated by
/// [`SystemElevator`]) and writes what they did.
pub fn admin_run(input: &Path, output: &Path) -> Res<()> {
    let text = std::fs::read_to_string(input)
        .map_err(|e| diag("KRX007", format!("{}: {e}", input.display())))?;
    let steps: Vec<Step> = serde_json::from_str(&text)
        .map_err(|e| diag("KRX010", format!("{}: {e}", input.display())))?;
    let out = run_steps(&steps);
    let text = serde_json::to_string(&out).map_err(|e| diag("KRX007", e.to_string()))?;
    std::fs::write(output, text).map_err(|e| diag("KRX007", format!("{}: {e}", output.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the elevated command passes the steps file to
    /// `admin-run` through each system's own prompt: sudo in a terminal,
    /// the macOS dialog with every path quoted, Windows' RunAs with the
    /// paths in environment variables (never in the script text). Breaks
    /// if a path with a quote or a space could change the command run as
    /// administrator.
    #[test]
    fn elevated_commands_quote_paths() {
        let exe = Path::new("/Apps/K \"x\"/kraxn");
        let (i, o) = (Path::new("/t/in.json"), Path::new("/t/out.json"));
        let c = command(Os::Linux, false, exe, i, o);
        assert_eq!(c.get_program(), "sudo");
        let args: Vec<_> = c.get_args().collect();
        assert_eq!(args[1], "admin-run");
        let c = command(Os::Macos, true, exe, i, o);
        let script = c.get_args().nth(1).unwrap().to_string_lossy().to_string();
        assert!(
            script.contains(r#"quoted form of "/Apps/K \"x\"/kraxn""#),
            "{script}"
        );
        assert!(script.ends_with("with administrator privileges"));
        let c = command(Os::Windows, true, exe, i, o);
        let script = c.get_args().nth(3).unwrap().to_string_lossy().to_string();
        assert!(
            !script.contains("kraxn") && script.contains("-Verb RunAs"),
            "{script}"
        );
    }

    /// Invariant: `admin-run` runs the steps of its file and writes their
    /// outcomes for the Kraxn that started it. Breaks if the two sides
    /// stop agreeing on the file formats.
    #[test]
    fn admin_run_writes_outcomes() {
        let d = std::env::temp_dir().join(format!("kraxn-adminrun-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let steps = vec![Step {
            title: "t".into(),
            admin: true,
            actions: vec![kraxn_core::actions::Action::MakeDir {
                path: d.join("made"),
            }],
        }];
        std::fs::write(d.join("in.json"), serde_json::to_string(&steps).unwrap()).unwrap();
        admin_run(&d.join("in.json"), &d.join("out.json")).unwrap();
        let out: Vec<Outcome> =
            serde_json::from_str(&std::fs::read_to_string(d.join("out.json")).unwrap()).unwrap();
        assert!(out[0].ok && d.join("made").is_dir());
        let _ = std::fs::remove_dir_all(&d);
    }
}
