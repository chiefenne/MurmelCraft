//! The live bridge ([docs/mcp.md](../../../docs/mcp.md), "Drive the open
//! window"): the window listens on a private local socket, and
//! `murmel mcp --live` sends it the tool calls it has checked against its
//! roots. The window runs them on its own session, so the person watches
//! the agent work and undoes its steps like their own.
//!
//! The socket is a Unix domain socket in a folder only the person's
//! account can open (`$XDG_RUNTIME_DIR/murmelcraft`, `$TMPDIR/murmelcraft`
//! on macOS; `MURMEL_LIVE_SOCKET` names another path). Windows has no
//! bridge yet (`LIVE001`).
//!
//! The wire is one JSON message per line, version 1:
//!
//! ```text
//! client: {"hello": "murmelcraft-live", "version": 1}
//! window: {"hello": "murmelcraft-live", "version": 1, "pid": 4242}
//! client: {"id": 1, "tool": "view.orbit", "input": {"dx": 40, "dy": 0}}
//! window: {"id": 1, "envelope": {"format": "murmelcraft.tool-result", ...}}
//! ```
//!
//! A malformed request is answered with `{"id": ..., "error": "..."}`; the
//! window closes the connection when the person stops the agents.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde_json::{Value, json};

use crate::Diagnostic;

pub const HELLO: &str = "murmelcraft-live";
pub const VERSION: u64 = 1;

/// The longest request line the window reads (as the MCP server's limit).
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;

/// How long the client waits for the window to answer one call (a
/// screenshot of a large mesh takes a while; a minimised window may not
/// draw until it is shown again).
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

/// Where the window listens and the client connects.
pub fn socket_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MURMEL_LIVE_SOCKET") {
        return Some(PathBuf::from(p));
    }
    let dir = if cfg!(target_os = "macos") {
        // per-user and private on macOS (/var/folders/.../T/)
        std::env::var_os("TMPDIR").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
    };
    let dir = match dir {
        Some(d) => d.join("murmelcraft"),
        None => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            std::env::temp_dir().join(format!("murmelcraft-{user}"))
        }
    };
    Some(dir.join("live.sock"))
}

fn no_window(path: &Path, why: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new(
        "LIVE001",
        format!(
            "no MurmelCraft window is listening at {}: {why}",
            path.display()
        ),
    )
}

fn broken(why: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new("LIVE002", format!("the window did not answer: {why}"))
}

/// Read one line of at most `max` bytes; `Ok(None)` at the end.
fn read_line(r: &mut impl BufRead, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let n = r.take(max as u64 + 1).read_until(b'\n', &mut line)?;
    if n == 0 {
        return Ok(None);
    }
    if line.len() > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "line exceeds the size limit",
        ));
    }
    Ok(Some(line))
}

fn send(w: &mut impl Write, v: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *w, v)?;
    w.write_all(b"\n")?;
    w.flush()
}

// ---- the window's side --------------------------------------------------

/// One call from an agent, waiting for the window to run it.
pub struct Job {
    pub tool: String,
    pub input: Value,
    reply: mpsc::Sender<Value>,
}

