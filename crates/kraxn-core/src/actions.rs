//! Steps and their actions: everything Kraxn changes on the computer, as
//! data. A plan becomes a list of steps that the window and the text mode
//! show before anything runs; steps that need administrator rights go, as
//! a file, to `kraxn admin-run` started with those rights (sudo or pkexec,
//! the macOS password prompt, Windows UAC), so only they run elevated.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diag::{Diagnostic, Res, diag};

/// The lines that mark Kraxn's block in a shell start file.
pub const BLOCK_BEGIN: &str = "# >>> kraxn >>>";
pub const BLOCK_END: &str = "# <<< kraxn <<<";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "kebab-case")]
pub enum Action {
    MakeDir {
        path: PathBuf,
    },
    /// Copies a file; `executable` sets the mode to 0755 (0644 otherwise)
    /// on macOS and Linux.
    CopyFile {
        from: PathBuf,
        to: PathBuf,
        executable: bool,
    },
    /// Copies a folder (with its modes) to `to`, which must not exist.
    CopyDir {
        from: PathBuf,
        to: PathBuf,
    },
    /// `link` points to `target`, replacing what was there: a symbolic
    /// link on macOS and Linux, a hard link on Windows (a file only).
    Link {
        target: PathBuf,
        link: PathBuf,
    },
    Write {
        path: PathBuf,
        text: String,
        executable: bool,
    },
    RemoveFile {
        path: PathBuf,
    },
    RemoveDir {
        path: PathBuf,
    },
    /// Sets Kraxn's marked block in a shell start file (`None` removes it).
    ShellBlock {
        file: PathBuf,
        text: Option<String>,
    },
    /// Adds a folder to, or removes it from, the PATH that Windows keeps
    /// for the user or for the machine.
    WindowsPath {
        dir: PathBuf,
        machine: bool,
        add: bool,
    },
    /// Makes a folder readable for everybody (`chmod -R a+rX`).
    Readable {
        path: PathBuf,
    },
}

/// One step people see: a title, whether it needs administrator rights,
/// and its actions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub title: String,
    pub admin: bool,
    pub actions: Vec<Action>,
}

/// What a step did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub title: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Diagnostic>,
}

/// Runs steps that need administrator rights, with those rights (the
/// front ends start `kraxn admin-run`; tests run them directly).
pub trait Elevator {
    fn run_admin(&mut self, steps: &[Step]) -> Res<Vec<Outcome>>;
}

/// Runs admin steps as the current user: when Kraxn already runs with
/// administrator rights, or in tests.
pub struct NoElevation;

impl Elevator for NoElevation {
    fn run_admin(&mut self, steps: &[Step]) -> Res<Vec<Outcome>> {
        Ok(run_steps(steps))
    }
}

/// Runs each step's actions in order; a failed action ends its step and
/// the steps after it are not run (they build on it).
pub fn run_steps(steps: &[Step]) -> Vec<Outcome> {
    let mut out = Vec::new();
    for s in steps {
        let r = s.actions.iter().try_for_each(run);
        let failed = r.is_err();
        out.push(Outcome {
            title: s.title.clone(),
            ok: r.is_ok(),
            error: r.err(),
        });
        if failed {
            break;
        }
    }
    out
}

/// Runs the steps: the plain ones directly, each run of admin steps in
/// one go through `elevator` (one password prompt for several steps).
pub fn run_all(steps: &[Step], elevator: &mut dyn Elevator) -> Vec<Outcome> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < steps.len() {
        let admin = steps[i].admin;
        let mut j = i;
        while j < steps.len() && steps[j].admin == admin {
            j += 1;
        }
        let chunk = &steps[i..j];
        let res = if admin {
            match elevator.run_admin(chunk) {
                Ok(o) => o,
                Err(e) => vec![Outcome {
                    title: chunk[0].title.clone(),
                    ok: false,
                    error: Some(e),
                }],
            }
        } else {
            run_steps(chunk)
        };
        let failed = res.len() < chunk.len() || res.iter().any(|o| !o.ok);
        out.extend(res);
        if failed {
            break;
        }
        i = j;
    }
    out
}

