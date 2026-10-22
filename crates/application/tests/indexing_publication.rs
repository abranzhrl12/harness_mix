use harness_application::{
    indexing::{PublicationStage as Stage, publish},
    ports::indexing::PublicationSession,
};
use harness_domain::{Failure, execution::OperationContext};

const STAGES: [Stage; 10] = [
    Stage::CheckIndexedHash,
    Stage::ExtractSource,
    Stage::CaptureExistingGraph,
    Stage::DeletePreviousGraph,
    Stage::WriteNodesAndEdges,
    Stage::RestoreIncomingEdges,
    Stage::InvalidateReferences,
    Stage::ResolveReferences,
    Stage::RecheckFrameworkSources,
    Stage::WriteMetadata,
];

#[derive(Default)]
struct FakeSession {
    events: Vec<Stage>,
    fail: Option<Stage>,
    cancel: Option<(Stage, OperationContext)>,
    unchanged: bool,
    finished: bool,
}

impl FakeSession {
    fn check(&self) -> Result<(), Failure> {
        if self.fail == self.events.last().copied() {
            Err(("INJECTED", "Stage failure"))
        } else {
            Ok(())
        }
    }
}

impl PublicationSession for FakeSession {
    type Extraction = u8;
    type Captured = u8;
    type Written = u8;
    type Invalidation = u8;
    type Resolution = u8;
    type Output = u8;

    fn stage(&mut self, stage: Stage) {
        self.events.push(stage);
        if let Some((at, context)) = &self.cancel {
            if *at == stage {
                context.cancel();
            }
        }
    }
    fn unchanged(&mut self, _: &OperationContext) -> Result<Option<u8>, Failure> {
        self.check()?;
        Ok(self.unchanged.then_some(42))
    }
    fn extract(&mut self, _: &OperationContext) -> Result<u8, Failure> {
        self.check()?;
        Ok(1)
    }
    fn capture(&mut self, extracted: &u8, _: &OperationContext) -> Result<u8, Failure> {
        assert_eq!(*extracted, 1);
        self.check()?;
        Ok(2)
    }
    fn remove_previous(&mut self, captured: &u8, _: &OperationContext) -> Result<(), Failure> {
        assert_eq!(*captured, 2);
        self.check()
    }
    fn write(
        &mut self,
        extracted: &u8,
        captured: &u8,
        _: &OperationContext,
    ) -> Result<u8, Failure> {
        assert_eq!((*extracted, *captured), (1, 2));
        self.check()?;
        Ok(3)
    }
    fn restore(
        &mut self,
        captured: &u8,
        written: &mut u8,
        _: &OperationContext,
    ) -> Result<(), Failure> {
        assert_eq!((*captured, *written), (2, 3));
        self.check()?;
        *written = 4;
        Ok(())
    }
    fn invalidate(
        &mut self,
        captured: &u8,
        written: &mut u8,
        _: &OperationContext,
    ) -> Result<u8, Failure> {
        assert_eq!((*captured, *written), (2, 4));
        self.check()?;
        *written = 5;
        Ok(6)
    }
    fn resolve(
        &mut self,
        written: &u8,
        invalidation: &u8,
        _: &OperationContext,
    ) -> Result<u8, Failure> {
        assert_eq!((*written, *invalidation), (5, 6));
        self.check()?;
        Ok(7)
    }
    fn recheck(&mut self, resolution: &u8, _: &OperationContext) -> Result<(), Failure> {
        assert_eq!(*resolution, 7);
        self.check()
    }
    fn metadata(
        &mut self,
        captured: &u8,
        resolution: &u8,
        _: &OperationContext,
    ) -> Result<(), Failure> {
        assert_eq!((*captured, *resolution), (2, 7));
        self.check()
    }
    fn finish(
        &mut self,
        extracted: u8,
        captured: u8,
        written: u8,
        invalidation: u8,
        resolution: u8,
    ) -> Result<u8, Failure> {
        assert_eq!(
            (extracted, captured, written, invalidation, resolution),
            (1, 2, 5, 6, 7)
        );
        self.finished = true;
        Ok(99)
    }
}

#[test]
fn pipeline_runs_in_order_and_passes_each_stage_output() {
    let mut session = FakeSession::default();
    assert_eq!(
        publish(&mut session, &OperationContext::without_deadline()),
        Ok(99)
    );
    assert_eq!(session.events, STAGES);
    assert!(session.finished);
}

#[test]
fn every_stage_failure_stops_later_stages_and_preserves_error() {
    for (index, stage) in STAGES.iter().enumerate() {
        let mut session = FakeSession {
            fail: Some(*stage),
            ..Default::default()
        };
        assert_eq!(
            publish(&mut session, &OperationContext::without_deadline())
                .unwrap_err()
                .0,
            "INJECTED"
        );
        assert_eq!(session.events, STAGES[..=index]);
        assert!(!session.finished);
    }
}

#[test]
fn cancellation_at_every_boundary_stops_before_the_stage_operation() {
    for (index, stage) in STAGES.iter().enumerate() {
        let context = OperationContext::without_deadline();
        let mut session = FakeSession {
            cancel: Some((*stage, context.clone())),
            ..Default::default()
        };
        assert_eq!(publish(&mut session, &context).unwrap_err().0, "CANCELLED");
        assert_eq!(session.events, STAGES[..=index]);
        assert!(!session.finished);
    }
}

#[test]
fn unchanged_sources_do_not_extract_or_write() {
    let mut session = FakeSession {
        unchanged: true,
        ..Default::default()
    };
    assert_eq!(
        publish(&mut session, &OperationContext::without_deadline()),
        Ok(42)
    );
    assert_eq!(session.events, [Stage::CheckIndexedHash]);
    assert!(!session.finished);
}

#[test]
fn expired_context_does_not_start_the_session() {
    let context = OperationContext::with_timeout(std::time::Duration::ZERO);
    let mut session = FakeSession::default();
    assert_eq!(publish(&mut session, &context).unwrap_err().0, "TIMEOUT");
    assert!(session.events.is_empty());
}