impl Job {
    /// Send the call's envelope back to the agent.
    pub fn answer(self, envelope: Value) {
        // the agent may have gone; nothing to tell then
        let _ = self.reply.send(envelope);
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    stop: AtomicBool,
    next: AtomicU64,
    #[cfg(unix)]
    conns: Mutex<Vec<(u64, std::os::unix::net::UnixStream)>>,
    #[cfg(not(unix))]
    conns: Mutex<Vec<(u64, ())>>,
}

/// The window's listener: agents connect, their calls queue as [`Job`]s
/// the window runs on its own thread. Dropping it closes the socket and
/// every connection.
pub struct Listener {
    path: PathBuf,
    jobs: mpsc::Receiver<Job>,
    shared: Arc<Shared>,
}

impl Listener {
    /// Listen at `path`; `wake` is called (from another thread) when a call
    /// arrives or an agent comes or goes, so the window draws a frame.
    pub fn start(path: &Path, wake: Wake) -> Result<Self, Diagnostic> {
        #[cfg(unix)]
        {
            unix::start(path, wake)
        }
        #[cfg(not(unix))]
        {
            let _ = wake;
            Err(no_window(path, "the live bridge needs macOS or Linux"))
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many agents are connected.
    pub fn connected(&self) -> usize {
        self.shared.conns.lock().map_or(0, |c| c.len())
    }

    /// The calls waiting to run, oldest first.
    pub fn jobs(&self) -> Vec<Job> {
        self.jobs.try_iter().collect()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        {
            if let Ok(mut conns) = self.shared.conns.lock() {
                for (_, c) in conns.drain(..) {
                    let _ = c.shutdown(std::net::Shutdown::Both);
                }
            }
            // wake the accept loop so it sees the stop
            let _ = std::os::unix::net::UnixStream::connect(&self.path);
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::fs::{DirBuilder, Permissions};
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};

    use super::*;

    /// The socket's folder: made private if new, refused if others can
    /// open it or it belongs to someone else.
    pub(super) fn private_dir(dir: &Path) -> Result<(), String> {
        if !dir.exists() {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let m = std::fs::metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if !m.is_dir() {
            return Err(format!("{} is not a folder", dir.display()));
        }
        if m.mode() & 0o077 != 0 {
            return Err(format!(
                "{} can be opened by other users (mode {:o}); make it private (chmod 700)",
                dir.display(),
                m.mode() & 0o777
            ));
        }
        // the person's home folder is theirs: the socket folder must be too
        if let Some(home) = std::env::var_os("HOME")
            && let Ok(h) = std::fs::metadata(home)
            && h.uid() != m.uid()
        {
            return Err(format!("{} belongs to another user", dir.display()));
        }
        Ok(())
    }

    pub(super) fn start(path: &Path, wake: Wake) -> Result<Listener, Diagnostic> {
        let fail = |why: String| Diagnostic::new("LIVE003", why);
        let dir = path.parent().unwrap_or(Path::new("."));
        private_dir(dir).map_err(fail)?;
        if let Ok(m) = std::fs::symlink_metadata(path) {
            if !m.file_type().is_socket() {
                return Err(fail(format!(
                    "{} exists and is not a socket",
                    path.display()
                )));
            }
            if UnixStream::connect(path).is_ok() {
                return Err(fail(format!(
                    "another MurmelCraft window already listens at {}",
                    path.display()
                )));
            }
            // left behind by a window that ended without closing it
            std::fs::remove_file(path).map_err(|e| fail(format!("{}: {e}", path.display())))?;
        }
        let listener =
            UnixListener::bind(path).map_err(|e| fail(format!("{}: {e}", path.display())))?;
        std::fs::set_permissions(path, Permissions::from_mode(0o600))
            .map_err(|e| fail(format!("{}: {e}", path.display())))?;
        let (tx, jobs) = mpsc::channel();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            next: AtomicU64::new(0),
            conns: Mutex::new(Vec::new()),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("murmel-live-accept".into())
            .spawn(move || accept(listener, s, tx, wake))
            .map_err(|e| fail(format!("cannot start the listener: {e}")))?;
        Ok(Listener {
            path: path.to_path_buf(),
            jobs,
            shared,
        })
    }

    fn accept(listener: UnixListener, shared: Arc<Shared>, tx: mpsc::Sender<Job>, wake: Wake) {
        for stream in listener.incoming() {
            if shared.stop.load(Ordering::SeqCst) {
                return;
            }
            let Ok(stream) = stream else { continue };
            let Ok(keep) = stream.try_clone() else {
                continue;
            };
            let id = shared.next.fetch_add(1, Ordering::SeqCst);
            match shared.conns.lock() {
                Ok(mut c) => c.push((id, keep)),
                Err(_) => return,
            }
            let (s, tx, w) = (shared.clone(), tx.clone(), wake.clone());
            let started = std::thread::Builder::new()
                .name("murmel-live-conn".into())
                .spawn(move || {
                    serve(stream, &tx, &w);
                    if let Ok(mut c) = s.conns.lock() {
                        c.retain(|(i, _)| *i != id);
                    }
                    w();
                });
            if started.is_err()
                && let Ok(mut c) = shared.conns.lock()
            {
                c.retain(|(i, _)| *i != id);
            }
            wake();
        }
    }

    fn serve(stream: UnixStream, tx: &mpsc::Sender<Job>, wake: &Wake) {
        let Ok(mut out) = stream.try_clone() else {
            return;
        };
        let mut input = BufReader::new(stream);
        // the handshake: a client of this wire and version, or nothing
        let Ok(Some(line)) = read_line(&mut input, MAX_REQUEST_BYTES) else {
            return;
        };
        let hello: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
        if hello["hello"] != HELLO || hello["version"] != VERSION {
            let _ = send(
                &mut out,
                &json!({"error": format!("expected {{\"hello\": \"{HELLO}\", \"version\": {VERSION}}}")}),
            );
            return;
        }
        let me = json!({"hello": HELLO, "version": VERSION, "pid": std::process::id()});
        if send(&mut out, &me).is_err() {
            return;
        }
        loop {
            let line = match read_line(&mut input, MAX_REQUEST_BYTES) {
                Ok(Some(l)) => l,
                Ok(None) => return,
                Err(e) => {
                    let _ = send(&mut out, &json!({"id": null, "error": e.to_string()}));
                    return;
                }
            };
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let req: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            let Some(tool) = req.get("tool").and_then(Value::as_str) else {
                if send(
                    &mut out,
                    &json!({"id": id, "error": "a request needs a tool"}),
                )
                .is_err()
                {
                    return;
                }
                continue;
            };
            let (reply, answer) = mpsc::channel();
            let job = Job {
                tool: tool.to_string(),
                input: req.get("input").cloned().unwrap_or(Value::Null),
                reply,
            };
            if tx.send(job).is_err() {
                return;
            }
            wake();
            // the window drops the job (and so the sender) when it stops
            let Ok(envelope) = answer.recv() else { return };
            if send(&mut out, &json!({"id": id, "envelope": envelope})).is_err() {
                return;
            }
        }
    }
}

// ---- the agent's side ---------------------------------------------------

/// The MCP server's end of the bridge: connects when a call comes (so the
/// window may open after the agent started), and again after the window
/// went away.
pub struct Client {
    path: PathBuf,
    #[cfg(unix)]
    conn: Option<(
        BufReader<std::os::unix::net::UnixStream>,
        std::os::unix::net::UnixStream,
    )>,
    next: u64,
    /// The pid of the window, once connected.
    pub window: Option<u64>,
}

impl Client {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            #[cfg(unix)]
            conn: None,
            next: 0,
            window: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run one call in the window; the envelope it answers, or `LIVE001`
    /// (no window) or `LIVE002` (the window went away or answered wrongly).
    pub fn call(&mut self, tool: &str, input: Value) -> Result<Value, Diagnostic> {
        #[cfg(unix)]
        {
            self.call_unix(tool, input)
        }
        #[cfg(not(unix))]
        {
            let _ = (tool, input, self.next);
            Err(no_window(
                &self.path,
                "the live bridge needs macOS or Linux",
            ))
        }
    }

    #[cfg(unix)]
    fn connect(&mut self) -> Result<(), Diagnostic> {
        use std::os::unix::net::UnixStream;
        let stream = UnixStream::connect(&self.path).map_err(|e| {
            no_window(
                &self.path,
                format!("{e} (open MurmelCraft and allow agents in its Settings)"),
            )
        })?;
        stream
            .set_read_timeout(Some(ANSWER_TIMEOUT))
            .map_err(broken)?;
        let mut out = stream.try_clone().map_err(broken)?;
        let mut input = BufReader::new(stream);
        send(&mut out, &json!({"hello": HELLO, "version": VERSION})).map_err(broken)?;
        let line = read_line(&mut input, usize::MAX / 2)
            .map_err(broken)?
            .ok_or_else(|| broken("the window closed the connection"))?;
        let hello: Value =
            serde_json::from_slice(&line).map_err(|e| broken(format!("bad handshake: {e}")))?;
        if hello["hello"] != HELLO || hello["version"] != VERSION {
            return Err(broken(format!("unexpected handshake {hello}")));
        }
        self.window = hello["pid"].as_u64();
        self.conn = Some((input, out));
        Ok(())
    }

    #[cfg(unix)]
    fn call_unix(&mut self, tool: &str, input: Value) -> Result<Value, Diagnostic> {
        self.next += 1;
        let req = json!({"id": self.next, "tool": tool, "input": input});
        // A connection the window closed since the last call (it was
        // stopped, or closed and opened again) fails on write or reads the
        // end at once; it is opened again once. The window closes a
        // connection only when it stops, without running the call, so the
        // call never runs twice; on a fresh connection the end is an error.
        for _ in 0..2 {
            let fresh = self.conn.is_none();
            if fresh {
                self.connect()?;
            }
            let Some((input, out)) = self.conn.as_mut() else {
                continue;
            };
            if send(out, &req).is_err() {
                self.conn = None;
                continue;
            }
            match read_line(input, usize::MAX / 2) {
                Ok(Some(line)) => {
                    let v: Value = serde_json::from_slice(&line)
                        .map_err(|e| broken(format!("bad answer: {e}")))?;
                    if v["id"] != json!(self.next) {
                        self.conn = None;
                        return Err(broken(format!("answer to another call: {v}")));
                    }
                    if let Some(e) = v.get("error").and_then(Value::as_str) {
                        return Err(broken(e.to_string()));
                    }
                    return v
                        .get("envelope")
                        .cloned()
                        .ok_or_else(|| broken("an answer without an envelope"));
                }
                Ok(None) if !fresh => self.conn = None,
                Ok(None) => {
                    self.conn = None;
                    return Err(broken("the window closed the connection"));
                }
                Err(e) => {
                    self.conn = None;
                    return Err(broken(e));
                }
            }
        }
        Err(broken("the window closed the connection"))
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("murmel-live-{}-{name}", std::process::id()))
            .join("sock");
        let _ = std::fs::remove_dir_all(&d);
        d.join("live.sock")
    }

    /// Run the calls waiting at `l` on `s`.
    fn pump(l: &Listener, s: &mut murmel_engine::Session) {
        for j in l.jobs() {
            let e = crate::call(&j.tool, j.input.clone(), s);
            j.answer(e.to_json());
        }
    }

    fn session() -> murmel_engine::Session {
        let mut s = murmel_engine::Session::new(murmel_engine::Tokens::builtin().unwrap());
        s.config_dir = None;
        s
    }

    /// Invariant: a call sent through the bridge runs on the listener's
    /// session and its envelope comes back, and the client says LIVE001
    /// when no window listens. A change to the wire that drops the
    /// envelope or the id would fail here; the MCP tests only see the
    /// server's side.
    #[test]
    fn calls_reach_the_window_and_answers_come_back() {
        let path = scratch("calls");
        let mut c = Client::new(path.clone());
        let e = c.call("view.get_camera", json!({})).unwrap_err();
        assert_eq!(e.code, "LIVE001");

        let l = Listener::start(&path, Arc::new(|| {})).unwrap();
        let worker = std::thread::spawn(move || {
            let mut c = Client::new(path);
            let a = c.call("view.standard", json!({"view": "top"}));
            let b = c.call("view.get_camera", json!({}));
            (a, b, c.window)
        });
        let mut s = session();
        while !worker.is_finished() {
            pump(&l, &mut s);
            std::thread::sleep(Duration::from_millis(5));
        }
        let (a, b, pid) = worker.join().unwrap();
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a["tool"], "view.standard");
        assert_eq!(a["status"], "finished");
        assert_eq!(b["tool"], "view.get_camera");
        assert_eq!(pid, Some(u64::from(std::process::id())));
        // the call ran on the listener's session
        assert_eq!(s.journal.undo.len(), 1);
    }

    /// Invariant: the socket is private (folder 0700, socket 0600), a
    /// second window does not take it over, a socket left behind is
    /// replaced, and stopping closes it so clients get LIVE001. Without
    /// the mode checks another user could drive the window.
    #[test]
    fn the_socket_is_private_and_owned_by_one_window() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("private");
        let l = Listener::start(&path, Arc::new(|| {})).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(mode(&path), 0o600);
        let e = Listener::start(&path, Arc::new(|| {})).err().unwrap();
        assert_eq!(e.code, "LIVE003");
        drop(l);
        assert!(!path.exists());
        assert_eq!(
            Client::new(path.clone())
                .call("view.get_camera", json!({}))
                .unwrap_err()
                .code,
            "LIVE001"
        );

        // a socket left behind by a window that crashed
        let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
        drop(stale);
        let l = Listener::start(&path, Arc::new(|| {})).unwrap();
        drop(l);

        // a folder others can open is refused
        let open = scratch("open");
        let dir = open.parent().unwrap();
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = Listener::start(&open, Arc::new(|| {})).err().unwrap();
        assert_eq!(e.code, "LIVE003");
    }

    /// Invariant: a client that does not say the handshake is turned
    /// away, a request without a tool gets an error and the connection
    /// lives on, and stopping the listener ends a waiting connection with
    /// LIVE002 for the agent. Without the handshake any program speaking
    /// lines could run tools; without the stop the person could not end
    /// an agent's session.
    #[test]
    fn strangers_are_refused_and_stop_ends_connections() {
        use std::os::unix::net::UnixStream;
        let path = scratch("stop");
        let l = Listener::start(&path, Arc::new(|| {})).unwrap();

        let mut raw = UnixStream::connect(&path).unwrap();
        raw.write_all(b"{\"hello\": \"other\"}\n").unwrap();
        let mut r = BufReader::new(raw.try_clone().unwrap());
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert!(line.contains("error"), "{line}");
        line.clear();
        assert_eq!(
            r.read_line(&mut line).unwrap(),
            0,
            "closed after the refusal"
        );

        let mut raw = UnixStream::connect(&path).unwrap();
        raw.write_all(b"{\"hello\": \"murmelcraft-live\", \"version\": 1}\n{\"id\": 7}\n")
            .unwrap();
        let mut r = BufReader::new(raw.try_clone().unwrap());
        line.clear();
        r.read_line(&mut line).unwrap();
        assert!(line.contains("pid"), "{line}");
        line.clear();
        r.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], 7);
        assert!(v["error"].is_string());

        // an agent waiting for an answer when the person stops
        let p = path.clone();
        let waiting = std::thread::spawn(move || Client::new(p).call("view.get_camera", json!({})));
        while l.connected() < 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
        // wait until its call is queued, then stop without answering
        let mut jobs = Vec::new();
        while jobs.is_empty() {
            jobs = l.jobs();
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(jobs);
        drop(l);
        let e = waiting.join().unwrap().unwrap_err();
        assert_eq!(e.code, "LIVE002");
    }
}