fn io(path: &Path, e: impl std::fmt::Display) -> Diagnostic {
    let mut m = format!("{}: {e}", path.display());
    if m.contains("ermission denied") || m.contains("os error 13") || m.contains("os error 5)") {
        m.push_str(" (this needs administrator rights; choose the install for you alone, or let Kraxn ask for the rights)");
        return diag("KRX008", m);
    }
    diag("KRX007", m)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Res<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|e| io(path, e))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Res<()> {
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Res<()> {
    std::fs::create_dir_all(to).map_err(|e| io(to, e))?;
    for e in std::fs::read_dir(from).map_err(|e| io(from, e))? {
        let e = e.map_err(|e| io(from, e))?;
        let src = e.path();
        let dst = to.join(e.file_name());
        let ft = e.file_type().map_err(|e| io(&src, e))?;
        if ft.is_dir() {
            copy_dir(&src, &dst)?;
        } else if ft.is_symlink() {
            #[cfg(unix)]
            {
                let t = std::fs::read_link(&src).map_err(|e| io(&src, e))?;
                std::os::unix::fs::symlink(t, &dst).map_err(|e| io(&dst, e))?;
            }
        } else {
            std::fs::copy(&src, &dst).map_err(|e| io(&dst, e))?;
        }
    }
    Ok(())
}

fn remove_existing(path: &Path) -> Res<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir(path).map_err(|e| io(path, e)),
        Ok(_) => std::fs::remove_file(path).map_err(|e| io(path, e)),
        Err(_) => Ok(()),
    }
}

/// `text` with Kraxn's block set to `block` (or removed).
pub fn with_block(text: &str, block: Option<&str>) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim() == BLOCK_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == BLOCK_END {
            inside = false;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(b) = block {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(BLOCK_BEGIN);
        out.push('\n');
        out.push_str(b.trim_end());
        out.push('\n');
        out.push_str(BLOCK_END);
        out.push('\n');
    } else {
        while out.ends_with("\n\n") {
            out.pop();
        }
    }
    out
}

fn windows_path(dir: &Path, machine: bool, add: bool) -> Res<()> {
    let target = if machine { "Machine" } else { "User" };
    let ps = |script: &str, env: Option<&str>| -> Res<String> {
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        if let Some(v) = env {
            c.env("KRAXN_PATH", v);
        }
        let o = c
            .output()
            .map_err(|e| io(dir, format!("powershell: {e}")))?;
        if !o.status.success() {
            return Err(io(
                dir,
                String::from_utf8_lossy(&o.stderr).trim().to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let now = ps(
        &format!("[Environment]::GetEnvironmentVariable('Path', '{target}')"),
        None,
    )?;
    let d = dir.to_string_lossy().to_string();
    let mut parts: Vec<String> = now
        .split(';')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    let has = parts.iter().any(|p| p.eq_ignore_ascii_case(&d));
    if add == has {
        return Ok(());
    }
    if add {
        parts.push(d);
    } else {
        parts.retain(|p| !p.eq_ignore_ascii_case(&d));
    }
    ps(
        &format!("[Environment]::SetEnvironmentVariable('Path', $env:KRAXN_PATH, '{target}')"),
        Some(&parts.join(";")),
    )
    .map(|_| ())
}

/// Runs one action.
pub fn run(a: &Action) -> Res<()> {
    match a {
        Action::MakeDir { path } => std::fs::create_dir_all(path).map_err(|e| io(path, e)),
        Action::CopyFile {
            from,
            to,
            executable,
        } => {
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p).map_err(|e| io(p, e))?;
            }
            // a running program's file cannot be overwritten on every
            // system, but it can be replaced
            remove_existing(to)?;
            std::fs::copy(from, to).map_err(|e| io(to, e))?;
            set_mode(to, if *executable { 0o755 } else { 0o644 })
        }
        Action::CopyDir { from, to } => {
            if to.exists() {
                return Err(diag("KRX013", format!("{} exists already", to.display())));
            }
            copy_dir(from, to)
        }
        Action::Link { target, link } => {
            if let Some(p) = link.parent() {
                std::fs::create_dir_all(p).map_err(|e| io(p, e))?;
            }
            remove_existing(link)?;
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(target, link).map_err(|e| io(link, e))
            }
            #[cfg(not(unix))]
            {
                std::fs::hard_link(target, link).map_err(|e| io(link, e))
            }
        }
        Action::Write {
            path,
            text,
            executable,
        } => {
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p).map_err(|e| io(p, e))?;
            }
            std::fs::write(path, text).map_err(|e| io(path, e))?;
            set_mode(path, if *executable { 0o755 } else { 0o644 })
        }
        Action::RemoveFile { path } => remove_existing(path),
        Action::RemoveDir { path } => {
            if std::fs::symlink_metadata(path).is_err() {
                return Ok(());
            }
            std::fs::remove_dir_all(path).map_err(|e| io(path, e))
        }
        Action::ShellBlock { file, text } => {
            let old = std::fs::read_to_string(file).unwrap_or_default();
            if text.is_none() && !old.contains(BLOCK_BEGIN) {
                return Ok(());
            }
            let new = with_block(&old, text.as_deref());
            if new != old {
                std::fs::write(file, new).map_err(|e| io(file, e))?;
            }
            Ok(())
        }
        Action::WindowsPath { dir, machine, add } => windows_path(dir, *machine, *add),
        Action::Readable { path } => readable(path),
    }
}

