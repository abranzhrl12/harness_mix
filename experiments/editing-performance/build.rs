use std::{env, fs, path::PathBuf};

fn main() {
    let source_path = "../../crates/domain/src/editing.rs";
    println!("cargo:rerun-if-changed={source_path}");
    let source = fs::read_to_string(source_path)
        .unwrap()
        .replace("\r\n", "\n");
    let source = source.split("#[cfg(test)]").next().unwrap();
    let source = source.lines().skip(1).collect::<Vec<_>>().join("\n");
    let from = "let first = normalized\n            .find(needle.as_ref())";
    let to = "let finder = memchr::memmem::Finder::new(needle.as_bytes());\n        let first = finder.find(normalized.as_bytes())";
    assert_eq!(
        source.matches(from).count(),
        1,
        "Update candidate generation after production search changes"
    );
    let source = source.replace(from, to);
    let from = "normalized[next..].contains(needle.as_ref())";
    assert_eq!(source.matches(from).count(), 1);
    let source = source.replace(from, "finder.find(normalized[next..].as_bytes()).is_some()");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("candidate.rs");
    fs::write(out, source).unwrap();
}
