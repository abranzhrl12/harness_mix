#[cfg(test)]
mod sha_versions;

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
    use std::{hint::black_box, time::{Duration, Instant}, path::Path};

    fn digest(data: &[u8], blake: bool) -> [u8; 32] {
        if blake { *blake3::hash(data).as_bytes() } else { Sha256::digest(data).into() }
    }

    fn measure(data: &[Vec<u8>], blake: bool, hex: bool) -> f64 {
        let start = Instant::now();
        let mut rounds = 0u64;
        loop {
            for bytes in data {
                if hex {
                    if blake { black_box(blake3::hash(black_box(bytes)).to_hex().to_string()); }
                    else { black_box(format!("{:x}", Sha256::digest(black_box(bytes)))); }
                } else { black_box(digest(black_box(bytes), blake)); }
            }
            rounds += 1;
            if start.elapsed() >= Duration::from_millis(150) { break; }
        }
        start.elapsed().as_secs_f64() / rounds as f64
    }

    fn collect(path: &Path, output: &mut Vec<Vec<u8>>) {
        let mut entries = std::fs::read_dir(path).unwrap().map(|e| e.unwrap().path()).collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_symlink() { continue; }
            if path.is_dir() { collect(&path, output); }
            else if path.extension().is_some_and(|e| e == "rs") {
                let bytes = std::fs::read(path).unwrap();
                if bytes.len() <= 1024 * 1024 { output.push(bytes); }
            }
        }
    }

    #[test]
    #[ignore = "historical BLAKE3 benchmark; run explicitly with --ignored"]
    fn compare_hashes() {
        assert_eq!(format!("{:x}", Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(blake3::hash(b"abc").to_hex().as_str(), "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85");
        for bytes in [b"".as_slice(), b"abc", b"fn main() {}\r\n", "á😀\n".as_bytes()] {
            assert_ne!(digest(bytes, false), digest(bytes, true));
            assert_eq!(digest(bytes, true), digest(bytes, true));
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut corpus = Vec::new();
        collect(&root.join("src"), &mut corpus);
        collect(&root.join("crates"), &mut corpus);
        assert!(!corpus.is_empty());
        let mut cases = vec![("repository-rust".to_string(), corpus)];
        for size in [64, 1024, 16384, 65536, 1048576] {
            cases.push((format!("synthetic-{size}"), vec![(0..size).map(|i| (i % 251) as u8).collect()]));
        }
        let mut results = Vec::new();
        for (name, data) in cases {
            for hex in [false, true] {
                let mut sha = Vec::new(); let mut blake = Vec::new();
                measure(&data, false, hex); measure(&data, true, hex);
                for trial in 0..7 {
                    if trial % 2 == 0 {
                        sha.push(measure(&data, false, hex)); blake.push(measure(&data, true, hex));
                    } else {
                        blake.push(measure(&data, true, hex)); sha.push(measure(&data, false, hex));
                    }
                }
                sha.sort_by(f64::total_cmp); blake.sort_by(f64::total_cmp);
                let bytes: usize = data.iter().map(Vec::len).sum();
                results.push(serde_json::json!({"case":name,"hex":hex,"files":data.len(),"bytes":bytes,
                    "sha256_ms":sha[3]*1000.0,"blake3_ms":blake[3]*1000.0,"speedup":sha[3]/blake[3],
                    "sha256_trials_seconds":sha,"blake3_trials_seconds":blake}));
            }
        }
        let report = serde_json::json!({"architecture":std::env::consts::ARCH,"os":std::env::consts::OS,
            "method":"opt-level=3; single thread; preloaded bytes; warmup; 7 alternating trials >=150ms; median; no IO in timing",
            "results":results});
        std::fs::write(Path::new(env!("CARGO_MANIFEST_DIR")).join("results.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    }
}
