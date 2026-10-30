//! Contract tests for every registered command (ADR-0006, validation section):
//! stale revision, dry run, duplicate request id, atomic batches and undo/redo.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeSet;

use fosscut_core::Session;
use fosscut_core::{CommandCall, Registry, Request, codes};
use fosscut_model::MemoryStore;
use serde_json::{Value, json};
use support::*;

/// A fresh session prepared for one command, and a valid call of that command.
fn sample(command: &str) -> (Session<MemoryStore>, CommandCall) {
    let (mut session, asset, clip) = session_with_clip();
    let args = match command {
        "media.add" => {
            let mut args = test_pattern_args();
            args["identity"]["size"] = json!(42);
            args["name"] = json!("second.mp4");
            args
        }
        "track.add" => json!({ "kind": "audio" }),
        "clip.add" => json!({ "asset": asset, "source_in": { "frames": 30 }, "duration": "2s" }),
        "clip.split" => json!({ "clip": clip, "at": "00:00:03:28" }),
        "clip.move" => json!({ "clip": clip, "start": { "seconds": "1" } }),
        "clip.trim" => json!({ "clip": clip, "start": "1s", "end": "5s" }),
        "clip.remove" => {
            let title = run(&mut session, "text.add", json!({ "text": "Remove me" }));
            json!({ "clip": title.results[0]["clip"] })
        }
        "text.add" => json!({ "text": "Hello", "start": "0f", "duration": "1.5s" }),
        "captions.add" => json!({
            "words": [
                { "text": "Hello", "start": { "seconds": "0.5" }, "end": { "seconds": "0.9" } },
                { "text": "world.", "start": { "seconds": "0.9" }, "end": { "seconds": "1.4" } }
            ],
            "preset": "highlight"
        }),
        "captions.edit" => {
            let added = run(&mut session, "captions.add", captions_sample());
            json!({ "clip": added.results[0]["pages"][0]["clip"], "text": "Hi there", "emphasis": [1] })
        }
        "clip.set" => {
            json!({ "clip": clip, "set": { "transform.scale": 1.5, "opacity": 0.8, "blend": "screen", "audio.fade_in": { "seconds": "0.5" } } })
        }
        "keyframe.set" => {
            json!({ "clip": clip, "property": "opacity", "at": "1s", "value": 0.25, "easing": "ease_in_out" })
        }
        "effect.add" => {
            json!({ "clip": clip, "type": "color.adjust", "params": { "exposure": 0.5 } })
        }
        "effect.set" | "effect.remove" | "effect.move" => {
            let added = run(
                &mut session,
                "effect.add",
                json!({ "clip": clip, "type": "color.vignette" }),
            );
            run(
                &mut session,
                "effect.add",
                json!({ "clip": clip, "type": "stylize.grain" }),
            );
            let effect = added.results[0]["effect"].clone();
            match command {
                "effect.set" => {
                    json!({ "clip": clip, "effect": effect, "params": { "amount": 0.4 } })
                }
                "effect.move" => json!({ "clip": clip, "effect": effect, "index": 1 }),
                _ => json!({ "clip": clip, "effect": effect }),
            }
        }
        "effect.copy" => {
            run(
                &mut session,
                "effect.add",
                json!({ "clip": clip, "type": "color.adjust" }),
            );
            let title = run(&mut session, "text.add", json!({ "text": "Graded too" }));
            json!({ "from": clip, "to": [title.results[0]["clip"]] })
        }
        "adjustment.add" => {
            json!({ "start": "1s", "duration": "2s", "effects": [{ "type": "color.adjust", "params": { "saturation": -1 } }] })
        }
        "look.apply" => json!({ "clip": clip, "look": "teal_orange" }),
        "transition.set" | "transition.remove" => {
            run(
                &mut session,
                "clip.split",
                json!({ "clip": clip, "at": "4s" }),
            );
            if command == "transition.remove" {
                run(
                    &mut session,
                    "transition.set",
                    json!({ "clip": clip, "type": "dissolve" }),
                );
                json!({ "clip": clip })
            } else {
                json!({ "clip": clip, "type": "wipe", "duration": "1s", "params": { "direction": "up" } })
            }
        }
        "clip.speed" => json!({ "clip": clip, "rate": 2, "pitch": "shift" }),
        "clip.freeze" => json!({ "clip": clip, "at": "3s", "duration": "1s" }),
        "animation.set" => {
            json!({ "clip": clip, "in": { "preset": "fade" }, "combo": { "preset": "pulse", "duration": "2s" } })
        }
        "mask.add" => {
            json!({ "clip": clip, "shape": "ellipse", "feather": 0.3, "center": [0.4, 0.5] })
        }
        "mask.set" | "mask.remove" => {
            let added = run(
                &mut session,
                "mask.add",
                json!({ "clip": clip, "shape": "rectangle" }),
            );
            let mask = added.results[0]["mask"].clone();
            if command == "mask.set" {
                json!({ "clip": clip, "mask": mask, "roundness": 0.5, "invert": true })
            } else {
                json!({ "clip": clip, "mask": mask })
            }
        }
        "keyframe.remove" => {
            run(
                &mut session,
                "keyframe.set",
                json!({ "clip": clip, "property": "transform.scale", "offset": "2s", "value": 1.2 }),
            );
            json!({ "clip": clip, "property": "transform.scale", "offset": "2s" })
        }
        "timeline.duplicate" => json!({ "width": 1080, "height": 1920, "fit": "fill" }),
        "timeline.remove_ranges" => {
            // Cut from the text track only, so the primary video guard does not apply.
            let title = run(
                &mut session,
                "text.add",
                json!({ "text": "Long title", "duration": "4s" }),
            );
            let doc = session
                .query("timeline.read", json!({ "detail": "outline" }))
                .unwrap();
            let track = doc["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["kind"] == "text")
                .unwrap()["id"]
                .clone();
            let _ = title;
            json!({ "ranges": [{ "start": "1s", "end": "2s" }], "tracks": [track] })
        }
        "captions.style" => {
            run(&mut session, "captions.add", captions_sample());
            json!({ "preset": "boxed", "style": { "size": 0.07 } })
        }
        "history.undo" => json!({}),
        "history.redo" => {
            run(&mut session, "history.undo", json!({}));
            json!({})
        }
        other => panic!("no contract sample for {other}; add one before registering the command"),
    };
    (
        session,
        CommandCall {
            command: command.to_string(),
            args,
        },
    )
}

fn captions_sample() -> Value {
    json!({ "words": [{ "text": "Caption", "start": { "seconds": "2" }, "end": { "seconds": "2.5" } }] })
}

fn command_names() -> Vec<String> {
    Registry::standard()
        .commands()
        .map(|c| c.name.to_string())
        .collect()
}

fn call_request(call: &CommandCall) -> Request {
    let mut request = request(&call.command, call.args.clone());
    request.request_id = Some(format!("contract-{}", call.command));
    request
}

#[test]
fn every_command_has_a_contract_sample() {
    let names: BTreeSet<String> = command_names().into_iter().collect();
    for name in &names {
        let _ = sample(name);
    }
    assert!(
        names.contains("clip.split") && names.contains("history.undo"),
        "{names:?}"
    );
}

#[test]
fn stale_revisions_are_rejected_without_changes() {
    for name in command_names() {
        let (mut session, call) = sample(&name);
        let before = documents(&session);
        let mut request = call_request(&call);
        request.expected_revision = Some(session.revision() + 7);
        let error = session.execute(request).unwrap_err();
        assert_eq!(error.code, codes::REVISION_CONFLICT, "{name}");
        assert_eq!(
            error.details["current_revision"],
            json!(session.revision()),
            "{name}"
        );
        assert_eq!(documents(&session), before, "{name} changed documents");
    }
}

#[test]
fn dry_runs_report_changes_without_committing() {
    for name in command_names() {
        let (mut session, call) = sample(&name);
        let before = documents(&session);
        let revision = session.revision();
        let log = session.log().len();
        let mut request = call_request(&call);
        request.dry_run = true;
        let response = session
            .execute(request)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(!response.committed && response.dry_run, "{name}");
        assert!(
            !response.changes.is_empty(),
            "{name} dry run predicted no changes"
        );
        assert_eq!(response.revision, revision, "{name}");
        assert_eq!(session.revision(), revision, "{name}");
        assert_eq!(session.log().len(), log, "{name}");
        assert_eq!(documents(&session), before, "{name} changed documents");
    }
}

#[test]
fn repeated_request_ids_replay_the_first_result() {
    for name in command_names() {
        let (mut session, call) = sample(&name);
        let first = session
            .execute(call_request(&call))
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(first.committed, "{name}");
        let after = documents(&session);
        let second = session.execute(call_request(&call)).unwrap();
        assert!(second.replayed, "{name}");
        assert_eq!(second.results, first.results, "{name}");
        assert_eq!(second.revision, first.revision, "{name}");
        assert_eq!(documents(&session), after, "{name} executed twice");

        let mut other = call_request(&call);
        other.commands[0].args = json!({ "unexpected": true });
        let error = session.execute(other).unwrap_err();
        assert_eq!(error.code, codes::REQUEST_REUSED, "{name}");
    }
}

#[test]
fn undo_restores_and_redo_reapplies_every_command() {
    for name in command_names()
        .into_iter()
        .filter(|n| !n.starts_with("history."))
    {
        let (mut session, call) = sample(&name);
        let before = session.project().clone();
        session.execute(call_request(&call)).unwrap();
        let after = session.project().clone();
        run(&mut session, "history.undo", json!({}));
        assert_eq!(session.project().timelines, before.timelines, "{name} undo");
        assert_eq!(session.project().media, before.media, "{name} undo");
        run(&mut session, "history.redo", json!({}));
        assert_eq!(session.project().timelines, after.timelines, "{name} redo");
        assert_eq!(session.project().media, after.media, "{name} redo");
    }
}

#[test]
fn batches_are_atomic() {
    for name in command_names()
        .into_iter()
        .filter(|n| !n.starts_with("history."))
    {
        let (mut session, call) = sample(&name);
        let before = documents(&session);
        let mut request = call_request(&call);
        request.commands.push(CommandCall {
            command: "clip.split".into(),
            args: json!({ "clip": "clip_zzzz", "at": "1f" }),
        });
        let error = session.execute(request).unwrap_err();
        assert_eq!(error.code, codes::CLIP_NOT_FOUND, "{name}");
        assert_eq!(error.details["command_index"], json!(1), "{name}");
        assert_eq!(documents(&session), before, "{name} partially applied");
    }
}

#[test]
fn removing_primary_video_needs_permission() {
    let (mut session, _, clip) = session_with_clip();
    let error = session
        .execute(request("clip.remove", json!({ "clip": clip })))
        .unwrap_err();
    assert_eq!(error.code, codes::GUARD_DESTRUCTIVE);
    assert!(error.hint.unwrap().contains("allow_destructive"));
    let mut allowed = request("clip.remove", json!({ "clip": clip }));
    allowed.allow_destructive = true;
    assert!(session.execute(allowed).unwrap().committed);
}

#[test]
fn unknown_commands_and_bad_arguments_are_explained() {
    let mut session = new_session();
    let error = session
        .execute(request("clip.splitt", json!({})))
        .unwrap_err();
    assert_eq!(error.code, codes::UNKNOWN_COMMAND);
    assert!(error.hint.unwrap().contains("clip.split"));
    let error = session
        .execute(request("text.add", json!({ "text": "x", "colour": "red" })))
        .unwrap_err();
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
    let error = session
        .execute(request("text.add", json!({ "text": "x", "start": 12 })))
        .unwrap_err();
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
    assert!(error.message.contains("explicit unit"), "{}", error.message);
}

#[test]
fn every_command_documents_itself() {
    let session = new_session();
    let capabilities = session.query("capabilities", Value::Null).unwrap();
    let commands = capabilities["commands"].as_array().unwrap();
    assert_eq!(commands.len(), command_names().len());
    for command in commands {
        assert!(!command["summary"].as_str().unwrap().is_empty());
        assert!(command["input_schema"].is_object(), "{}", command["name"]);
    }
    assert!(
        capabilities["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["code"] == codes::REVISION_CONFLICT)
    );
}

#[test]
fn capabilities_by_name_carry_only_those_schemas() {
    let session = new_session();
    let everything = session
        .query("capabilities", json!({ "detail": "full" }))
        .unwrap();
    let named = session
        .query(
            "capabilities",
            json!({ "detail": "outline", "names": ["clip.split", "timeline.read"] }),
        )
        .unwrap();
    let names = |list: &Value| -> Vec<String> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names(&named["commands"]), ["clip.split"]);
    assert_eq!(names(&named["queries"]), ["timeline.read"]);
    // Schemas even at outline detail, identical to the full listing, and nothing else.
    let split = everything["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "clip.split")
        .unwrap();
    assert_eq!(&named["commands"][0], split);
    assert!(named["queries"][0]["input_schema"].is_object());
    assert!(named.get("errors").is_none() && named.get("request_schema").is_none());
    assert!(named.to_string().len() * 5 < everything.to_string().len());

    let error = session
        .query(
            "capabilities",
            json!({ "names": ["clip.split", "clip.explode", "secrets"] }),
        )
        .unwrap_err();
    assert_eq!(error.code, codes::INVALID_ARGUMENTS);
    assert_eq!(error.details["unknown"], json!(["clip.explode", "secrets"]));
    assert!(error.hint.unwrap().contains("outline"));
}
