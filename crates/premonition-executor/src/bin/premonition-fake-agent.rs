//! Deterministic process fixture for executor boundary tests.

use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    if std::env::args().any(|argument| argument == "--version") {
        println!("premonition-fake-agent 1.0.0");
        return;
    }
    let mut prompt = String::new();
    if std::io::stdin().read_to_string(&mut prompt).is_err() {
        std::process::exit(70);
    }
    if prompt.contains("SCENARIO_CRASH") {
        std::process::exit(23);
    }
    if prompt.contains("SCENARIO_TIMEOUT") || prompt.contains("SCENARIO_CANCEL") {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    if prompt.contains("SCENARIO_OVERFLOW") {
        let block = vec![b'x'; 600 * 1024];
        let _ = std::io::stdout().write_all(&block);
        return;
    }
    if prompt.contains("SCENARIO_HOSTILE") {
        println!("\x1b[31mnot json\x1b[0m");
        return;
    }
    if prompt.contains("SCENARIO_MALFORMED") {
        println!(
            r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"not-json"}}}}"#
        );
        return;
    }
    println!(
        r#"{{"type":"thread.started","thread_id":"fixture"}}
{{"type":"item.completed","item":{{"type":"agent_message","text":"{{\"patch\":\"diff --git a/a.txt b/a.txt\\n--- a/a.txt\\n+++ b/a.txt\\n@@ -1 +1 @@\\n-old\\n+new\\n\",\"rationale\":\"Replace the incorrect value.\"}}"}}}}"#
    );
}
