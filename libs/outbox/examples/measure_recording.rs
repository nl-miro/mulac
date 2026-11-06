//! Measures one batch recording in a fresh process. Driven by
//! `scripts/measure_outbox_recording.py`, which prepares the database outside
//! this process; run it only with a disposable database.
//!
//! `prepare <size>` records the batch (to seed replay measurements) and exits.
//! `measure <size>` builds the same batch, then times one `record_batch` call
//! and prints the client's memory. Boundaries:
//!
//! * the timed region is the `record_batch` call alone: validation, conversion,
//!   statements, verification and commit. Input construction, process start and
//!   connection-pool creation are outside it;
//! * peak memory is the whole process (`VmHWM`), so it *includes* the input
//!   batch; `rss_before_kib` is taken after the input is built, so
//!   `peak - rss_before` is what recording itself added.
use mulac_outbox::prelude::*;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

/// Payload sizes cycle over 20 events: 14 small, 5 medium, 1 large.
fn payload_len(i: usize) -> usize {
    match i % 20 {
        0 => 16 * 1024,
        1..=5 => 2 * 1024,
        _ => 256,
    }
}

fn batch(size: usize) -> Vec<NewOutboxEnvelope> {
    (0..size)
        .map(|i| {
            let id = Uuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_0000 + i as u128);
            let pad = "x".repeat(payload_len(i));
            NewOutboxEnvelope {
                payload: format!(r#"{{"i":{i},"pad":"{pad}"}}"#),
                metadata: NewOutboxMetadata {
                    sequence: None,
                    event_id: id,
                    message_id: None,
                    correlation_id: None,
                    causation_id: None,
                    event_type: "Measured.Event".into(),
                    routing_key: "measured.event".into(),
                    source: Some("measure".into()),
                    content_type: Some("application/json".into()),
                },
            }
        })
        .collect()
}

fn status_kib(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with(field))
                .and_then(|line| line.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("mode: prepare | measure");
    let size: usize = args
        .next()
        .expect("size")
        .parse()
        .expect("size is a number");
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");

    let envelopes = batch(size);
    let pool = build_pool(&url).expect("pool");
    drop(pool.get().expect("warm connection"));
    let recorder = OutboxRecorder::new(Arc::new(DieselPgOutboxRepository::new(pool)));
    let payload_bytes: usize = envelopes.iter().map(|e| e.payload.len()).sum();
    let rss_before = status_kib("VmRSS:");

    let started = Instant::now();
    recorder.record_batch(&envelopes).expect("record");
    let elapsed = started.elapsed();

    if mode == "measure" {
        println!(
            "size={size} payload_bytes={payload_bytes} record_ms={:.1} rss_before_kib={rss_before} peak_kib={}",
            elapsed.as_secs_f64() * 1000.0,
            status_kib("VmHWM:"),
        );
    }
}
