//! SQLite reference invalidation strategies and transaction-local models.
//!
//! The application owns the port and language policy; this adapter owns SQL,
//! declaration snapshots and retry memo integration. Publication owns commit/rollback.

use super::super::{
    LIMIT, MAX_ROWS, imports, razor_invalidation, resolution::RetryMemo, sql, sync_diagnostic,
};
use super::retry::{self, Declaration};
use harness_application::{
    indexing::InvalidationPolicy, ports::indexing::ReferenceInvalidatorPort,
};
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::Transaction;
use std::collections::BTreeSet;

/// Infrastructure binding of the application port; SQLite stays in this adapter.
pub(in crate::adapters::index_store) type ReferenceInvalidator =
    dyn for<'db, 'scope> ReferenceInvalidatorPort<
            Transaction<'db>,
            InvalidationScope<'scope>,
            Outcome = InvalidationOutcome,
        >;

/// Transaction-local SQLite inputs. Not a domain model.
pub(in crate::adapters::index_store) struct InvalidationScope<'a> {
    pub(in crate::adapters::index_store) path: &'a str,
    pub(in crate::adapters::index_store) language: &'a str,
    pub(in crate::adapters::index_store) config_changed: bool,
    pub(in crate::adapters::index_store) before_declarations: &'a Option<Vec<Declaration>>,
    pub(in crate::adapters::index_store) razor_changes: &'a Option<razor_invalidation::Changes>,
    pub(in crate::adapters::index_store) retry_memo: &'a mut RetryMemo,
}

/// Pending reference IDs and diagnostics for this publication.
#[derive(Debug, Default)]
pub(in crate::adapters::index_store) struct InvalidationOutcome {
    pub recheck_refs: BTreeSet<i64>,
    pub import_recheck_count: usize,
    pub framework_recheck_count: usize,
    pub razor_recheck_count: usize,
    pub after_declarations: Option<Vec<Declaration>>,
    pub declaration_pool_changed: bool,
}

/// Production driven adapter for incremental / single-file indexing (Default Strategy).
///
/// Executes reactive cross-file invalidation for modified JS/TS/framework sources,
/// queries pending references for newly introduced declarations, and schedules rechecks.
#[derive(Debug, Default, Clone, Copy)]
pub(in crate::adapters::index_store) struct IncrementalSqliteInvalidator;

impl<'db, 'scope> ReferenceInvalidatorPort<Transaction<'db>, InvalidationScope<'scope>>
    for IncrementalSqliteInvalidator
{
    type Outcome = InvalidationOutcome;
    fn invalidate(
        &self,
        db: &Transaction<'db>,
        context: &OperationContext,
        scope: InvalidationScope<'scope>,
    ) -> Result<InvalidationOutcome, Failure> {
        context.check()?;
        let InvalidationScope {
            path,
            language,
            config_changed,
            before_declarations,
            razor_changes,
            retry_memo,
        } = scope;

        sync_diagnostic::stage("invalidate_references");
        let after_declarations = if retry_memo.enabled() {
            retry::declarations(db, path)?
        } else {
            None
        };
        let declaration_pool_changed = config_changed
            || before_declarations
                .as_ref()
                .zip(after_declarations.as_ref())
                .is_none_or(|(before, after)| before != after);
        if declaration_pool_changed {
            retry_memo.clear();
        }

        let mut outcome = InvalidationOutcome {
            after_declarations,
            declaration_pool_changed,
            ..Default::default()
        };

        let policy = InvalidationPolicy::for_publication(language, config_changed);
        let (import_rechecks, framework_rechecks) =
            imports::invalidate_publication(db, policy.imports, policy.frameworks)?;
        outcome.import_recheck_count = import_rechecks.len();
        outcome.recheck_refs.extend(import_rechecks);
        outcome.framework_recheck_count = framework_rechecks.len();
        outcome.recheck_refs.extend(framework_rechecks);

        if policy.rust {
            outcome.recheck_refs.extend(imports::invalidate_rust(db)?);
        }

        if let Some(changes) = razor_changes {
            let rechecks = changes.recheck(db, context)?;
            outcome.razor_recheck_count = rechecks.len();
            outcome.recheck_refs.extend(rechecks);
        }

        if outcome.recheck_refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }

        // Retry the pending refs whose exact name became available in this file.
        let import_bindings = if policy.imports {
            imports::retry_binding_catalog(db)?
        } else {
            None
        };
        for id in retry::pending(
            db,
            path,
            language,
            config_changed,
            &retry_memo.json(),
            import_bindings.as_deref(),
        )? {
            outcome.recheck_refs.insert(id);
            if outcome.recheck_refs.len() > MAX_ROWS {
                return Err(LIMIT);
            }
        }

        // Registration refs do not necessarily share an exported class's local alias.
        if policy.providers {
            let mut provider_refs = sql(db.prepare(
                "SELECT id FROM unresolved_refs WHERE status='pending' AND reference_kind IN ('angular_provider_token','angular_provider_implementation') LIMIT 20001",
            ))?;
            for row in sql(provider_refs.query_map([], |row| row.get::<_, i64>(0)))? {
                outcome.recheck_refs.insert(sql(row)?);
                if outcome.recheck_refs.len() > MAX_ROWS {
                    return Err(LIMIT);
                }
            }
        }

        Ok(outcome)
    }
}

/// Compatibility strategy for workspace indexing. Until an atomic final retry pass
/// exists, batch mode must retain incremental correctness (including warm rebuilds).
#[derive(Debug, Default, Clone, Copy)]
pub(in crate::adapters::index_store) struct BatchSqliteInvalidator;

impl<'db, 'scope> ReferenceInvalidatorPort<Transaction<'db>, InvalidationScope<'scope>>
    for BatchSqliteInvalidator
{
    type Outcome = InvalidationOutcome;

    fn invalidate(
        &self,
        db: &Transaction<'db>,
        context: &OperationContext,
        scope: InvalidationScope<'scope>,
    ) -> Result<Self::Outcome, Failure> {
        IncrementalSqliteInvalidator.invalidate(db, context, scope)
    }
}

#[cfg(all(test, windows))]
#[path = "invalidation_tests.rs"]
mod tests;
