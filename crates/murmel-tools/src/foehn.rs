//! `field.foehn_derive`: FoehnCraft's own derived fields of the open
//! result (y⁺, wall shear stress, the wall treatment's values, the
//! turbulence features, the wall normal, the curvature correction),
//! computed by the `foehn` installed on the machine with `foehn post
//! derive` and read in as the next step of the same mesh, so the view,
//! the derived fields and the items stay ([`murmel_engine::foehn`]).
//! MurmelCraft neither links nor bundles FoehnCraft: it runs the program
//! as ffmpeg is run for movies, and confines its writing to the folder
//! of the file it writes (`--write-root`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use murmel_engine::Session;
use murmel_engine::foehn::{self, Model, Plan};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{need_fields, object, parse};
use crate::{Diagnostic, Run};

/// The `foehn` to run: the file `MURMEL_FOEHN` names when set (`var`),
/// else the first `foehn` on `PATH` (`path`), in Cargo's folder of
/// `home` (`~/.cargo/bin`, where `cargo install` puts it) or in
/// Homebrew's folders, since a Mac app started from the Finder does not
/// see the shell's PATH.
pub fn find_foehn(
    var: Option<OsString>,
    path: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, Diagnostic> {
    if let Some(v) = var.filter(|v| !v.is_empty()) {
        let p = PathBuf::from(v);
        return if p.is_file() {
            Ok(p)
        } else {
            Err(Diagnostic::new(
                "FIELD018",
                format!("MURMEL_FOEHN names {}, which is not a file", p.display()),
            ))
        };
    }
    let exe = if cfg!(windows) { "foehn.exe" } else { "foehn" };
    let mut dirs: Vec<PathBuf> = path
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(h) = home.filter(|h| !h.is_empty()) {
        dirs.push(PathBuf::from(h).join(".cargo/bin"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.into_iter()
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            Diagnostic::new(
                "FIELD018",
                "FoehnCraft's foehn was not found on PATH, in ~/.cargo/bin, /opt/homebrew/bin or /usr/local/bin",
            )
        })
}

/// The `foehn` of this machine (the environment's `MURMEL_FOEHN`,
/// `PATH` and `HOME`).
pub fn foehn_here() -> Result<PathBuf, Diagnostic> {
    find_foehn(
        std::env::var_os("MURMEL_FOEHN"),
        std::env::var_os("PATH"),
        std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")),
    )
}

/// The plan for the open result of `s`, or why there is none (FIELD019).
pub fn plan_of(s: &Session) -> Result<Plan, Diagnostic> {
    let m = s
        .mesh
        .as_ref()
        .ok_or_else(|| Diagnostic::new("VIEW001", "no mesh is open"))?;
    foehn::plan(&m.path).map_err(|e| Diagnostic::new("FIELD019", e.message()))
}

/// `foehn post derive` running for `models`, on a thread of the
/// operating system's own (a child process), so a window can wait for
/// it without stopping. It writes beside its file first and renames
/// that over the file once it is done, so a file open now, or one a
/// cancelled run leaves, is never half written.
pub struct Running {
    child: Child,
    pub models: Vec<Model>,
    pub file: PathBuf,
    part: PathBuf,
    pub started: Instant,
    /// The command, as a person would type it.
    pub command: Vec<String>,
}

impl Running {
    /// Start `program` on `plan` for `models` and the FoehnCraft fields
    /// `keep` the result has ([`foehn::kept`]); it may write only in the
    /// folder of the file it writes.
    pub fn start(
        program: &Path,
        plan: &Plan,
        models: &[Model],
        keep: &[&str],
    ) -> Result<Running, Diagnostic> {
        let stem = plan
            .file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "derived".into());
        let part = plan.file.with_file_name(format!("{stem}.writing.vtkhdf"));
        let mut args = plan.args(models, keep, &part);
        if let Some(dir) = plan.file.parent().filter(|d| !d.as_os_str().is_empty()) {
            args.push("--write-root".into());
            args.push(dir.display().to_string());
        }
        let mut command = vec![program.display().to_string()];
        command.extend(args.iter().cloned());
        let child = Command::new(program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                Diagnostic::new(
                    "FIELD020",
                    format!("{} could not be started: {e}", program.display()),
                )
            })?;
        Ok(Running {
            child,
            models: models.to_vec(),
            file: plan.file.clone(),
            part,
            started: Instant::now(),
            command,
        })
    }

    /// `None` while it runs; then whether it wrote its file.
    pub fn poll(&mut self) -> Option<Result<(), Diagnostic>> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) => Some(self.finish(status)),
            Err(e) => Some(Err(Diagnostic::new(
                "FIELD020",
                format!("foehn post derive could not be waited for: {e}"),
            ))),
        }
    }

    /// Wait until it is done.
    pub fn wait(mut self) -> Result<PathBuf, Diagnostic> {
        let status = self.child.wait().map_err(|e| {
            Diagnostic::new(
                "FIELD020",
                format!("foehn post derive could not be waited for: {e}"),
            )
        })?;
        self.finish(status)?;
        Ok(self.file)
    }

    /// Stop it, and remove what it wrote.
    pub fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.part);
    }

    fn finish(&mut self, status: std::process::ExitStatus) -> Result<(), Diagnostic> {
        use std::io::Read;
        let mut err = String::new();
        if let Some(mut e) = self.child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        if !status.success() {
            let _ = std::fs::remove_file(&self.part);
            // The last lines say why.
            let tail: Vec<&str> = err.lines().rev().take(6).collect();
            let why: Vec<&str> = tail.into_iter().rev().collect();
            return Err(Diagnostic::new(
                "FIELD020",
                format!(
                    "foehn post derive failed ({status}): {}",
                    if why.is_empty() {
                        "it printed nothing".to_string()
                    } else {
                        why.join(" / ")
                    }
                ),
            ));
        }
        if !self.part.is_file() {
            return Err(Diagnostic::new(
                "FIELD020",
                format!(
                    "foehn post derive finished but wrote no {}",
                    self.part.display()
                ),
            ));
        }
        std::fs::rename(&self.part, &self.file).map_err(|e| {
            Diagnostic::new(
                "FIELD020",
                format!(
                    "{} could not be renamed to {}: {e}",
                    self.part.display(),
                    self.file.display()
                ),
            )
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FoehnInput {
    fields: Vec<String>,
    /// A file `foehn post derive` already wrote: read it in.
    #[serde(default)]
    file: Option<PathBuf>,
    /// Where to write; left out, `derived(_<N>).vtkhdf` beside the result.
    #[serde(default)]
    output: Option<PathBuf>,
}

pub(crate) fn foehn_derive_schema() -> Value {
    object(
        json!({
            "fields": {"type": "array", "minItems": 1, "items": {"enum": Model::ALL.map(Model::key)}},
            "file": {"type": "string", "description": "a file foehn post derive already wrote for this result: read it in, run nothing"},
            "output": {"type": "string", "description": "the .vtkhdf to write; left out, derived.vtkhdf (derived_<N>.vtkhdf) beside the result"}
        }),
        &["fields"],
    )
}

/// The fields of `models` the open result lacks.
fn lacking(s: &Session, models: &[Model]) -> Vec<&'static str> {
    let Some(set) = s.fields() else {
        return models.iter().flat_map(|m| m.fields()).copied().collect();
    };
    models
        .iter()
        .flat_map(|m| m.fields())
        .copied()
        .filter(|f| set.get(f).is_none())
        .collect()
}

/// The FoehnCraft keys of the fields the open result's file has
/// ([`foehn::kept`]), so a file that replaces it keeps them.
pub fn kept_of(s: &Session) -> Vec<&'static str> {
    let Some(set) = s.fields() else {
        return Vec::new();
    };
    let made: Vec<&str> = set.made().iter().map(|m| m.field()).collect();
    foehn::kept(set.names().into_iter().filter(|n| !made.contains(n)))
}

pub(crate) fn field_foehn_derive(input: Value, s: &mut Session) -> Run {
    derive_with(input, s, foehn_here)
}

/// `field.foehn_derive` with the `foehn` that `program` finds.
fn derive_with(
    input: Value,
    s: &mut Session,
    program: impl FnOnce() -> Result<PathBuf, Diagnostic>,
) -> Run {
    let i: FoehnInput = parse(input)?;
    need_fields(s)?;
    let mut models: Vec<Model> = Vec::new();
    for f in &i.fields {
        let m = Model::parse(f).ok_or_else(|| {
            Diagnostic::new(
                "TOOL002",
                format!(
                    "no FoehnCraft field {f}; there are {}",
                    Model::ALL.map(Model::key).join(", ")
                ),
            )
        })?;
        if !models.contains(&m) {
            models.push(m);
        }
    }
    if i.file.is_some() && i.output.is_some() {
        return Err(Diagnostic::new(
            "TOOL002",
            "give file (already written) or output (to write), not both",
        ));
    }
    let before = lacking(s, &models);
    if before.is_empty() {
        return Ok(
            json!({"ran": false, "in_file": models.iter().flat_map(|m| m.fields()).collect::<Vec<_>>()}),
        );
    }
    let (file, ran, command, seconds) = match i.file {
        Some(f) => {
            if !f.is_file() {
                return Err(Diagnostic::new(
                    "FIELD020",
                    format!("{} does not exist", f.display()),
                ));
            }
            (f, false, Vec::new(), 0.0)
        }
        None => {
            let mut plan = plan_of(s)?;
            if let Some(o) = i.output {
                plan.file = o;
            }
            let program = program()?;
            let run = Running::start(&program, &plan, &models, &kept_of(s))?;
            let command = run.command.clone();
            let started = run.started;
            let file = run.wait()?;
            (file, true, command, started.elapsed().as_secs_f64())
        }
    };
    s.open_same_step(&file)
        .map_err(|e| Diagnostic::new(e.code, e.message))?;
    let after = lacking(s, &models);
    let added: Vec<&str> = before
        .iter()
        .copied()
        .filter(|f| !after.contains(f))
        .collect();
    Ok(json!({
        "ran": ran,
        "file": file,
        "command": command,
        "seconds": seconds,
        "added": added,
        // A field FoehnCraft leaves out: one its case has no model for
        // (the features without a turbulence model, the wall normal
        // without elliptic blending).
        "not_written": after,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: `foehn` is found where MURMEL_FOEHN points, else on
    /// PATH, else in ~/.cargo/bin; a MURMEL_FOEHN that is no file is
    /// refused rather than passed over. Breaks when the search order
    /// changes.
    #[test]
    fn foehn_is_found() {
        let dir = std::env::temp_dir().join(format!("murmel-find-foehn-{}", std::process::id()));
        let bin = dir.join("bin");
        let cargo = dir.join("home/.cargo/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&cargo).unwrap();
        let exe = if cfg!(windows) { "foehn.exe" } else { "foehn" };
        std::fs::write(cargo.join(exe), b"").unwrap();
        let home = Some(dir.join("home").into_os_string());
        assert_eq!(
            find_foehn(None, Some(bin.clone().into_os_string()), home.clone()).unwrap(),
            cargo.join(exe)
        );
        std::fs::write(bin.join(exe), b"").unwrap();
        assert_eq!(
            find_foehn(None, Some(bin.clone().into_os_string()), home.clone()).unwrap(),
            bin.join(exe)
        );
        let named = Some(cargo.join(exe).into_os_string());
        assert_eq!(
            find_foehn(named, Some(bin.into_os_string()), home.clone()).unwrap(),
            cargo.join(exe)
        );
        let e = find_foehn(Some(dir.join("nothing").into_os_string()), None, home).unwrap_err();
        assert_eq!(e.code, "FIELD018");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures/vtkhdf")
            .join(name)
    }

    fn opened(path: &Path) -> Session {
        let mut s = Session::new(murmel_engine::Tokens::builtin().unwrap());
        s.config_dir = None;
        let e = crate::call("mesh.open", json!({ "path": path }), &mut s);
        assert!(e.ok(), "{:?}", e.diagnostics);
        s
    }

    /// A stand-in for foehn: a shell script that notes its arguments and
    /// writes `body` (with $FILE the --file it was given).
    #[cfg(unix)]
    fn fake(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("foehn");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\necho \"$@\" > \"{}\"\nFILE=\"\"\nwhile [ $# -gt 0 ]; do if [ \"$1\" = --file ]; then FILE=\"$2\"; fi; shift; done\n{body}\n",
                dir.join("args.txt").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// Invariant: field.foehn_derive runs foehn post derive on the
    /// result's case and restart file for the fields it lacks, asking
    /// again for the FoehnCraft fields the result has, writes beside the
    /// file and renames it over derived.vtkhdf when foehn is done, then
    /// reads it in as a step of the same mesh; a failing foehn says why
    /// and leaves no file; fields the result has are not asked for; a
    /// file of another mesh is refused and the session stays. Breaks when
    /// the fields kept are left out (they would vanish with the step).
    #[cfg(unix)]
    #[test]
    fn foehn_post_derive_is_run_and_read_in() {
        let dir = std::env::temp_dir().join(format!("murmel-foehn-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let result = dir.join("fields.vtkhdf");
        std::fs::copy(fixture("channel_fields.vtkhdf"), &result).unwrap();
        let mut s = opened(&result);
        let none = || -> Result<PathBuf, Diagnostic> { panic!("foehn is not run") };
        // what the file has is not asked for
        let v = derive_with(json!({"fields": ["y-plus"]}), &mut s, none).unwrap();
        assert_eq!(v["ran"], false);
        // no restart file: FIELD019
        let e = derive_with(json!({"fields": ["turbulence-wall"]}), &mut s, none).unwrap_err();
        assert_eq!(e.code, "FIELD019", "{}", e.message);
        std::fs::write(dir.join("restart.h5"), b"").unwrap();
        std::fs::write(dir.join("case.resolved.toml"), b"").unwrap();
        // a foehn that fails
        let bad = fake(&dir, "echo 'CASE003 no turbulence model' >&2\nexit 1");
        let e =
            derive_with(json!({"fields": ["turbulence-wall"]}), &mut s, || Ok(bad)).unwrap_err();
        assert_eq!(e.code, "FIELD020");
        assert!(
            e.message.contains("CASE003 no turbulence model"),
            "{}",
            e.message
        );
        assert!(!dir.join("derived.vtkhdf").exists());
        // a foehn that writes the result again (without the wall fields)
        let src = fixture("channel_fields.vtkhdf");
        let good = fake(&dir, &format!("cp \"{}\" \"$FILE\"", src.display()));
        let v = derive_with(json!({"fields": ["turbulence-wall"]}), &mut s, || Ok(good)).unwrap();
        assert_eq!(v["ran"], true);
        assert_eq!(
            v["not_written"],
            json!(["friction_velocity", "y_star", "blending"])
        );
        let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
        assert!(
            args.contains("--fields wall-shear-stress,y-plus,turbulence-wall"),
            "{args}"
        );
        assert!(args.contains("derived.writing.vtkhdf"), "{args}");
        assert!(
            args.contains(&format!("--write-root {}", dir.display())),
            "{args}"
        );
        assert!(dir.join("derived.vtkhdf").is_file());
        assert!(!dir.join("derived.writing.vtkhdf").exists());
        assert_eq!(s.mesh.as_ref().unwrap().path, dir.join("derived.vtkhdf"));
        // a file of another mesh
        let other = fixture("box_latest.vtkhdf");
        let e = derive_with(
            json!({"fields": ["turbulence-wall"], "file": other}),
            &mut s,
            none,
        )
        .unwrap_err();
        assert_eq!(e.code, "FIELD020", "{}", e.message);
        assert_eq!(s.mesh.as_ref().unwrap().path, dir.join("derived.vtkhdf"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
