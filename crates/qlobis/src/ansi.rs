//! Minimal ANSI styling (std-only).

pub fn red(s: &str) -> String {
    color(31, s)
}
pub fn green(s: &str) -> String {
    color(32, s)
}
pub fn yellow(s: &str) -> String {
    color(33, s)
}
pub fn bold(s: &str) -> String {
    format!("\x1b[1m{}\x1b[0m", s)
}

fn color(code: u8, s: &str) -> String {
    if !std::io::IsTerminal::is_terminal(&std::io::stderr()) && code == 31 {
        // keep colors even when piped — diagnostics look better; strip only
        // when NO_COLOR is set
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return s.to_string();
    }
    format!("\x1b[{}m{}\x1b[0m", code, s)
}
