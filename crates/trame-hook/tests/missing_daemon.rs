// An integration test is an ordinary binary: `clippy.toml`'s exemptions do not apply here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! ★★ **The negative control, written before the happy path.**
//!
//! This is the project's first IPC, and therefore the prime spot for a plausible output that
//! lies. The failure mode being guarded against is precise:
//!
//! > The daemon is not listening. `trame-hook` cannot ask. If it exits 0 saying nothing, the
//! > CLI reads "no objection" and the write goes through. The invariant is dead, and the agent
//! > works normally: **no symptom at all.**
//!
//! These tests exist **before** the happy path, not after, because a happy path that works makes
//! the degraded case abstract — and an abstract degraded case never really gets tested.
//!
//! They cover the four ways of getting no verdict: missing socket, stale socket, mute daemon,
//! unintelligible response. **All four must deny.**

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use trame_hook::{Decision, HookError, ask};

const PAYLOAD: &str = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"echo x > f.txt"}}"#;

/// A faithful fake daemon: it **reads the request before answering**.
///
/// That detail is not decoration. A fake daemon that writes then closes without reading
/// sometimes wins the race against our own write, which then takes a `BrokenPipe` — and the
/// test fails one time in ten for a reason unrelated to what it checks. A real daemon reads then
/// answers; the fake must do the same.
fn reply_after_read(listener: &UnixListener, response: &str) {
    if let Ok((stream, _)) = listener.accept() {
        let mut request = String::new();
        let _ = BufReader::new(&stream).read_line(&mut request);
        let mut writer = &stream;
        let _ = writer.write_all(response.as_bytes());
        let _ = writer.flush();
    }
}

fn throwaway_path(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    std::env::temp_dir().join(format!("trame-hook-{name}-{unique}.sock"))
}

/// ★ Missing socket: the daemon never started for this project.
///
/// The most ordinary case — the user has not opened the project in Trame — and the most
/// dangerous one to let through.
#[test]
fn a_missing_socket_denies_and_says_why() {
    let socket = throwaway_path("missing");
    assert!(!socket.exists());

    let error = ask(&socket, PAYLOAD).expect_err("an absent daemon must NOT let the call through");

    assert!(
        matches!(error, HookError::SocketMissing { .. }),
        "the cause must be named, not generic: {error:?}"
    );
    let reason = error.reason();
    assert!(
        reason.contains("refused"),
        "the reason must say it is refused: {reason}"
    );
    assert!(
        reason.contains(&socket.display().to_string()),
        "and where to look: {reason}"
    );
}

/// ★ Stale socket: the file exists, nobody at the other end.
///
/// This is the case of a daemon that crashed leaving its file behind. It differs from the
/// previous one because it is not fixed the same way, and the reason must say so.
#[test]
fn a_stale_socket_denies_and_says_why() {
    let socket = throwaway_path("stale");
    // An ordinary file with a socket's name: exactly what a dead process leaves behind.
    std::fs::write(&socket, b"").expect("witness file");

    let error = ask(&socket, PAYLOAD).expect_err("a stale socket must NOT let the call through");

    assert!(
        matches!(error, HookError::Unreachable { .. }),
        "a stale socket is not a missing socket: {error:?}"
    );
    assert!(error.reason().contains("refused"));
    std::fs::remove_file(&socket).ok();
}

/// ★ Mute daemon: it accepts the connection then closes without answering.
///
/// The trickiest of the four, because everything looks like it works — the connection succeeds
/// and there simply is no verdict. An empty response **is not** silence.
///
/// # This test does not pin the error variant, and that is deliberate
///
/// Depending on who wins the race, our write leaves before the close (`UnreadableResponse`, the
/// daemon closed without answering) or after (`Unreachable` on a `BrokenPipe`). An early version
/// pinned `UnreadableResponse` and passed **three times out of five** — a flaky test, and
/// therefore a test people end up ignoring.
///
/// The property that matters is not which of the two: it is that neither lets the call through.
#[test]
fn a_daemon_that_answers_nothing_denies() {
    let socket = throwaway_path("mute");
    let listener = UnixListener::bind(&socket).expect("listening socket");
    let thread = std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            drop(stream); // accepts, then closes without a word
        }
    });

    let error = ask(&socket, PAYLOAD).expect_err("a mute daemon must NOT let the call through");
    assert!(
        matches!(
            error,
            HookError::UnreadableResponse(_) | HookError::Unreachable { .. }
        ),
        "with no verdict we refuse — whichever way the connection dies: {error:?}"
    );
    assert!(
        error.reason().contains("refused"),
        "and the reason says so: {}",
        error.reason()
    );

    thread.join().ok();
    std::fs::remove_file(&socket).ok();
}

