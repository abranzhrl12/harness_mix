pub use harness_domain::{Failure, contracts, hash};

#[allow(dead_code)]
#[path = "../reference/mod.rs"]
mod reference;

use harness_domain::{
    contracts::{ReplaceArgs, ReplaceChunk},
    editing::prepare,
};
use std::{hint::black_box, time::Instant};

fn fixture(size: usize, count: usize, mode: &str) -> (Vec<u8>, ReplaceArgs) {
    let eol = if mode.contains("crlf") { "\r\n" } else { "\n" };
    let mut source = if mode.contains("bom") {
        "\u{feff}".to_owned()
    } else {
        String::new()
    };
    let mut rows = Vec::new();
    let long = if mode == "long" {
        "x".repeat(180)
    } else {
        String::new()
    };
    let mut id = 0;
    loop {
        let unicode = if mode.contains("crlf") {
            " café🙂"
        } else {
            ""
        };
        let row = format!("r{id:05} marker_{id:06}{unicode} {long}{eol}");
        if source.len() + row.len() > size {
            break;
        }
        let start = source.len();
        source.push_str(&row);
        rows.push((start, row));
        id += 1;
    }
    let chunks = (0..count)
        .map(|slot| {
            let row_id = slot * rows.len() / count;
            let row = &rows[row_id].1;
            let old = if mode == "long" {
                row.trim_end().to_owned()
            } else if mode == "fallback" || (mode == "mixed" && slot % 4 == 3) {
                format!("\t{}", row.trim_end())
            } else {
                format!("marker_{row_id:06}")
            };
            ReplaceChunk {
                old_text: old,
                new_text: format!("updated_{slot:03}\nnext_{slot:03}"),
            }
        })
        .collect();
    let mut args = ReplaceArgs {
        path: "a.rs".into(),
        hash: hash(source.as_bytes()),
        chunks: Some(chunks),
        ..ReplaceArgs::default()
    };
    if mode == "missing" {
        args.chunks.as_mut().unwrap()[0].old_text = "absent_pattern".into();
    }
    if mode == "ambiguous" {
        args.chunks.as_mut().unwrap()[0].old_text = "marker_".into();
    }
    if mode == "duplicate" && count > 1 {
        let first = args.chunks.as_ref().unwrap()[0].old_text.clone();
        args.chunks.as_mut().unwrap()[1].old_text = first;
    }
    (source.into_bytes(), args)
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn sample(engine: usize, bytes: &[u8], args: &ReplaceArgs, rounds: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..rounds {
        let result = if engine == 0 {
            reference::prepare(black_box(bytes), black_box(args))
        } else {
            prepare(black_box(bytes), black_box(args))
        };
        black_box(result).ok();
    }
    start.elapsed().as_secs_f64()
}

fn compare(label: &str, bytes: &[u8], args: &ReplaceArgs, initial_rounds: usize) {
    let expected = reference::prepare(bytes, args);
    let actual = prepare(bytes, args);
    assert_eq!(actual, expected, "full output or error mismatch: {label}");
    let mut rounds = [initial_rounds; 2];
    for engine in 0..2 {
        while rounds[engine] < 8192 && sample(engine, bytes, args, rounds[engine]) < 0.020 {
            rounds[engine] *= 2;
        }
    }
    let mut samples = [Vec::new(), Vec::new()];
    for run in 0..7 {
        for index in 0..2 {
            let engine = (run + index) % 2;
            let elapsed = sample(engine, bytes, args, rounds[engine]);
            samples[engine].push(elapsed * 1e6 / rounds[engine] as f64);
        }
    }
    let [old, new] = samples.map(median);
    println!(
        "{label},bytes={},chunks={},old_us={old:.3},new_us={new:.3},speedup={:.3}",
        bytes.len(),
        args.chunks.as_ref().unwrap().len(),
        old / new
    );
}

fn checks() {
    for mode in [
        "lf",
        "crlf",
        "bom_crlf",
        "fallback",
        "mixed",
        "long",
        "missing",
        "ambiguous",
        "duplicate",
    ] {
        for count in [1, 8, 16, 32, 64, 128] {
            let (bytes, args) = fixture(128 * 1024, count, mode);
            assert_eq!(
                prepare(&bytes, &args),
                reference::prepare(&bytes, &args),
                "mode={mode} count={count}"
            );
        }
    }
    let (bytes, mut args) = fixture(128 * 1024, 16, "crlf");
    args.hash = "0".repeat(64);
    assert_eq!(prepare(&bytes, &args), reference::prepare(&bytes, &args));
    assert_eq!(prepare(&bytes, &args).unwrap_err().0, "STALE");
    args.hash = hash(&bytes);
    args.bom = Some(true);
    assert_eq!(prepare(&bytes, &args), reference::prepare(&bytes, &args));
    args.bom = Some(false);
    assert_eq!(prepare(&bytes, &args), reference::prepare(&bytes, &args));
    args.chunks.as_mut().unwrap()[0].new_text = "\0".into();
    assert_eq!(prepare(&bytes, &args), reference::prepare(&bytes, &args));
    println!(
        "full_equivalence_checks=PASS; golden baseline includes request validation, matching, CRLF/BOM and exact error messages"
    );
}

fn main() {
    checks();
    println!(
        "Full prepare medians, 7 samples, rotating order; includes per-call automaton build when selected and supplied hash validation; excludes syntax validation and publication."
    );
    for size in [4 * 1024, 64 * 1024, 96 * 1024, 512 * 1024, 2 * 1024 * 1024] {
        for count in [1, 8, 16, 32, 64, 128] {
            let (bytes, args) = fixture(size, count, "lf");
            compare("lf", &bytes, &args, if size >= 512 * 1024 { 3 } else { 12 });
        }
    }
    for mode in [
        "crlf",
        "bom_crlf",
        "fallback",
        "mixed",
        "long",
        "missing",
        "ambiguous",
        "duplicate",
    ] {
        let (bytes, args) = fixture(512 * 1024, 32, mode);
        compare(mode, &bytes, &args, 3);
    }
}
