//! The MCP server of MurmelCraft (`murmel mcp`, `murmel-mcp`). It offers
//! the tools of the registry (`murmel-tools`) by their dotted ids to an
//! agent over standard input and output, on one headless session: the same
//! calls the window's clicks and keys make and `murmel call` runs. Paths in
//! tool inputs are confined to read and write roots, `view.screenshot`
//! returns its PNG as an MCP image, and the settings tools never write the
//! person's config files. Every result is the registry's envelope
//! (`murmelcraft.tool-result`) as text and as structured content. With
//! `--live` the same server drives the person's open window instead.
//!
//! The protocol is the MCP specification, revision 2025-11-25
//! (<https://modelcontextprotocol.io/specification/2025-11-25>): JSON-RPC
//! 2.0, one message per line on stdio.

mod roots;
mod server;

use std::ffi::OsString;
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use murmel_engine::{Session, Tokens, settings};
use serde_json::Value;

pub use crate::roots::Roots;
pub use crate::server::{
    MAX_INLINE_IMAGE_BYTES, PROTOCOL_VERSION, SHOT_DIR, Server, listing, rpc_error,
};

/// The largest message the server reads; a longer line is answered with an
/// error and skipped.
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;

pub const USAGE: &str = "Usage: murmel mcp --read-root DIR [--read-root DIR ...] --write-root DIR [--write-root DIR ...] [--live]
       murmel mcp --root DIR          (the same as --read-root DIR --write-root DIR)
       murmel mcp --live --root DIR   (drive the open MurmelCraft window)
       murmel-mcp ...                 (the same server as its own program)

Runs the MCP server on standard input and output (one JSON-RPC message per
line). Meshes are read below a read or a write root, screenshots are written
below a write root; relative paths start at the first write root, and a
screenshot without a path goes to screenshots/ there. Settings are read from
MURMEL_CONFIG_DIR or the platform's config folder and never written.

With --live the calls run in the person's open MurmelCraft window instead of
a headless session (the window must allow agents: Settings > Agents); the
roots are checked here all the same. The window listens at a private socket
(MURMEL_LIVE_SOCKET names another).";

/// The server's options.
#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    /// Drive the open window instead of a headless session.
    pub live: bool,
}

/// Parse the arguments; both kinds of root are required.
pub fn parse(args: &[OsString]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || {
            it.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{} needs a folder", flag.to_string_lossy()))
        };
        match flag.to_str() {
            Some("--read-root") => o.read_roots.push(value()?),
            Some("--write-root") => o.write_roots.push(value()?),
            Some("--live") => o.live = true,
            Some("--root") => {
                let d = value()?;
                o.read_roots.push(d.clone());
                o.write_roots.push(d);
            }
            _ => return Err(format!("unknown argument {}", flag.to_string_lossy())),
        }
    }
    if o.read_roots.is_empty() || o.write_roots.is_empty() {
        return Err("the MCP server needs at least one --read-root and one --write-root".into());
    }
    Ok(o)
}

/// A headless session with the person's settings (theme, colours, mouse
/// bindings), as the CLI starts one; the server drops its config folder.
pub fn session() -> Result<Session, String> {
    let mut s = Session::new(Tokens::builtin()?);
    if let Some(dir) = settings::config_dir() {
        let (st, nav, problems) = settings::load(&dir, cfg!(target_os = "macos"));
        for p in problems {
            eprintln!("murmel mcp: warning: {p}");
        }
        s.theme = st.theme;
        s.navigation = nav;
        s.view_cube = st.view_cube;
        s.agents = st.agents.enabled;
        s.defaults = st.defaults;
        s.formulas = st.formulas;
        s.keys = st.keys;
        s.help = st.help;
        s.memory_budget = st.memory.budget_gib;
        s.convert_large = st.memory.convert_large;
        murmel_engine::defaults::start_view(&mut s);
        if let Err(e) = s.set_colors(st.colors) {
            eprintln!("murmel mcp: warning: settings.toml colors: {e}");
        }
    }
    Ok(s)
}

/// Run the server on standard input and output.
pub fn main(args: Vec<OsString>) -> ExitCode {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        eprintln!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let started = parse(&args).and_then(|o| {
        let roots = Roots::new(&o.read_roots, &o.write_roots)?;
        if o.live {
            let socket = murmel_tools::live::socket_path()
                .ok_or("no folder for the live socket is known (set MURMEL_LIVE_SOCKET)")?;
            Ok(Server::live(socket, roots))
        } else {
            Ok(Server::new(session()?, roots))
        }
    });
    let server = match started {
        Ok(s) => s,
        Err(e) => {
            eprintln!("murmel mcp: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match serve(server, io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("murmel mcp: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Serve messages from `input` until it ends, answering on `output`.
pub fn serve(
    mut server: Server,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let mut frame = Vec::new();
    loop {
        frame.clear();
        let n = (&mut input)
            .take(MAX_REQUEST_BYTES as u64 + 1)
            .read_until(b'\n', &mut frame)?;
        if n == 0 {
            return Ok(());
        }
        let response = if n > MAX_REQUEST_BYTES {
            if frame.last() != Some(&b'\n') {
                input.skip_until(b'\n')?;
            }
            Some(rpc_error(
                Value::Null,
                -32600,
                "request exceeds the size limit",
            ))
        } else if frame.iter().all(u8::is_ascii_whitespace) {
            None
        } else {
            server.handle(&frame)
        };
        if let Some(r) = response {
            serde_json::to_writer(&mut output, &r)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Invariant: the server does not start without a read and a write
    /// root, and `--root` gives both. Dropping the check would let an agent
    /// read and write anywhere the person can.
    #[test]
    fn roots_are_required() {
        let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(parse(&a(&[])).is_err());
        assert!(parse(&a(&["--read-root", "x"])).is_err());
        assert!(parse(&a(&["--write-root", "x"])).is_err());
        assert!(parse(&a(&["--read-root"])).is_err());
        assert!(parse(&a(&["--bogus"])).is_err());
        let o = parse(&a(&["--root", "d"])).unwrap();
        assert_eq!(o.read_roots, vec![PathBuf::from("d")]);
        assert_eq!(o.write_roots, vec![PathBuf::from("d")]);
        assert!(!o.live);
        // --live still needs both roots
        assert!(parse(&a(&["--live"])).is_err());
        assert!(parse(&a(&["--live", "--root", "d"])).unwrap().live);
    }
}
