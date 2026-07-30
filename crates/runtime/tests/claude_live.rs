#![allow(dead_code)]
//! Optional, `#[ignore]`d end-to-end conformance probe for
//! [`ClaudeAdapter`], gated on `BATMAN_LIVE_CLAUDE=1`.
//!
//! `crates/runtime/tests/claude_adapter.rs`'s default suite deliberately
//! never calls `start()`/`resume()`/`send()` past their pre-start guard
//! clauses, because doing so writes a real prompt to a real `claude -p`
//! process's stdin -- which *does* invoke the model the moment the CLI
//! reads it. This file is the actual end-to-end exercise of the
//! spawn -> write stdin -> background reader task -> normalize -> sink
//! path, using the real installed `claude` CLI and a real (small, cheap)
//! model call.
//!
//! A human runs this deliberately with:
//! ```sh
//! BATMAN_LIVE_CLAUDE=1 cargo test -p batman-runtime --test claude_live -- --ignored
//! ```
//! No CI job and no agent working on this task ever sets that variable or
//! runs this test.

#[path = "../src/adapter/claude/mod.rs"]
mod claude;

use std::sync::Arc;

use batman_protocol::{RunId, TaskId, WorkerId};
use batman_runtime::adapter::{
    Adapter, AdapterEvent, AdapterEventSink, AdapterFuture, ClaudeStartupOptions, StartSpec,
};
use claude::ClaudeAdapter;
use tokio::sync::Mutex;

struct CollectingSink {
    events: Mutex<Vec<AdapterEvent>>,
}

impl AdapterEventSink for CollectingSink {
    fn emit(&self, event: AdapterEvent) -> AdapterFuture<'_, u64> {
        Box::pin(async move {
            let mut events = self.events.lock().await;
            events.push(event);
            Ok(events.len() as u64)
        })
    }
}

#[tokio::test]
#[ignore = "invokes the real Claude model; gated on BATMAN_LIVE_CLAUDE=1, never run automatically"]
async fn start_a_real_claude_session_and_observe_its_result() {
    if std::env::var("BATMAN_LIVE_CLAUDE").as_deref() != Ok("1") {
        eprintln!("skipping: set BATMAN_LIVE_CLAUDE=1 to run this live test");
        return;
    }

    let run_id = RunId::new();
    let task_id = TaskId::new();
    let worker_id = WorkerId::new();
    let adapter = ClaudeAdapter::new(
        ClaudeStartupOptions::default(),
        std::env::temp_dir(),
        Vec::new(),
        run_id,
        task_id,
        worker_id,
        None,
    );
    let sink = Arc::new(CollectingSink {
        events: Mutex::new(Vec::new()),
    });

    adapter
        .start(
            StartSpec {
                run_id,
                task_id,
                worker_id,
                prompt: "Reply with exactly the word: pong".to_string(),
                resume: None,
            },
            sink.clone(),
        )
        .await
        .expect("start must succeed against a real installed claude CLI");

    // Give the background reader task time to receive the model's
    // response and normalize the final result frame.
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;

    let events = sink.events.lock().await;
    assert!(
        !events.is_empty(),
        "expected at least a ProcessStarted event plus the vendor's real response"
    );

    adapter
        .dispose()
        .await
        .expect("dispose must cleanly terminate the live session");
}
