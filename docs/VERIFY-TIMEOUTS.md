# Verification timeouts

`harness_verify` accepts the optional `timeoutSeconds` field. Omission or `null` uses the Rust backend default of 900 seconds (15 minutes). Values must be integers from 1 through 3600; zero and values above one hour are rejected.

The configured value is the outer verifier-process deadline. The synchronous request deadline adds a fixed 5-second cleanup grace so the process adapter can collect bounded output and stop the process tree. Cancellation remains active throughout; this grace is not additional build time. Tool-specific internal deadlines can still apply: for example, Go tests keep their 20-second default unless `timeoutSeconds` is explicitly set, in which case the Go test deadline follows that value.

This budget applies only to `harness_verify`. Other foreground tools retain their existing short limits. In `harness_jobs`, the outer `timeoutSeconds` is a separate override (up to 24 hours); when omitted, a nested `verify.timeoutSeconds` is honored if provided. If both are omitted, managed jobs retain their existing no-deadline behavior. Browser operations retain their independent 20-second operation deadline.
