#[path = "../../../../src/adapters/search/matcher_cache.rs"]
mod matcher_cache;

use std::{hint::black_box, time::Instant};

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    for (name, pattern, options) in [
        (
            "literal",
            "cachedNeedle".to_owned(),
            matcher_cache::Options {
                literal: true,
                case_sensitive: true,
                multi_line: true,
            },
        ),
        (
            "fallback16",
            (0..16)
                .map(|i| format!("term{i:03}"))
                .collect::<Vec<_>>()
                .join("|"),
            matcher_cache::Options {
                literal: false,
                case_sensitive: false,
                multi_line: false,
            },
        ),
    ] {
        matcher_cache::get(&pattern, options).unwrap();
        let mut cold = Vec::new();
        let mut warm = Vec::new();
        for sample in 0..9 {
            for index in 0..2 {
                let hit = (sample + index) % 2 == 0;
                let start = Instant::now();
                for _ in 0..50 {
                    let matcher = if hit {
                        matcher_cache::get(black_box(&pattern), options).unwrap()
                    } else {
                        grep_regex::RegexMatcherBuilder::new()
                            .fixed_strings(options.literal)
                            .case_insensitive(!options.case_sensitive)
                            .multi_line(options.multi_line)
                            .line_terminator(Some(b'\n'))
                            .build(black_box(&pattern))
                            .unwrap()
                    };
                    black_box(matcher);
                }
                let micros = start.elapsed().as_secs_f64() * 1e6 / 50.0;
                if hit {
                    warm.push(micros);
                } else {
                    cold.push(micros);
                }
            }
        }
        let (cold, warm) = (median(cold), median(warm));
        println!(
            "{name}: uncached_compile_us={cold:.3} cache_hit_us={warm:.3} preparation_speedup={:.2}x",
            cold / warm
        );
    }
    println!(
        "Includes actual production cache lookup, mutex, recency update and matcher clone; excludes file IO and the rest of search."
    );
}
