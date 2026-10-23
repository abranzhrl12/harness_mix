use std::{hint::black_box, path::Path, time::{Duration, Instant}};

fn old(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}
fn new(bytes: &[u8]) -> [u8; 32] {
    use sha2_new::{Digest, Sha256};
    Sha256::digest(bytes).into()
}
fn lookup_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = Vec::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize]);
        output.push(HEX[(byte & 15) as usize]);
    }
    String::from_utf8(output).unwrap()
}
fn run(bytes: &[u8], variant: usize) {
    match variant {
        0 => { black_box(old(bytes)); }
        1 => { black_box(new(bytes)); }
        2 => { use sha2::{Digest, Sha256}; black_box(format!("{:x}", Sha256::digest(bytes))); }
        3 => {
            // 0.11 output has no LowerHex; adapt to the identical 0.10 formatter.
            let output: sha2::digest::Output<sha2::Sha256> = new(bytes).into();
            black_box(format!("{:x}", output));
        }
        4 => { black_box(lookup_hex(&old(bytes))); }
        5 => { black_box(lookup_hex(&new(bytes))); }
        _ => unreachable!(),
    }
}
fn collect(path: &Path, data: &mut Vec<Vec<u8>>) {
    let mut paths = std::fs::read_dir(path).unwrap().map(|e| e.unwrap().path()).collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if path.is_symlink() { continue; }
        if path.is_dir() { collect(&path, data); }
        else if path.extension().is_some_and(|e| e == "rs") {
            let bytes = std::fs::read(path).unwrap();
            if bytes.len() <= 1048576 { data.push(bytes); }
        }
    }
}
fn measure(data: &[Vec<u8>], variant: usize) -> f64 {
    let start = Instant::now();
    let mut rounds = 0;
    loop {
        for bytes in data { run(black_box(bytes), variant); }
        rounds += 1;
        if start.elapsed() >= Duration::from_millis(75) { break; }
    }
    start.elapsed().as_secs_f64() / rounds as f64
}

#[test]
fn compare_sha_versions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut corpus = Vec::new();
    collect(&root.join("src"), &mut corpus);
    collect(&root.join("crates"), &mut corpus);
    assert!(!corpus.is_empty());
    let mut cases = vec![("repository-rust".to_string(), corpus)];
    for size in [64, 1024, 16384, 65536, 1048576] {
        cases.push((format!("synthetic-{size}"), vec![(0..size).map(|i| (i % 251) as u8).collect()]));
    }
    // Compare every byte value, not just common ASCII inputs, and known padding boundaries.
    for size in 0..=256 {
        let bytes: Vec<u8> = (0..size).map(|i| i as u8).collect();
        assert_eq!(old(&bytes), new(&bytes));
        use sha2::{Digest, Sha256};
        assert_eq!(lookup_hex(&old(&bytes)), format!("{:x}", Sha256::digest(&bytes)));
    }
    assert_eq!(lookup_hex(&new(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let names = ["sha2-0.10.9-raw", "sha2-0.11.1-raw", "sha2-0.10.9-format", "sha2-0.11.1-format", "sha2-0.10.9-lookup", "sha2-0.11.1-lookup"];
    let mut results = Vec::new();
    for (name, data) in cases {
        for bytes in &data {
            assert_eq!(old(bytes), new(bytes));
            use sha2::{Digest, Sha256};
            assert_eq!(lookup_hex(&old(bytes)), format!("{:x}", Sha256::digest(bytes)));
        }
        let mut samples = vec![Vec::<f64>::new(); names.len()];
        for variant in 0..names.len() { measure(&data, variant); }
        for trial in 0..7 {
            // Rotate and reverse to spread temporal bias across all variants.
            for step in 0..names.len() {
                let offset = if trial % 2 == 0 { step } else { names.len() - 1 - step };
                let variant = (offset + trial) % names.len();
                samples[variant].push(measure(&data, variant));
            }
        }
        let bytes: usize = data.iter().map(Vec::len).sum();
        for (variant, trials) in samples.iter().enumerate() {
            let mut sorted = trials.clone(); sorted.sort_by(f64::total_cmp);
            results.push(serde_json::json!({"case":name,"variant":names[variant],"files":data.len(),"bytes":bytes,
                "median_ms":sorted[3]*1000.0,"trials_seconds":trials}));
        }
    }
    #[cfg(target_arch = "x86_64")]
    let cpu_features = serde_json::json!({"sha":std::is_x86_feature_detected!("sha"),"sse2":std::is_x86_feature_detected!("sse2"),"ssse3":std::is_x86_feature_detected!("ssse3"),"sse4.1":std::is_x86_feature_detected!("sse4.1"),"avx2":std::is_x86_feature_detected!("avx2")});
    #[cfg(not(target_arch = "x86_64"))]
    let cpu_features = serde_json::Value::Null;
    let backend_mode = if cfg!(feature = "force-soft") { "force-soft" } else { "runtime-dispatch" };
    let report = serde_json::json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"cpu_features":cpu_features,
        "backend_mode":backend_mode,
        "method":"opt-level=3; sha2 0.10.9; single thread; preloaded data; warmup; 7 rotated/reversed trials >=75ms; median; no IO in timing",
        "results":results});
    let result_file = if cfg!(feature = "force-soft") { "sha-results-force-soft.json" } else { "sha-results-runtime-dispatch.json" };
    std::fs::write(Path::new(env!("CARGO_MANIFEST_DIR")).join(result_file), serde_json::to_string_pretty(&report).unwrap()).unwrap();
}
