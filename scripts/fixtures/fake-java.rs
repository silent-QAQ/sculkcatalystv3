// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

//! Tiny Java-compatible process used by local lifecycle E2E tests.
//! It only implements `java -version` and the stdin/stdout behaviour that the
//! server process actor needs; it is never shipped with production builds.

use std::io::{self, BufRead, Write};

fn main() {
    if std::env::args().skip(1).any(|argument| argument == "-version") {
        eprintln!("openjdk version \"21.0.7\" 2026-04-15 LTS");
        return;
    }

    println!("[Server thread/INFO]: Done (0.123s)! For help, type \"help\"");
    let _ = io::stdout().flush();
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().eq_ignore_ascii_case("stop") {
            println!("[Server thread/INFO]: Stopping server");
            let _ = io::stdout().flush();
            return;
        }
        println!("[Server thread/INFO]: executed {}", line.trim());
        let _ = io::stdout().flush();
    }
}
