//! Scoped current projections use one read-only snapshot after reader progress.
//! No caller may refresh a cursor while retaining rows from an earlier snapshot.
use super::*;

pub struct ProjectionReaderSnapshot {
    pub heartbeat: Option<BaseIndexerHeartbeat>,
    pub cursor: Option<BaseLogScanCursor>,
}

pub struct AutonomousProjectionSnapshot {
    pub reader: ProjectionReaderSnapshot,
    pub events: Vec<AutonomousBountyEvent>,
    pub terms: Vec<AutonomousBountyTermsRecord>,
}

pub struct OpenCompetitionProjectionSnapshot {
    pub reader: ProjectionReaderSnapshot,
    pub events: Vec<OpenCompetitionEvent>,
}

pub struct OpenCompetitionV2ProjectionSnapshot {
    pub reader: ProjectionReaderSnapshot,
    pub agreement: OpenCompetitionV2IndexerAgreement,
    pub records: Vec<OpenCompetitionV2StoredProjection>,
    pub events: Vec<OpenCompetitionV2Event>,
}

fn unavailable() -> DbError {
    DbError::InvalidEnum("projection reader evidence unavailable".into())
}

async fn begin(store: &PostgresStore) -> DbResult<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = store.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

async fn reader(
    connection: &mut sqlx::PgConnection,
    network: &str,
    factory: &str,
) -> DbResult<ProjectionReaderSnapshot> {
    // Heartbeat precedes cursor, including every progress re-observation.
    let heartbeat =
        PostgresStore::get_base_indexer_heartbeat_on(&mut *connection, network, factory).await?;
    let cursor = PostgresStore::get_base_log_cursor_on(&mut *connection, network, factory).await?;
    // Explicit quarantine is terminal even if the last successful heartbeat is
    // still fresh. Missing certification is checked by the scoped data loader.
    if PostgresStore::autonomous_canonical_checkpoint_on(&mut *connection, network, factory)
        .await?
        .is_some_and(|checkpoint| checkpoint.invalidated)
        || PostgresStore::get_open_competition_v2_indexer_agreement_on(
            &mut *connection,
            network,
            factory,
        )
        .await?
        .is_some_and(|agreement| !agreement.agrees || agreement.failure_code.is_some())
    {
        return Err(unavailable());
    }
    Ok(ProjectionReaderSnapshot { heartbeat, cursor })
}

impl PostgresStore {
    pub async fn projection_reader_snapshot(
        &self,
        network: &str,
        factory: &str,
    ) -> DbResult<ProjectionReaderSnapshot> {
        let mut tx = begin(self).await?;
        let snapshot = reader(&mut tx, network, factory).await?;
        tx.commit().await?;
        Ok(snapshot)
    }

    pub async fn autonomous_projection_snapshot(
        &self,
        network: &str,
        factory: &str,
        contract: Option<&str>,
        through: u64,
    ) -> DbResult<AutonomousProjectionSnapshot> {
        let mut tx = begin(self).await?;
        let reader = reader(&mut tx, network, factory).await?;
        let checkpoint =
            Self::autonomous_canonical_checkpoint_on(&mut *tx, network, factory).await?;
        if checkpoint
            .is_none_or(|checkpoint| checkpoint.invalidated || checkpoint.block_number < through)
        {
            return Err(unavailable());
        }
        let mut events = match contract {
            Some(contract) => {
                Self::list_autonomous_bounty_history_for_contract_on(
                    &mut *tx, network, factory, contract,
                )
                .await?
            }
            None => {
                Self::list_autonomous_bounty_events_for_factory_on(
                    &mut *tx, network, factory, false,
                )
                .await?
            }
        };
        events.retain(|event| event.block_number <= through);
        let hashes: std::collections::BTreeSet<_> = events
            .iter()
            .filter(|event| {
                event.kind == chain_base::AutonomousBountyEventKind::CanonicalBountyCreated
            })
            .filter_map(|event| {
                event
                    .data
                    .get("terms_hash")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::to_ascii_lowercase)
            .collect();
        let mut terms = Vec::new();
        for hash in hashes {
            if let Some(record) = Self::get_autonomous_bounty_terms_on(&mut *tx, &hash).await? {
                terms.push(record);
            }
        }
        tx.commit().await?;
        Ok(AutonomousProjectionSnapshot {
            reader,
            events,
            terms,
        })
    }

    pub async fn open_competition_projection_snapshot(
        &self,
        network: &str,
        factory: &str,
        contract: Option<&str>,
        through: u64,
    ) -> DbResult<OpenCompetitionProjectionSnapshot> {
        let mut tx = begin(self).await?;
        let reader = reader(&mut tx, network, factory).await?;
        let mut events = match contract {
            Some(contract) => {
                Self::list_open_competition_history_for_contract_on(
                    &mut *tx, network, factory, contract,
                )
                .await?
            }
            None => Self::list_open_competition_events_on(&mut *tx, network, factory).await?,
        };
        events.retain(|event| event.block_number <= through);
        tx.commit().await?;
        Ok(OpenCompetitionProjectionSnapshot { reader, events })
    }

    pub async fn open_competition_v2_projection_snapshot(
        &self,
        network: &str,
        factory: &str,
        contract: Option<&str>,
        through: u64,
    ) -> DbResult<OpenCompetitionV2ProjectionSnapshot> {
        let mut tx = begin(self).await?;
        let reader = reader(&mut tx, network, factory).await?;
        let agreement =
            Self::get_open_competition_v2_indexer_agreement_on(&mut *tx, network, factory)
                .await?
                .ok_or_else(unavailable)?;
        let records = match contract {
            Some(contract) => {
                Self::list_open_competition_v2_projections_for_contract_on(
                    &mut *tx, network, factory, contract,
                )
                .await?
            }
            None => {
                Self::list_open_competition_v2_projections_on(&mut *tx, network, factory).await?
            }
        };
        // These are current-state projections, not historical event rows. Never
        // discard a newer incompatible row and describe the source as empty.
        if records
            .iter()
            .any(|record| record.projection.last_block > through.min(agreement.common_safe_block))
        {
            return Err(unavailable());
        }
        let mut events = match contract {
            Some(contract) => {
                Self::list_open_competition_v2_history_for_contract_on(
                    &mut *tx, network, factory, contract,
                )
                .await?
            }
            None => Self::list_open_competition_v2_events_on(&mut *tx, network, factory).await?,
        };
        events.retain(|event| event.block_number <= through.min(agreement.common_safe_block));
        tx.commit().await?;
        Ok(OpenCompetitionV2ProjectionSnapshot {
            reader,
            agreement,
            records,
            events,
        })
    }
}

#[cfg(test)]
#[path = "projection_snapshot_tests.rs"]
mod tests;
