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
    Node,
    Dotnet,
    Msbuild,
}

#[derive(Debug)]
pub struct ProcessSpec {
    program: Program,
    arguments: Vec<String>,
    timeout: Duration,
    output_limit: usize,
    background_timeout: Option<Option<Duration>>,
}

impl ProcessSpec {
    pub fn new(
        program: Program,
        arguments: Vec<String>,
        timeout: Duration,
        output_limit: usize,
    ) -> Result<Self, Failure> {
        if timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err((
                "PROCESS_BUDGET",
                "Process timeout must be within 60 seconds",
            ));
        }
        let maximum_output = if matches!(program, Program::Dotnet | Program::Msbuild) {
            32 * 1024 * 1024
        } else {
            2 * 1024 * 1024
        };
        if output_limit == 0 || output_limit > maximum_output {
            return Err((
                "PROCESS_BUDGET",
                "Process output exceeds its tool-specific bounded budget",
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
            background_timeout: None,
        })
    }

    /// Explicit opt-in for managed background processes. Synchronous callers
    /// retain the bounded 60-second constructor budget.
    pub fn with_background_timeout(mut self, timeout: Option<Duration>) -> Result<Self, Failure> {
        if timeout.is_some_and(|value| value.is_zero() || value > Duration::from_secs(86400)) {
            return Err((
                "PROCESS_BUDGET",
                "Background timeout must be 1 second to 24 hours",
            ));
        }
        self.background_timeout = Some(timeout);
        Ok(self)
    }

    pub fn timeout_option(&self) -> Option<Duration> {
        self.background_timeout.unwrap_or(Some(self.timeout))
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
        assert!(spec(vec![], 61, 1).is_err());
        assert!(spec(vec![], 60, 1).is_ok());
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

    #[test]
    fn dotnet_has_a_larger_but_bounded_warning_capture_budget() {
        for program in [Program::Dotnet, Program::Msbuild] {
            assert!(
                ProcessSpec::new(program, vec![], Duration::from_secs(30), 32 * 1024 * 1024)
                    .is_ok()
            );
            assert!(
                ProcessSpec::new(
                    program,
                    vec![],
                    Duration::from_secs(30),
                    32 * 1024 * 1024 + 1
                )
                .is_err()
            );
        }
    }

    #[test]
    fn background_deadline_is_optional_and_separate_from_sync_budget() {
        let unlimited = spec(vec![], 30, 1024)
            .unwrap()
            .with_background_timeout(None)
            .unwrap();
        assert_eq!(unlimited.timeout_option(), None);
        let bounded = spec(vec![], 30, 1024)
            .unwrap()
            .with_background_timeout(Some(Duration::from_secs(3600)))
            .unwrap();
        assert_eq!(bounded.timeout_option(), Some(Duration::from_secs(3600)));
        assert!(
            spec(vec![], 30, 1024)
                .unwrap()
                .with_background_timeout(Some(Duration::ZERO))
                .is_err()
        );
        assert!(
            spec(vec![], 30, 1024)
                .unwrap()
                .with_background_timeout(Some(Duration::from_secs(86401)))
                .is_err()
        );
    }
}