/// ★ Unintelligible response: the daemon answers something we cannot read.
///
/// The case of a protocol break between two versions. We do not guess, we refuse.
#[test]
fn an_unreadable_response_denies() {
    for response in [
        "not json at all\n",
        "{\"decision\":\"maybe\"}\n",
        "{\"something\":\"else\"}\n",
    ] {
        let socket = throwaway_path("unreadable");
        let listener = UnixListener::bind(&socket).expect("listening socket");
        let to_send = response.to_owned();
        let thread = std::thread::spawn(move || reply_after_read(&listener, &to_send));

        let error = ask(&socket, PAYLOAD)
            .expect_err("an unreadable response must NOT let the call through");
        assert!(
            matches!(error, HookError::UnreadableResponse(_)),
            "response {response:?}: {error:?}"
        );

        thread.join().ok();
        std::fs::remove_file(&socket).ok();
    }
}

/// A payload the CLI should not have sent is refused too.
///
/// If the CLI's contract changes, we do not guess what it meant.
#[test]
fn an_unreadable_payload_denies() {
    let socket = throwaway_path("payload");
    let listener = UnixListener::bind(&socket).expect("listening socket");
    let thread = std::thread::spawn(move || {
        // An obliging daemon: it would say "silence" to anything. The hook must not even put
        // the question to it on an unreadable payload.
        reply_after_read(&listener, "{\"decision\":\"silence\"}\n");
    });

    let error = ask(&socket, "this is not json")
        .expect_err("an unreadable payload must NOT let the call through");
    assert!(
        matches!(error, HookError::UnreadablePayload(_)),
        "{error:?}"
    );

    drop(thread);
    std::fs::remove_file(&socket).ok();
}

/// ★★ **The control on the control**: can the apparatus say yes?
///
/// Without this test, all the previous ones would pass with an `ask` that refused *everything*,
/// including a healthy daemon. A negative control with no positive counterpart proves nothing —
/// that is the lesson written into the `concurrency-testing` skill.
///
/// # ★ The rule this test carries, beyond its own subject
///
/// > **A test that manufactures both sides of a protocol does not test the protocol.**
///
/// It tests that the two halves of one codebase agree with each other, which they will by
/// construction — including when they agree on the wrong thing, and including when they both
/// move at once.
///
/// So the daemon's answer here is written **by hand**, as a literal, and does not come from
/// `trame_daemon::hooks::Response::to_line`. That is what caught the `refus` -> `deny` wire
/// rename: the literal did not move when `hooks.rs` moved, so the disagreement surfaced. A
/// round-trip between the two real sides would have stayed green while the protocol changed
/// underneath it.
///
/// It is the twin problem from ADR 0018 in a new place — there, a harness measured a twin of
/// the production notice; here, a test would have measured the encoder against itself. The
/// general form: **when a test's expected value is computed by the code under test, the test
/// has no independent grip on the property.** Pin the literal, on at least one side.
#[test]
fn the_apparatus_can_say_both_yes_and_no() {
    for (response, expected) in [
        ("{\"decision\":\"silence\"}\n", Decision::Silence),
        (
            "{\"decision\":\"deny\",\"reason\":\"write through your file tool\"}\n",
            Decision::Deny("write through your file tool".to_owned()),
        ),
    ] {
        let socket = throwaway_path("nominal");
        let listener = UnixListener::bind(&socket).expect("listening socket");
        let to_send = response.to_owned();
        let thread = std::thread::spawn(move || reply_after_read(&listener, &to_send));

        let decision = ask(&socket, PAYLOAD).expect("a healthy daemon must answer");
        assert_eq!(decision, expected);

        thread.join().ok();
        std::fs::remove_file(&socket).ok();
    }
}

/// A refusal produces the JSON the CLI expects, and silence produces **nothing**.
///
/// The keys come from probe 2, where they were observed — not from a type.
#[test]
fn the_emitted_json_is_the_shape_the_cli_expects() {
    assert!(
        Decision::Silence.to_json().is_none(),
        "saying nothing is what lets the call through, and that is intended for silence"
    );
    let json = Decision::Deny("reason".to_owned())
        .to_json()
        .expect("a refusal produces JSON");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let output = &parsed["hookSpecificOutput"];
    assert_eq!(output["hookEventName"], "PreToolUse");
    assert_eq!(output["permissionDecision"], "deny");
    assert_eq!(output["permissionDecisionReason"], "reason");
}
