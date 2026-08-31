//! Deterministic process fixture for executor boundary tests.

use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.get(1).map(String::as_str) == Some("--fixture-descendant") {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(path) = arguments.get(2) {
            let _ = std::fs::write(path, b"survived\n");
        }
        return;
    }
    if arguments.iter().any(|argument| argument == "--version") {
        println!("premonition-fake-agent 1.0.0");
        return;
    }
    let model = option_value(&arguments, "--model");
    let effort = option_value(&arguments, "--config");
    if model != Some("gpt-5.6-sol")
        || !matches!(
            effort,
            Some("model_reasoning_effort=\"low\"" | "model_reasoning_effort=\"medium\"")
        )
    {
        std::process::exit(64);
    }
    let mut prompt = String::new();
    if std::io::stdin().read_to_string(&mut prompt).is_err() {
        std::process::exit(70);
    }
    if prompt.contains("SCENARIO_CRASH") {
        std::process::exit(23);
    }
    if prompt.contains("SCENARIO_REQUIRE_MEDIUM")
        && effort != Some("model_reasoning_effort=\"medium\"")
    {
        std::process::exit(23);
    }
    if prompt.contains("SCENARIO_TIMEOUT") || prompt.contains("SCENARIO_CANCEL") {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    if prompt.contains("SCENARIO_DESCENDANT") {
        let sentinel = std::env::current_dir()
            .unwrap_or_default()
            .join("descendant-survived");
        if let Ok(executable) = std::env::current_exe() {
            let _ = std::process::Command::new(executable)
                .args(["--fixture-descendant", &sentinel.to_string_lossy()])
                .spawn();
        }
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

fn option_value<'a>(arguments: &'a [String], option: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == option)
        .map(|pair| pair[1].as_str())
}
