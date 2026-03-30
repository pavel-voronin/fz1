use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[derive(Debug)]
pub struct EnrichmentResult {
    pub path: PathBuf,
    pub command_index: usize,
    pub output: String,
    pub status_text: Option<String>,
}

/// Spawn enrichment threads for all entries. Returns (Sender, Receiver).
/// The Sender is retained so additional entries can be enriched later
/// (e.g., after an inline edit) by calling `enrich_entry`.
pub fn spawn_enrichment(
    entries: &[crate::catalog::Entry],
) -> (Sender<EnrichmentResult>, Receiver<EnrichmentResult>) {
    let (tx, rx) = mpsc::channel();
    for entry in entries {
        enrich_entry(entry, &tx);
    }
    (tx, rx)
}

/// Spawn threads for a single entry's enrich_commands.
pub fn enrich_entry(entry: &crate::catalog::Entry, tx: &Sender<EnrichmentResult>) {
    for (cmd_index, cmd) in entry.enrich_commands.iter().enumerate() {
        let tx = tx.clone();
        let cmd = cmd.clone();
        let path = entry.path.clone();
        std::thread::spawn(move || {
            let (output, status_text) = run_command(&cmd);
            let _ = tx.send(EnrichmentResult {
                path,
                command_index: cmd_index,
                output,
                status_text,
            });
        });
    }
}

fn run_command(cmd: &str) -> (String, Option<String>) {
    let mut parts = cmd.splitn(2, ' ');
    let program = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.next().unwrap_or("").split_whitespace().collect();
    let mut command = Command::new(program);
    command.args(&args);
    configure_isolated_command(&mut command);
    match command.output() {
        Ok(out) => {
            if out.status.success() {
                (sanitize_command_output(&out.stdout), None)
            } else {
                let status_text = match out.status.code() {
                    Some(code) => format!("exit code: {}", code),
                    None => "terminated by signal".to_string(),
                };
                (String::new(), Some(status_text))
            }
        }
        Err(e) => (String::new(), Some(format!("failed to start: {}", e))),
    }
}

fn configure_isolated_command(command: &mut Command) {
    command.stdin(Stdio::null());

    #[cfg(unix)]
    unsafe {
        // Start the child in a new session so `/dev/tty` is unavailable.
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn sanitize_command_output(bytes: &[u8]) -> String {
    let mut sanitized = String::new();
    for ch in String::from_utf8_lossy(bytes).chars() {
        match ch {
            '\u{0008}' => {
                sanitized.pop();
            }
            '\r' => {}
            '\t' => sanitized.push_str("    "),
            '\n' => sanitized.push('\n'),
            control if control.is_control() => {}
            _ => sanitized.push(ch),
        }
    }
    sanitized.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{BodyLine, Entry};
    use std::path::PathBuf;
    use std::time::Duration;

    fn entry_with(cmds: Vec<&str>) -> Entry {
        Entry {
            filename: "t".into(),
            display_name: None,
            description: String::new(),
            body_lines: cmds
                .iter()
                .enumerate()
                .map(|(i, _)| BodyLine::Command(i))
                .collect(),
            templates: vec![],
            enrich_commands: cmds.into_iter().map(String::from).collect(),
            enriched_output: vec![String::new(); 1],
            enriched_status: vec![None; 1],
            category: "test".into(),
            path: PathBuf::from("test/t"),
        }
    }

    #[test]
    fn echo_command_delivers_output() {
        let entries = vec![entry_with(vec!["echo hello"])];
        let (_tx, rx) = spawn_enrichment(&entries);
        let r = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(r.output.contains("hello"));
        assert_eq!(r.status_text, None);
        assert_eq!(r.command_index, 0);
    }

    #[test]
    fn failed_command_reports_exit_code() {
        let entries = vec![entry_with(vec!["false"])];
        let (_tx, rx) = spawn_enrichment(&entries);
        let r = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(r.output, "");
        assert_eq!(r.status_text.as_deref(), Some("exit code: 1"));
    }

    #[test]
    fn no_commands_no_results() {
        let entries = vec![Entry {
            filename: "x".into(),
            display_name: None,
            description: String::new(),
            body_lines: vec![],
            templates: vec![],
            enrich_commands: vec![],
            enriched_output: vec![],
            enriched_status: vec![],
            category: "test".into(),
            path: PathBuf::from("test/x"),
        }];
        let (_tx, rx) = spawn_enrichment(&entries);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn enrich_single_entry_via_sender() {
        let (tx, rx) = mpsc::channel::<EnrichmentResult>();
        let entry = entry_with(vec!["echo world"]);
        enrich_entry(&entry, &tx);
        let r = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(r.output.contains("world"));
    }

    #[test]
    fn strips_terminal_control_sequences_from_output() {
        let raw = b"N\x08NA\x08AM\x08ME\x08E\nc\x08cp\x08p\n\tflag\r\n";
        assert_eq!(sanitize_command_output(raw), "NAME\ncp\n    flag");
    }

    #[cfg(unix)]
    #[test]
    fn isolated_commands_do_not_have_a_controlling_tty() {
        let mut command = Command::new("tty");
        configure_isolated_command(&mut command);

        let out = command.output().unwrap();

        assert!(!out.status.success());
    }
}
