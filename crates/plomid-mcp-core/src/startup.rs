//! Compact, secret-free ready panel for PLOMID-MCP server processes.

use std::io::IsTerminal;

const COPPER: &str = "\x1b[38;2;226;104;60m";
const INK: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Horizontal PLOMID wordmark shown above the MCP ready panel.
pub const LOGO_LINES: &[&str] = &[
    "    **      **********     **             ********      **        ***   **    **********",
    " ********            **    **           ***      ***    ****    *****   **             ***",
    "***    ***           **    **          **         **    ** ******* **   **    **        **",
    "***  *****  **********     **          **         **    **   **    **   **    **        **",
    "***  **     **             **           ***      ***    **         **   **    **       ***",
    "  *         **             **********     ********      **         **   **    **********  ",
];

/// Maximum rendered logo lines; the compactness test pins this.
pub const LOGO_MAX_LINES: usize = 10;

/// Render the server-ready panel without exposing database connection details.
#[must_use]
pub fn render_ready(transport: &str, endpoint: &str, color: bool) -> String {
    let (copper, green, ink, dim, bold, reset) = if color {
        (COPPER, GREEN, INK, DIM, BOLD, RESET)
    } else {
        ("", "", "", "", "", "")
    };
    debug_assert!(LOGO_LINES.len() <= LOGO_MAX_LINES);
    let mut logo = String::new();
    for line in LOGO_LINES {
        logo.push_str(&format!("  {copper}{bold}{line}{reset}\n"));
    }
    let ready_line = if transport == "STDIO" {
        format!("{dim}accepting MCP messages on{reset} {copper}STDIO{reset}")
    } else {
        format!("{dim}accepting connections on{reset} {copper}{endpoint}{reset}")
    };

    format!(
        "\n{logo}\n  {copper}{bold}PLOMID-MCP{reset}  {dim}Model Context Protocol server{reset}\n\
         +-----------------------------------------------------------+\n\
         | {green}ok{reset}  {ink}status   {reset}ready                                  |\n\
         |    {ink}transport{reset} {transport:<18}                       |\n\
         +-----------------------------------------------------------+\n\
           {ready_line}\n\n"
    )
}

/// Print the ready panel only for interactive terminals. Redirected output is
/// reserved for structured logs and MCP protocol messages.
pub fn report_ready(transport: &str, endpoint: &str) {
    let stderr = std::io::stderr();
    if stderr.is_terminal() {
        let color = std::env::var_os("NO_COLOR").is_none();
        eprint!("{}", render_ready(transport, endpoint, color));
    }
}

/// Record readiness for the connection-oriented STDIO transport.
pub fn log_stdio_ready() {
    tracing::info!("STDIO MCP server ready");
    report_ready("STDIO", "awaiting MCP client");
}

#[cfg(test)]
mod tests {
    use super::{LOGO_LINES, LOGO_MAX_LINES, render_ready};

    #[test]
    fn ready_panel_is_secret_free_and_plain_when_color_is_disabled() {
        let output = render_ready("STDIO", "awaiting MCP client", false);
        assert!(output.contains(LOGO_LINES[0]));
        assert!(output.contains("PLOMID-MCP"));
        assert!(output.contains("STDIO"));
        assert!(output.contains("accepting MCP messages on"));
        assert!(!output.contains('\u{1b}'));
        assert!(!output.contains("postgresql://"));
    }

    #[test]
    fn ready_panel_uses_the_ecosystem_palette_when_requested() {
        let output = render_ready("Streamable HTTP", "https://127.0.0.1:8443/mcp", true);
        assert!(output.contains("\u{1b}[38;2;226;104;60m"));
        assert!(output.contains("Streamable HTTP"));
        assert!(output.contains("https://127.0.0.1:8443/mcp"));
    }

    #[test]
    fn wordmark_is_ascii_and_stays_within_the_compact_line_limit() {
        assert!(LOGO_LINES.len() <= LOGO_MAX_LINES);
        assert!(LOGO_LINES.iter().all(|line| line.is_ascii()));
    }
}
