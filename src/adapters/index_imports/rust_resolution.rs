use super::{Failure, INDEX, OperationContext, Project, VerifiedSources, sql};
use super::rust_analysis_cache::RustFactsCache;
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Rust module and call facts, kept separate from import resolver coordination.
pub(super) struct RustResolution {
    rust: BTreeMap<String, Arc<super::super::rust::Facts>>,
    rust_reserved: usize,
    cache: RustFactsCache,
}

impl Default for RustResolution {
    fn default() -> Self {
        Self::with_cache(RustFactsCache::default())
    }
}

impl RustResolution {
    pub(super) fn with_cache(cache: RustFactsCache) -> Self {
        Self {
            rust: BTreeMap::new(),
            rust_reserved: 0,
            cache,
        }
    }
}

#[cfg(test)]
#[path = "rust_resolution_tests.rs"]
mod cache_tests;

impl RustResolution {
    fn analyzed(
        &mut self,
        sources: &mut VerifiedSources,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
    ) -> Result<bool, Failure> {
        if self.rust.contains_key(path) {
            crate::adapters::index_profile::count("rust_facts_hits", 1);
            context.check()?;
            return Ok(true);
        }
        crate::adapters::index_profile::count("rust_facts_misses", 1);
        if !sources.contains(path) && sources.len() >= 64 {
            return Ok(false);
        }
        if !sources.observe(db, project, context, path)? {
            return Ok(false);
        }
        let source = std::str::from_utf8(sources.bytes(path).ok_or(INDEX)?).map_err(|_| INDEX)?;
        context.check()?;
        let cached = {
            let _timer = crate::adapters::index_profile::operation("rust_analysis_cache_lookup");
            let digest = super::super::hash(source.as_bytes());
            (digest.clone(), self.cache.get(source.as_bytes(), &digest))
        };
        let (digest, hit) = cached;
        if let Some(facts) = hit {
            context.check()?;
            crate::adapters::index_profile::count("rust_analysis_cache_hits", 1);
            let reserved = facts.reserved();
            if self.rust_reserved.saturating_add(reserved) > 16 * 1024 * 1024 {
                self.rust.insert(path.into(), Arc::new(super::super::rust::Facts::default()));
            } else {
                self.rust_reserved += reserved;
                self.rust.insert(path.into(), facts);
            }
            return Ok(true);
        }
        crate::adapters::index_profile::count("rust_analysis_cache_misses", 1);
        context.check()?;
        let analyzed = {
            let _timer = crate::adapters::index_profile::operation("rust_analyze");
            crate::adapters::index_profile::count("rust_analyze_bytes", source.len() as u64);
            super::super::rust::analyze(source, context)
        };
        let (facts, cacheable) = match analyzed {
            Ok(facts) => (Arc::new(facts), true),
            Err(("PARSER_LIMIT" | "SYNTAX_ERROR", _)) => {
                (Arc::new(super::super::rust::Facts::default()), false)
            }
            Err(error) => return Err(error),
        };
        context.check()?;
        if cacheable {
            context.check()?;
            let _timer = crate::adapters::index_profile::operation("rust_analysis_cache_insert");
            self.cache.insert(source.as_bytes(), digest, Arc::clone(&facts));
        }
        let reserved = facts.reserved();
        if self.rust_reserved.saturating_add(reserved) > 16 * 1024 * 1024 {
            self.rust
                .insert(path.into(), Arc::new(super::super::rust::Facts::default()));
        } else {
            self.rust_reserved += reserved;
            self.rust.insert(path.into(), facts);
        }
        Ok(true)
    }

    fn rooted(
        &mut self,
        sources: &mut VerifiedSources,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        depth: usize,
    ) -> Result<bool, Failure> {
        if depth > 8 {
            return Ok(false);
        }
        let Some((dir, file)) = path.rsplit_once('/') else {
            return Ok(false);
        };
        if matches!(file, "lib.rs" | "main.rs") {
            return Ok(true);
        }
        if file != "mod.rs" {
            return Ok(false);
        }
        let Some((parent_dir, module)) = dir.rsplit_once('/') else {
            return Ok(false);
        };
        let mut parents = Vec::new();
        for file in ["mod.rs", "lib.rs", "main.rs"] {
            let parent = format!("{parent_dir}/{file}");
            if self.analyzed(sources, db, project, context, &parent)?
                && self.rust[&parent].modules.contains(module)
            {
                parents.push(parent);
            }
        }
        if parents.len() != 1 {
            return Ok(false);
        }
        self.rooted(sources, db, project, context, &parents[0], depth + 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn target(
        &mut self,
        sources: &mut VerifiedSources,
        db: &Connection,
        project: &Project,
        context: &OperationContext,
        path: &str,
        name: &str,
        line: i64,
        col: i64,
    ) -> Result<Option<String>, Failure> {
        let _timer = crate::adapters::index_profile::operation("rust_target");
        if !self.analyzed(sources, db, project, context, path)? {
            return Ok(None);
        }
        let Some(parts) = self.rust[path]
            .call(name, line, col)
            .map(<[String]>::to_vec)
        else {
            crate::adapters::index_profile::count("rust_call_proof_miss", 1);
            return Ok(None);
        };
        let (target, symbol) = if parts.len() == 1 {
            (path.to_owned(), parts[0].clone())
        } else if parts.len() == 3 && parts[0] == "super" {
            // Only siblings declared by one indexed parent module. No disk
            // probing, guessed global name or arbitrary suffix matching.
            let Some((dir, file)) = path.rsplit_once('/') else {
                return Ok(None);
            };
            let Some(stem) = file.strip_suffix(".rs") else {
                return Ok(None);
            };
            if matches!(stem, "mod" | "lib" | "main") {
                return Ok(None);
            }
            let mut parents = Vec::new();
            for parent in [
                format!("{dir}/mod.rs"),
                format!("{dir}/lib.rs"),
                format!("{dir}/main.rs"),
            ] {
                if self.analyzed(sources, db, project, context, &parent)?
                    && self.rust[&parent].modules.contains(stem)
                    && self.rust[&parent].modules.contains(&parts[1])
                {
                    parents.push(parent);
                }
            }
            if parents.len() != 1 {
                return Ok(None);
            }
            if !self.rooted(sources, db, project, context, &parents[0], 0)? {
                return Ok(None);
            }
            let mut targets = Vec::new();
            for target in [
                format!("{dir}/{}.rs", parts[1]),
                format!("{dir}/{}/mod.rs", parts[1]),
            ] {
                if sql(db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM files WHERE path=?1)",
                    [&target],
                    |r| r.get::<_, bool>(0),
                ))? {
                    targets.push(target);
                }
            }
            if targets.len() != 1 {
                return Ok(None);
            }
            let target = targets.remove(0);
            if !self.analyzed(sources, db, project, context, &target)? {
                return Ok(None);
            }
            (target, parts[2].clone())
        } else {
            return Ok(None);
        };
        let Some(&(start, public)) = self.rust[&target].functions.get(&symbol) else {
            return Ok(None);
        };
        if target != path && !public {
            return Ok(None);
        }
        let mut statement = sql(db.prepare_cached("SELECT id FROM nodes WHERE file_path=?1 AND name=?2 AND start_line=?3 AND kind='function' AND language='rust' LIMIT 2"))?;
        let ids = sql(sql(
            statement.query_map(rusqlite::params![target, symbol, start], |r| {
                r.get::<_, String>(0)
            }),
        )?
        .collect::<Result<Vec<_>, _>>())?;
        Ok(if ids.len() == 1 {
            ids.into_iter().next()
        } else {
            None
        })
    }
}
