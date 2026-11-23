//! The read and write roots of the MCP server. A tool input that names a
//! file (the fields a tool declares in `reads` and `outputs`) is resolved
//! and checked before the call runs: reads below a read or a write root,
//! writes below a write root. Relative paths resolve from the first write
//! root, never from the server's working folder.

use std::path::{Component, Path, PathBuf};

use murmel_tools::Diagnostic;

#[derive(Clone, Debug)]
pub struct Roots {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

fn canonical_dir(p: &Path, what: &str) -> Result<PathBuf, String> {
    let c = p
        .canonicalize()
        .map_err(|e| format!("{what} {}: {e}", p.display()))?;
    if !c.is_dir() {
        return Err(format!("{what} {} is not a folder", p.display()));
    }
    Ok(c)
}

fn refused(path: &Path, kind: &str) -> Diagnostic {
    Diagnostic::new(
        "MCP001",
        format!(
            "{} is outside the MCP server's {kind} roots",
            path.display()
        ),
    )
}

fn unresolved(path: &Path, why: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new("MCP002", format!("{}: {why}", path.display()))
}

impl Roots {
    /// Both lists must name existing folders; the server refuses to start
    /// otherwise.
    pub fn new(read: &[PathBuf], write: &[PathBuf]) -> Result<Self, String> {
        if read.is_empty() || write.is_empty() {
            return Err(
                "the MCP server needs at least one --read-root and one --write-root".into(),
            );
        }
        Ok(Self {
            read: read
                .iter()
                .map(|p| canonical_dir(p, "read root"))
                .collect::<Result<_, _>>()?,
            write: write
                .iter()
                .map(|p| canonical_dir(p, "write root"))
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn read_roots(&self) -> &[PathBuf] {
        &self.read
    }

    pub fn write_roots(&self) -> &[PathBuf] {
        &self.write
    }

    /// The first write root, where relative paths and unnamed screenshots go.
    pub fn home(&self) -> &Path {
        &self.write[0]
    }

    fn absolute(&self, p: &Path) -> PathBuf {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.home().join(p)
        }
    }

    /// A file or folder a call reads: it must exist, and its real path
    /// (links followed) must lie below a read or a write root.
    pub fn read(&self, p: &Path) -> Result<PathBuf, Diagnostic> {
        let c = self
            .absolute(p)
            .canonicalize()
            .map_err(|e| unresolved(p, e))?;
        if self
            .read
            .iter()
            .chain(&self.write)
            .any(|r| c.starts_with(r))
        {
            Ok(c)
        } else {
            Err(refused(p, "read"))
        }
    }

    /// A file a call writes: its folder must exist and lie below a write
    /// root (links followed), the name must be a plain file name, and an
    /// existing file there must not be a link (which could point out).
    pub fn write(&self, p: &Path) -> Result<PathBuf, Diagnostic> {
        let abs = self.absolute(p);
        let name = match abs.components().next_back() {
            Some(Component::Normal(n)) => n.to_os_string(),
            _ => return Err(unresolved(p, "not a file name")),
        };
        let parent = abs
            .parent()
            .ok_or_else(|| unresolved(p, "no folder"))?
            .canonicalize()
            .map_err(|e| unresolved(p, format!("its folder: {e}")))?;
        if !self.write.iter().any(|r| parent.starts_with(r)) {
            return Err(refused(p, "write"));
        }
        let target = parent.join(name);
        if let Ok(m) = std::fs::symlink_metadata(&target) {
            if m.file_type().is_symlink() {
                return Err(refused(p, "write (it is a link)"));
            }
            if m.is_dir() {
                return Err(unresolved(p, "is a folder"));
            }
        }
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("murmel-mcp-roots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("read/sub")).unwrap();
        std::fs::create_dir_all(d.join("write")).unwrap();
        std::fs::create_dir_all(d.join("outside")).unwrap();
        std::fs::write(d.join("read/sub/a.vtu"), "x").unwrap();
        std::fs::write(d.join("outside/b.vtu"), "x").unwrap();
        d
    }

    /// Invariant: reads stay below the read or write roots and writes below
    /// the write roots, `..` cannot climb out, relative paths start at the
    /// first write root, and missing roots stop the server. Breaks when a
    /// check compares the text of a path instead of its real path (then
    /// `read/../outside` passes), and the stdio test only tries one case.
    #[test]
    fn paths_stay_inside_the_roots() {
        let d = scratch("inside");
        let r = Roots::new(&[d.join("read")], &[d.join("write")]).unwrap();
        assert!(r.read(&d.join("read/sub/a.vtu")).is_ok());
        assert_eq!(
            r.read(&d.join("read/../outside/b.vtu")).unwrap_err().code,
            "MCP001"
        );
        assert_eq!(r.read(&d.join("outside/b.vtu")).unwrap_err().code, "MCP001");
        assert_eq!(r.read(&d.join("read/nope.vtu")).unwrap_err().code, "MCP002");
        assert!(
            r.write(Path::new("shot.png"))
                .unwrap()
                .starts_with(r.home())
        );
        assert_eq!(
            r.write(&d.join("read/shot.png")).unwrap_err().code,
            "MCP001"
        );
        assert_eq!(
            r.write(Path::new("../outside/shot.png")).unwrap_err().code,
            "MCP001"
        );
        assert_eq!(
            r.write(Path::new("missing/shot.png")).unwrap_err().code,
            "MCP002"
        );
        assert_eq!(r.write(Path::new("..")).unwrap_err().code, "MCP002");
        assert!(Roots::new(&[], &[d.join("write")]).is_err());
        assert!(Roots::new(&[d.join("read")], &[d.join("gone")]).is_err());
    }

    /// Invariant: a link inside a root that points out is followed and
    /// refused, for reads and for writes. Breaks when the check stops
    /// following links.
    #[cfg(unix)]
    #[test]
    fn links_out_of_a_root_are_refused() {
        let d = scratch("links");
        let r = Roots::new(&[d.join("read")], &[d.join("write")]).unwrap();
        std::os::unix::fs::symlink(d.join("outside/b.vtu"), d.join("read/link.vtu")).unwrap();
        std::os::unix::fs::symlink(d.join("outside"), d.join("write/out")).unwrap();
        std::os::unix::fs::symlink(d.join("outside/b.vtu"), d.join("write/shot.png")).unwrap();
        assert_eq!(r.read(&d.join("read/link.vtu")).unwrap_err().code, "MCP001");
        assert_eq!(
            r.write(Path::new("out/shot.png")).unwrap_err().code,
            "MCP001"
        );
        assert_eq!(r.write(Path::new("shot.png")).unwrap_err().code, "MCP001");
    }
}
