//! `murmel-mcp`: the MCP server of MurmelCraft (the same as `murmel mcp`).

fn main() -> std::process::ExitCode {
    murmel_mcp::main(std::env::args_os().skip(1).collect())
}
