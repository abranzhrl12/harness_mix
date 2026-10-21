use super::super::{hash, read_source};
use super::{Failure, LIMIT, OperationContext, Project, sql};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

/// Source bytes accepted only when they still match the indexed publication.
#[derive(Default)]
pub(super) struct VerifiedSources {
    files: BTreeMap<String, Vec<u8>>,
    reserved: usize,
}

impl VerifiedSources {
    pub(super) fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    pub(super) fn len(&self) -> usize {
        self.files.len()
    }

    pub(super) fn bytes(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub(super) fn observe(
        &mut self,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
    ) -> Result<bool, Failure> {
        if !self.files.contains_key(path) {
            if self.files.len() >= 64 || self.reserved + 1024 * 1024 > 64 * 1024 * 1024 {
                return Err(LIMIT);
            }
            let Some(bytes) = read_source(project, path, context)? else {
                return Ok(false);
            };
            let digest: Option<String> = sql(db
                .query_row(
                    "SELECT content_hash FROM files WHERE path=?1",
                    [path],
                    |r| r.get(0),
                )
                .optional())?;
            if digest.as_deref() != Some(hash(&bytes).as_str()) {
                return Ok(false);
            }
            self.reserved += bytes.len();
            self.files.insert(path.into(), bytes);
        }
        Ok(true)
    }

    pub(super) fn recheck(
        &self,
        project: &Project,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        for (path, bytes) in &self.files {
            if read_source(project, path, context)?
                .as_ref()
                .map(|b| hash(b))
                != Some(hash(bytes))
            {
                return Err((
                    "STALE",
                    "Import resolution source changed; transaction rolled back",
                ));
            }
        }
        Ok(())
    }
}
