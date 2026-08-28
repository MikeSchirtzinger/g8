//! Adapter implementing [`ConflictStore`] over any `g8_store::StoreConnection`.
//!
//! This is the crate's own bridge from the full `StoreConnection` API to the
//! narrower read-only [`ConflictStore`] surface conflict detection needs —
//! the SPEC Decision 8 `g8-conflict -> g8-store` edge, realized at the
//! TRAIT level. It is deliberately generic over `S: StoreConnection`
//! (mirroring `g8-planner`'s `plan_check<S>` pattern) so no concrete store
//! type is ever named in this crate (OBL-D5-01).
//!
//! Each method is a 1-3 line delegation to the corresponding
//! `StoreConnection` method, converting the rich `g8-core` types to the
//! lightweight row types expected by [`ConflictStore`].

use std::path::Path;

use g8_core::SpaceId;
use g8_store::StoreConnection;

use crate::store_trait::{CapabilityRow, DecisionRow, IntentRow, PlanRow};
use crate::ConflictStore;

/// Wrapper that makes any [`StoreConnection`] satisfy [`ConflictStore`].
pub struct ConflictAdapter<'a, S: StoreConnection> {
    store: &'a S,
}

impl<'a, S: StoreConnection> ConflictAdapter<'a, S> {
    /// Wrap a store reference for conflict detection.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
}

impl<'a, S: StoreConnection> ConflictStore for ConflictAdapter<'a, S> {
    fn list_capabilities(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<CapabilityRow>, Box<dyn std::error::Error + Send + Sync>> {
        // Collect capabilities across all projects in this space.
        let projects = self.store.list_projects(space).map_err(box_err)?;
        let mut rows = Vec::new();
        for project in &projects {
            let caps = self.store.list_capabilities(&project.id).map_err(box_err)?;
            for cap in caps {
                rows.push(CapabilityRow {
                    id: cap.id,
                    name: cap.name,
                    project_name: project.name.clone(),
                });
            }
        }
        Ok(rows)
    }

    fn list_intents(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<IntentRow>, Box<dyn std::error::Error + Send + Sync>> {
        // list_intents_at_path with the empty path returns all intents in the space.
        let intents = self
            .store
            .list_intents_at_path(space, Path::new(""))
            .map_err(box_err)?;
        let rows = intents
            .into_iter()
            .map(|i| IntentRow {
                id: i.id,
                kind: i.kind,
                heading: i.heading,
                description: i.description,
                scope_path: i.scope_path,
            })
            .collect();
        Ok(rows)
    }

    fn list_decisions(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<DecisionRow>, Box<dyn std::error::Error + Send + Sync>> {
        let decisions = self.store.list_decisions(space).map_err(box_err)?;
        let rows = decisions
            .into_iter()
            .map(|d| DecisionRow {
                id: d.id,
                title: d.title,
                status_accepted: d.status == g8_core::model::DecisionStatus::Accepted,
                body: d.body,
            })
            .collect();
        Ok(rows)
    }

    fn list_plans(
        &self,
        space: &SpaceId,
    ) -> Result<Vec<PlanRow>, Box<dyn std::error::Error + Send + Sync>> {
        let filter = g8_store::PlanFilter {
            status: None,
            substrate: None,
            project: None,
            limit: None,
        };
        let plans = self.store.list_plans(space, filter).map_err(box_err)?;
        let rows = plans
            .into_iter()
            .map(|p| PlanRow {
                id: p.id,
                title: p.title,
                substrate: p.substrate,
            })
            .collect();
        Ok(rows)
    }

    fn resolve_canonical(
        &self,
        ref_name: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.store.resolve_canonical(ref_name).map_err(box_err)
    }
}

fn box_err<E: std::error::Error + Send + Sync + 'static>(
    e: E,
) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(e)
}
