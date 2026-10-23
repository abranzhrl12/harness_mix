#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    use std::{hint::black_box, path::Path, time::{Duration, Instant}};

    fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

    fn compare(left: &[u8], right: &[u8], bytes: bool) -> bool {
        if bytes { left == right } else { hash(left) == hash(right) }
    }

    fn measure(left: &[u8], right: &[u8], bytes: bool) -> f64 {
        let started = Instant::now();
        let mut rounds = 0u64;
        loop {
            // Amortize the timer so the fast comparison is not dominated by it.
            for _ in 0..64 {
                black_box(compare(black_box(left), black_box(right), bytes));
            }
            rounds += 64;
            if started.elapsed() >= Duration::from_millis(100) { break; }
        }
        started.elapsed().as_secs_f64() / rounds as f64
    }

    #[test]
    fn benchmark_comparison_cost() {
        let mut results = Vec::new();
        for size in [8192, 131072, 1048576] {
            let original: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            for changed in [false, true] {
                let mut current = original.clone();
                if changed { current[size - 1] ^= 1; }
                assert_eq!(compare(&original, &current, true), !changed);
                assert_eq!(compare(&original, &current, false), !changed);
                measure(&original, &current, false);
                measure(&original, &current, true);
                let mut hashes = Vec::new(); let mut bytes = Vec::new();
                for trial in 0..7 {
                    for direct in if trial % 2 == 0 { [false, true] } else { [true, false] } {
                        let value = measure(&original, &current, direct);
                        if direct { bytes.push(value); } else { hashes.push(value); }
                    }
                }
                hashes.sort_by(f64::total_cmp); bytes.sort_by(f64::total_cmp);
                results.push(serde_json::json!({"bytes":size,"changedLastByte":changed,
                    "twoSha256Microseconds":hashes[3]*1e6,"directComparisonMicroseconds":bytes[3]*1e6,
                    "speedup":hashes[3]/bytes[3],"shaTrialsSeconds":hashes,"byteTrialsSeconds":bytes}));
            }
        }
        let report = serde_json::json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
            "method":"opt-level=3; SHA256 sha2 0.10.9 with hex formatter; preloaded distinct buffers; single thread; 7 alternating trials >=100ms each; batches of64; median; no filesystem/SQLite",
            "results":results});
        std::fs::write(Path::new(env!("CARGO_MANIFEST_DIR")).join("results.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
}
