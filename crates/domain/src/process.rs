//! Internal process contract. Never deserialize this from an MCP request.
use crate::Failure;
use std::time::Duration;

/// Logical tools, resolved by a platform adapter, never arbitrary shell text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    Cargo,
    Rustc,
    Git,
    Go,
    Python,
}

#[derive(Debug)]
pub struct ProcessSpec {
    program: Program,
    arguments: Vec<String>,
    timeout: Duration,
    output_limit: usize,
}

impl ProcessSpec {
    pub fn new(
        program: Program,
        arguments: Vec<String>,
        timeout: Duration,
        output_limit: usize,
    ) -> Result<Self, Failure> {
        if timeout.is_zero() || timeout > Duration::from_secs(30) {
            return Err((
                "PROCESS_BUDGET",
                "Process timeout must be within 30 seconds",
            ));
        }
        if output_limit == 0 || output_limit > 2 * 1024 * 1024 {
            return Err((
                "PROCESS_BUDGET",
                "Process output limit must be within 2 MiB",
            ));
        }
        if arguments.len() > 128
            || arguments.iter().any(|arg| arg.contains('\0'))
            || arguments.iter().map(String::len).sum::<usize>() > 16 * 1024
        {
            return Err(("PROCESS_ARGUMENTS", "Invalid or oversized argument list"));
        }
        Ok(Self {
            program,
            arguments,
            timeout,
            output_limit,
        })
    }

    pub fn program(&self) -> Program {
        self.program
    }
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
    pub fn output_limit(&self) -> usize {
        self.output_limit
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Exited,
    Cancelled,
    TimedOut,
    OutputLimit,
}

/// Bytes preserve split UTF-8 sequences; presentation decoding happens afterwards.
#[derive(Debug)]
pub struct ProcessOutcome {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
    pub reason: StopReason,
    /// False must never be presented as confirmed termination of descendants.
    pub tree_termination_confirmed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(args: Vec<String>, seconds: u64, limit: usize) -> Result<ProcessSpec, Failure> {
        ProcessSpec::new(Program::Cargo, args, Duration::from_secs(seconds), limit)
    }

    #[test]
    fn validates_budgets_and_arguments() {
        assert!(spec(vec![], 0, 1).is_err());
        assert!(spec(vec![], 31, 1).is_err());
        assert!(spec(vec![], 1, 0).is_err());
        assert!(spec(vec![], 1, 2 * 1024 * 1024 + 1).is_err());
        assert!(spec(vec!["a\0b".into()], 1, 1).is_err());
        assert!(spec(vec![String::new(); 129], 1, 1).is_err());
        assert!(spec(vec!["x".repeat(16 * 1024 + 1)], 1, 1).is_err());
        let valid = spec(
            vec!["test".into(), "path with spaces".into()],
            30,
            2 * 1024 * 1024,
        )
        .unwrap();
        assert_eq!(valid.program(), Program::Cargo);
        assert_eq!(valid.arguments()[1], "path with spaces");
        assert_eq!(valid.timeout(), Duration::from_secs(30));
        assert_eq!(valid.output_limit(), 2 * 1024 * 1024);
    }
}