#[cfg(unix)]
fn readable(path: &Path) -> Res<()> {
    use std::os::unix::fs::PermissionsExt;
    let m = std::fs::symlink_metadata(path).map_err(|e| io(path, e))?;
    if m.file_type().is_symlink() {
        return Ok(());
    }
    let mode = m.permissions().mode();
    let add = if m.is_dir() || mode & 0o100 != 0 {
        0o055
    } else {
        0o044
    };
    set_mode(path, (mode | add) & 0o7777)?;
    if m.is_dir() {
        for e in std::fs::read_dir(path).map_err(|e| io(path, e))? {
            readable(&e.map_err(|e| io(path, e))?.path())?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn readable(_path: &Path) -> Res<()> {
    Ok(())
}

/// The action as a line people can read (and, on macOS and Linux, mostly
/// run themselves).
pub fn describe(a: &Action) -> String {
    let q = |p: &Path| {
        let s = p.display().to_string();
        if s.contains(' ') {
            format!("\"{s}\"")
        } else {
            s
        }
    };
    match a {
        Action::MakeDir { path } => format!("mkdir -p {}", q(path)),
        Action::CopyFile { from, to, .. } => format!("cp {} {}", q(from), q(to)),
        Action::CopyDir { from, to } => format!("cp -R {} {}", q(from), q(to)),
        Action::Link { target, link } => format!("ln -sfn {} {}", q(target), q(link)),
        Action::Write { path, .. } => format!("write {}", q(path)),
        Action::RemoveFile { path } => format!("rm -f {}", q(path)),
        Action::RemoveDir { path } => format!("rm -rf {}", q(path)),
        Action::ShellBlock {
            file,
            text: Some(_),
        } => {
            format!("set the Kraxn block in {}", q(file))
        }
        Action::ShellBlock { file, text: None } => {
            format!("remove the Kraxn block from {}", q(file))
        }
        Action::WindowsPath { dir, machine, add } => format!(
            "{} {} {} the {} PATH",
            if *add { "add" } else { "remove" },
            q(dir),
            if *add { "to" } else { "from" },
            if *machine { "machine's" } else { "user's" }
        ),
        Action::Readable { path } => format!("chmod -R a+rX {}", q(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: Kraxn's block in a shell start file is added once,
    /// replaced in place and removed without touching the lines around
    /// it. Breaks if a second install appended a second block or an
    /// uninstall left the block (or ate the user's own lines).
    #[test]
    fn shell_block_is_replaced_and_removed() {
        let mine = "export EDITOR=vi\n";
        let one = with_block(mine, Some("export PATH=\"$HOME/.local/bin:$PATH\""));
        assert!(one.starts_with(mine));
        assert_eq!(one.matches(BLOCK_BEGIN).count(), 1);
        let two = with_block(&one, Some("export PATH=\"/x:$PATH\""));
        assert_eq!(two.matches(BLOCK_BEGIN).count(), 1);
        assert!(two.contains("/x:") && !two.contains(".local/bin"));
        assert_eq!(with_block(&two, None), mine);
    }

    struct Refuse;
    impl Elevator for Refuse {
        fn run_admin(&mut self, _: &[Step]) -> Res<Vec<Outcome>> {
            Err(diag("KRX008", "refused"))
        }
    }

    /// Invariant: plain steps run directly, admin steps go to the
    /// elevator, and when the rights are refused nothing after that step
    /// runs. Breaks if admin steps ran without the elevator (they would
    /// fail on a system folder) or later steps ran after a refusal (links
    /// to files that were never copied).
    #[test]
    fn admin_steps_go_to_the_elevator_and_stop_on_refusal() {
        let d = std::env::temp_dir().join(format!("kraxn-steps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let mk = |n: &str, admin| Step {
            title: n.into(),
            admin,
            actions: vec![Action::MakeDir { path: d.join(n) }],
        };
        let steps = [mk("a", false), mk("b", true), mk("c", false)];
        let out = run_all(&steps, &mut Refuse);
        assert_eq!(out.len(), 2);
        assert!(out[0].ok && !out[1].ok);
        assert_eq!(out[1].error.as_ref().unwrap().code, "KRX008");
        assert!(d.join("a").exists() && !d.join("b").exists() && !d.join("c").exists());
        let out = run_all(&steps, &mut NoElevation);
        assert!(out.iter().all(|o| o.ok) && out.len() == 3 && d.join("c").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
