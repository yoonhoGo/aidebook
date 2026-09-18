//! Reproducible FTS5 benchmark for the M5 acceptance target.

use aidebook_lib::core::{Core, SearchRequest, Snapshot, SourceRef};
use std::time::Instant;

const SEED: u64 = 20260919;
const SEED_COUNT: usize = 10_000;
const WARMUP_RUNS: usize = 5;
const MEASURED_RUNS: usize = 30;

#[test]
fn search_10k_p95_is_recorded() {
    let core = Core::in_memory().expect("benchmark core");
    for index in 0..SEED_COUNT {
        let source = SourceRef::new(
            "benchmark",
            format!("seed-{SEED}"),
            format!("note-{index:05}"),
            format!("https://benchmark.test/{index}"),
            "note",
        );
        let snapshot = Snapshot::new(
            source,
            format!("Benchmark note {index}"),
            format!(
                "seed-{SEED} benchmarktoken shard-{} durable local context",
                index % 32
            ),
            Some("2026-09-19T00:00:00Z".to_string()),
            "2026-09-19T00:00:01Z",
        );
        core.ingest_snapshot(snapshot).expect("seed snapshot");
    }
    let request = SearchRequest {
        query: "benchmarktoken".to_string(),
        provider: Some("benchmark".to_string()),
        kind: Some("note".to_string()),
        source_updated_after: None,
        source_updated_before: None,
        max_age_seconds: None,
        limit: Some(20),
    };
    for _ in 0..WARMUP_RUNS {
        let response = core.search(request.clone()).expect("warmup search");
        assert_eq!(response.results.len(), 20);
    }
    let mut durations = Vec::with_capacity(MEASURED_RUNS);
    for _ in 0..MEASURED_RUNS {
        let start = Instant::now();
        let response = core.search(request.clone()).expect("measured search");
        assert_eq!(response.results.len(), 20);
        durations.push(start.elapsed().as_secs_f64() * 1_000.0);
    }
    durations.sort_by(|left, right| left.partial_cmp(right).expect("finite duration"));
    let p95_index = ((MEASURED_RUNS as f64 * 0.95).ceil() as usize).saturating_sub(1);
    let p95_ms = durations[p95_index];
    let environment = format!(
        "os={} arch={} rust={} parallelism={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        rustc_version(),
        std::thread::available_parallelism()
            .map(|value| value.get().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    );
    println!(
        "M5_BENCHMARK seed={SEED} seed_count={SEED_COUNT} warmup_runs={WARMUP_RUNS} measured_runs={MEASURED_RUNS} p95_ms={p95_ms:.3} environment={environment}"
    );
    assert!(
        p95_ms <= 300.0,
        "10k search p95 exceeded 300ms: {p95_ms:.3}ms"
    );
}

fn rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
