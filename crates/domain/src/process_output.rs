//! Shared byte budget for stdout/stderr. No lossy decoding until collection ends.
use crate::process::ProcessSpec;

pub struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    limit: usize,
    limited: bool,
}

impl Output {
    pub fn new(spec: &ProcessSpec) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            limit: spec.output_limit(),
            limited: false,
        }
    }

    /// Returns false when the caller must terminate the child tree.
    pub fn push(&mut self, stderr: bool, bytes: &[u8]) -> bool {
        let remaining = self.limit - self.stdout.len() - self.stderr.len();
        let count = bytes.len().min(remaining);
        let target = if stderr {
            &mut self.stderr
        } else {
            &mut self.stdout
        };
        target.extend_from_slice(&bytes[..count]);
        self.limited |= count < bytes.len();
        !self.limited
    }

    pub fn finish(self) -> (Vec<u8>, Vec<u8>, bool) {
        (self.stdout, self.stderr, self.limited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::Program;
    use std::time::Duration;

    fn output(limit: usize) -> Output {
        Output::new(
            &ProcessSpec::new(Program::Cargo, vec![], Duration::from_secs(1), limit).unwrap(),
        )
    }

    #[test]
    fn shares_budget_between_pipes_and_retains_prefix() {
        let mut out = output(5);
        assert!(out.push(false, b"abc"));
        assert!(!out.push(true, b"def"));
        assert!(!out.push(false, b"ignored"));
        let (stdout, stderr, limited) = out.finish();
        assert_eq!(stdout, b"abc");
        assert_eq!(stderr, b"de");
        assert!(limited);
    }

    #[test]
    fn exact_budget_is_not_truncation_and_split_utf8_is_preserved() {
        let mut out = output(7);
        let bytes = "\u{feff}🙂".as_bytes();
        for byte in bytes {
            assert!(out.push(false, &[*byte]));
        }
        assert!(out.push(true, b""));
        let (stdout, stderr, limited) = out.finish();
        assert_eq!(String::from_utf8(stdout).unwrap(), "\u{feff}🙂");
        assert!(stderr.is_empty());
        assert!(!limited);
    }
}
