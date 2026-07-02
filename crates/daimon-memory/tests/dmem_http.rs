//! Unit tests for the memory-tier seam + sidecar client.
//!
//! - DTO JSON encode/decode against a recorded dm-lite `/recall` response shape.
//! - `pre_turn_recall` is Err-free: pointed at a black-hole address (connect
//!   refused) it returns `degraded()` with no panic and no `Err`.

use daimon_memory::{
    DmemHttpMemory, MemoryService, NullMemory, PreTurnContext, RecallBudget, RecordKind,
    TypedBody, TypedRecord,
};

/// A recorded dm-lite `/recall` response: a JSON array of `Entry` objects. The
/// client must decode this (reading uri/kind/namespace/title/body, ignoring the
/// bitemporal + tags + importance columns).
const RECORDED_RECALL: &str = r#"[
  {
    "uri": "daimon://resources/incidents/incident_summary/mail-relay-down",
    "kind": "incident_summary",
    "namespace": "resources/incidents",
    "title": "Mail relay down",
    "body": "postfix on the local raid stopped accepting connections",
    "tags": ["mail", "postfix"],
    "importance": 60,
    "dedup_key": "daimon://resources/incidents/incident_summary/mail-relay-down",
    "created_ms": 1750000000000,
    "valid_from_ms": 1750000000000,
    "valid_to_ms": null,
    "system_from_ms": 1750000000000,
    "system_to_ms": null
  },
  {
    "uri": "daimon://resources/notes/memory/vector-substrate",
    "kind": "memory",
    "namespace": "resources/notes",
    "title": "Vector substrate",
    "body": "the vector substrate is zvec",
    "tags": [],
    "importance": 50,
    "dedup_key": "daimon://resources/notes/memory/vector-substrate",
    "created_ms": 1750000000000,
    "valid_from_ms": 1750000000000,
    "valid_to_ms": null,
    "system_from_ms": 1750000000000,
    "system_to_ms": null
  }
]"#;

/// The subset of `Entry` the client reads — mirror of the private `DmEntry`.
#[derive(serde::Deserialize)]
struct EntryProbe {
    uri: String,
    kind: String,
    namespace: String,
    title: String,
    body: String,
}

#[test]
fn recorded_recall_response_decodes() {
    let entries: Vec<EntryProbe> =
        serde_json::from_str(RECORDED_RECALL).expect("recorded /recall must decode");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].kind, "incident_summary");
    assert_eq!(entries[0].namespace, "resources/incidents");
    assert!(entries[0].body.contains("postfix"));
    assert!(entries[0].uri.starts_with("daimon://"));
    assert_eq!(entries[1].title, "Vector substrate");
}

#[test]
fn typed_record_json_round_trips() {
    // A TypedRecord (flattened body + optional namespace) must encode/decode
    // stably — this is the shape captured over the wire and rendered on the
    // hydrate side.
    let rec = TypedRecord {
        body: TypedBody::Decision {
            title: "Lock sidecar memory".into(),
            context: "musl-static cannot link zvec".into(),
            decision: "talk to dmem serve over HTTP".into(),
            rationale: "keeps the binary static".into(),
        },
        namespace: Some("resources/daimon".into()),
    };
    let j = serde_json::to_string(&rec).expect("encode");
    // The `kind` tag drives the discriminant.
    assert!(j.contains("\"kind\":\"decision\""));
    let back: TypedRecord = serde_json::from_str(&j).expect("decode");
    assert_eq!(back.body.kind(), RecordKind::Decision);
    assert_eq!(back.namespace.as_deref(), Some("resources/daimon"));
}

#[test]
fn typed_body_incident_and_lesson_encode() {
    let inc = TypedBody::Incident {
        title: "t".into(),
        impact: "i".into(),
        resolution: "r".into(),
    };
    assert_eq!(inc.kind(), RecordKind::Incident);
    let les = TypedBody::Lesson {
        title: "t".into(),
        lesson: "l".into(),
    };
    assert_eq!(les.kind(), RecordKind::Lesson);
    // Both round-trip.
    for b in [inc, les] {
        let j = serde_json::to_string(&b).unwrap();
        let back: TypedBody = serde_json::from_str(&j).unwrap();
        assert_eq!(back.kind(), b.kind());
    }
}

#[test]
fn pre_turn_context_degraded_sentinel() {
    let d = PreTurnContext::degraded();
    assert!(d.degraded);
    assert!(!d.has_context());
    assert_eq!(d.hits, 0);
}

#[tokio::test]
async fn pre_turn_recall_never_errs_on_connect_refused() {
    // 127.0.0.1:9 is the discard port — connections are refused fast. The client
    // must return degraded() (no Err, no panic) so a dead sidecar never blocks
    // or fails a chat turn.
    let mem = DmemHttpMemory::new("http://127.0.0.1:9", "unused-token").expect("client builds");
    let ctx = mem
        .pre_turn_recall("does the mail relay use postfix?", RecallBudget::default())
        .await;
    assert!(ctx.degraded, "connect-refused must degrade");
    assert!(!ctx.has_context());
    assert_eq!(ctx.hits, 0);
}

#[tokio::test]
async fn null_memory_is_fail_soft() {
    let mem = NullMemory;
    // Reads: empty + degraded.
    let ctx = mem.pre_turn_recall("anything", RecallBudget::default()).await;
    assert!(ctx.degraded);
    let health = mem.health().await;
    assert!(!health.reachable);
    // Writes: Ok-swallowed with a sentinel uri.
    let uri = mem
        .capture(TypedRecord {
            body: TypedBody::Lesson {
                title: "t".into(),
                lesson: "l".into(),
            },
            namespace: None,
        })
        .await
        .expect("null capture is Ok");
    assert!(uri.contains("null-memory"));
}

#[test]
fn recall_budget_defaults() {
    let b = RecallBudget::default();
    assert_eq!(b.max_tokens, 3000);
    assert_eq!(b.top_k, 6);
    assert!(b.rerank);
}

#[test]
fn typed_record_decodes_from_tool_args_with_injected_kind() {
    // Mirrors chat.rs::parse_typed_record: the LLM tool args (title/decision,
    // no `kind`, no `namespace`) get a `kind` tag injected, then decode into a
    // TypedRecord via the flattened tagged enum + serde defaults. This is the
    // exact contract the chat memory-tool dispatch depends on.
    let args = serde_json::json!({
        "kind": "decision",
        "title": "Lock sidecar",
        "decision": "use dmem serve"
    });
    let rec: TypedRecord = serde_json::from_value(args).expect("decode with injected kind");
    assert_eq!(rec.body.kind(), RecordKind::Decision);
    assert!(rec.namespace.is_none(), "namespace defaults to None");
    match &rec.body {
        TypedBody::Decision {
            title,
            context,
            decision,
            rationale,
        } => {
            assert_eq!(title, "Lock sidecar");
            assert_eq!(context, ""); // serde default
            assert_eq!(decision, "use dmem serve");
            assert_eq!(rationale, ""); // serde default
        }
        _ => panic!("wrong variant"),
    }
}
