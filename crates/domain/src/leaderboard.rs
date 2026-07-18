use chrono::{DateTime, Datelike, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use utoipa::ToSchema;

pub const DAILY_LEADERBOARD_REWARD_USDC_BASE_UNITS: u64 = 3_000_000;
pub const WEEKLY_LEADERBOARD_REWARD_USDC_BASE_UNITS: u64 = 26_000_000;
pub const LEADERBOARD_MINIMUM_SOLVER_REWARD_USDC_BASE_UNITS: u64 = 2_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardPeriodKind {
    Daily,
    Weekly,
}

impl LeaderboardPeriodKind {
    pub fn reward_usdc_base_units(self) -> u64 {
        match self {
            Self::Daily => DAILY_LEADERBOARD_REWARD_USDC_BASE_UNITS,
            Self::Weekly => WEEKLY_LEADERBOARD_REWARD_USDC_BASE_UNITS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LeaderboardPeriod {
    pub kind: LeaderboardPeriodKind,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

impl LeaderboardPeriod {
    pub fn contains(&self, occurred_at: DateTime<Utc>) -> bool {
        occurred_at >= self.starts_at && occurred_at < self.ends_at
    }
}

pub fn leaderboard_period(kind: LeaderboardPeriodKind, at: DateTime<Utc>) -> LeaderboardPeriod {
    let day_start = at
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_utc();
    let starts_at = match kind {
        LeaderboardPeriodKind::Daily => day_start,
        LeaderboardPeriodKind::Weekly => {
            day_start - Duration::days(i64::from(at.weekday().num_days_from_monday()))
        }
    };
    let ends_at = starts_at
        + match kind {
            LeaderboardPeriodKind::Daily => Duration::days(1),
            LeaderboardPeriodKind::Weekly => Duration::days(7),
        };
    LeaderboardPeriod {
        kind,
        starts_at,
        ends_at,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSolverCompletion {
    pub bounty_id: String,
    pub bounty_contract: String,
    pub solver_wallet: String,
    pub creator_wallet: String,
    pub solver_reward_usdc_base_units: u64,
    pub occurred_at: DateTime<Utc>,
    pub block_number: u64,
    pub log_index: u64,
    pub standing_meta_bounty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SolverLeaderboardEntry {
    pub rank: u32,
    pub solver_wallet: String,
    pub completed_bounties: u32,
    pub prize_eligible_bounties: u32,
    pub excluded_bounties: u32,
    pub eligible_solver_rewards_usdc_base_units: String,
    pub last_eligible_settlement_at: Option<DateTime<Utc>>,
    pub exclusion_counts: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SolverLeaderboardRanking {
    pub period: LeaderboardPeriod,
    pub minimum_solver_reward_usdc_base_units: String,
    pub reward_usdc_base_units: String,
    pub leader_wallet: Option<String>,
    pub entries: Vec<SolverLeaderboardEntry>,
    pub rules: Vec<String>,
}

#[derive(Default)]
struct EntryAccumulator {
    completed_bounties: u32,
    prize_eligible_bounties: u32,
    eligible_solver_rewards_usdc_base_units: u64,
    last_eligible_settlement_at: Option<DateTime<Utc>>,
    last_eligible_block_number: Option<u64>,
    last_eligible_log_index: Option<u64>,
    exclusion_counts: BTreeMap<String, u32>,
    counted_creators: HashSet<String>,
}

pub fn rank_solver_completions(
    period: LeaderboardPeriod,
    completions: impl IntoIterator<Item = CanonicalSolverCompletion>,
) -> SolverLeaderboardRanking {
    let mut completions = completions
        .into_iter()
        .filter(|completion| period.contains(completion.occurred_at))
        .collect::<Vec<_>>();
    completions.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then(left.block_number.cmp(&right.block_number))
            .then(left.log_index.cmp(&right.log_index))
            .then_with(|| left.bounty_contract.cmp(&right.bounty_contract))
    });

    let mut by_solver = HashMap::<String, EntryAccumulator>::new();
    for completion in completions {
        let solver = completion.solver_wallet.to_ascii_lowercase();
        let creator = completion.creator_wallet.to_ascii_lowercase();
        let entry = by_solver.entry(solver.clone()).or_default();
        entry.completed_bounties = entry.completed_bounties.saturating_add(1);

        let exclusion = if completion.standing_meta_bounty {
            Some("standing_meta_bounty")
        } else if completion.solver_reward_usdc_base_units
            < LEADERBOARD_MINIMUM_SOLVER_REWARD_USDC_BASE_UNITS
        {
            Some("solver_reward_below_2_usdc")
        } else if creator == solver {
            Some("creator_is_solver")
        } else if !entry.counted_creators.insert(creator) {
            Some("creator_already_counted_for_solver")
        } else {
            None
        };

        if let Some(reason) = exclusion {
            *entry
                .exclusion_counts
                .entry(reason.to_string())
                .or_default() += 1;
            continue;
        }

        entry.prize_eligible_bounties = entry.prize_eligible_bounties.saturating_add(1);
        entry.eligible_solver_rewards_usdc_base_units = entry
            .eligible_solver_rewards_usdc_base_units
            .saturating_add(completion.solver_reward_usdc_base_units);
        entry.last_eligible_settlement_at = Some(completion.occurred_at);
        entry.last_eligible_block_number = Some(completion.block_number);
        entry.last_eligible_log_index = Some(completion.log_index);
    }

    let mut entries = by_solver.into_iter().collect::<Vec<_>>();
    entries.sort_by(|(left_wallet, left), (right_wallet, right)| {
        right
            .prize_eligible_bounties
            .cmp(&left.prize_eligible_bounties)
            .then_with(|| {
                left.last_eligible_settlement_at
                    .cmp(&right.last_eligible_settlement_at)
            })
            .then_with(|| {
                left.last_eligible_block_number
                    .cmp(&right.last_eligible_block_number)
            })
            .then_with(|| {
                left.last_eligible_log_index
                    .cmp(&right.last_eligible_log_index)
            })
            .then_with(|| left_wallet.cmp(right_wallet))
    });

    let entries = entries
        .into_iter()
        .enumerate()
        .map(|(index, (solver_wallet, value))| SolverLeaderboardEntry {
            rank: u32::try_from(index + 1).unwrap_or(u32::MAX),
            solver_wallet,
            completed_bounties: value.completed_bounties,
            prize_eligible_bounties: value.prize_eligible_bounties,
            excluded_bounties: value
                .completed_bounties
                .saturating_sub(value.prize_eligible_bounties),
            eligible_solver_rewards_usdc_base_units: value
                .eligible_solver_rewards_usdc_base_units
                .to_string(),
            last_eligible_settlement_at: value.last_eligible_settlement_at,
            exclusion_counts: value.exclusion_counts,
        })
        .collect::<Vec<_>>();
    let leader_wallet = entries
        .first()
        .filter(|entry| entry.prize_eligible_bounties > 0)
        .map(|entry| entry.solver_wallet.clone());

    SolverLeaderboardRanking {
        reward_usdc_base_units: period.kind.reward_usdc_base_units().to_string(),
        period,
        minimum_solver_reward_usdc_base_units: LEADERBOARD_MINIMUM_SOLVER_REWARD_USDC_BASE_UNITS
            .to_string(),
        leader_wallet,
        entries,
        rules: vec![
            "Count confirmed BountySettled events with verified block time.".to_string(),
            "Require at least 2 USDC solver reward.".to_string(),
            "Exclude standing meta-bounties.".to_string(),
            "Count one completion per creator and solver per period.".to_string(),
            "Break ties by earliest final qualifying settlement, then block, log, and wallet."
                .to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Weekday};

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0)
            .single()
            .unwrap()
    }

    fn completion(
        solver: &str,
        creator: &str,
        occurred_at: DateTime<Utc>,
        reward: u64,
        block_number: u64,
    ) -> CanonicalSolverCompletion {
        CanonicalSolverCompletion {
            bounty_id: format!("bounty-{block_number}"),
            bounty_contract: format!("0x{block_number:040x}"),
            solver_wallet: solver.to_string(),
            creator_wallet: creator.to_string(),
            solver_reward_usdc_base_units: reward,
            occurred_at,
            block_number,
            log_index: 0,
            standing_meta_bounty: false,
        }
    }

    #[test]
    fn periods_use_utc_and_monday_week_start() {
        let reference = at(17, 12);
        let daily = leaderboard_period(LeaderboardPeriodKind::Daily, reference);
        assert_eq!(daily.starts_at, at(17, 0));
        assert_eq!(daily.ends_at, at(18, 0));

        let weekly = leaderboard_period(LeaderboardPeriodKind::Weekly, reference);
        assert_eq!(weekly.starts_at.weekday(), Weekday::Mon);
        assert_eq!(weekly.starts_at, at(13, 0));
        assert_eq!(weekly.ends_at, at(20, 0));
    }

    #[test]
    fn ties_go_to_earliest_final_qualifying_settlement() {
        let period = leaderboard_period(LeaderboardPeriodKind::Daily, at(17, 12));
        let ranking = rank_solver_completions(
            period,
            [
                completion("0xbbb", "0x101", at(17, 2), 2_000_000, 10),
                completion("0xaaa", "0x201", at(17, 1), 2_000_000, 11),
                completion("0xbbb", "0x102", at(17, 4), 2_000_000, 12),
                completion("0xaaa", "0x202", at(17, 3), 2_000_000, 13),
            ],
        );

        assert_eq!(ranking.leader_wallet.as_deref(), Some("0xaaa"));
        assert_eq!(ranking.entries[0].prize_eligible_bounties, 2);
        assert_eq!(ranking.entries[1].prize_eligible_bounties, 2);
    }

    #[test]
    fn low_value_meta_and_repeated_creator_work_remain_visible_but_ineligible() {
        let period = leaderboard_period(LeaderboardPeriodKind::Daily, at(17, 12));
        let mut meta = completion("0xaaa", "0x102", at(17, 3), 2_000_000, 12);
        meta.standing_meta_bounty = true;
        let ranking = rank_solver_completions(
            period,
            [
                completion("0xaaa", "0x101", at(17, 1), 2_000_000, 10),
                completion("0xaaa", "0x101", at(17, 2), 3_000_000, 11),
                meta,
                completion("0xaaa", "0x103", at(17, 4), 1_999_999, 13),
            ],
        );

        let entry = &ranking.entries[0];
        assert_eq!(entry.completed_bounties, 4);
        assert_eq!(entry.prize_eligible_bounties, 1);
        assert_eq!(entry.excluded_bounties, 3);
        assert_eq!(
            entry.exclusion_counts["creator_already_counted_for_solver"],
            1
        );
        assert_eq!(entry.exclusion_counts["standing_meta_bounty"], 1);
        assert_eq!(entry.exclusion_counts["solver_reward_below_2_usdc"], 1);
    }

    #[test]
    fn period_end_is_exclusive() {
        let period = leaderboard_period(LeaderboardPeriodKind::Daily, at(17, 12));
        let ranking = rank_solver_completions(
            period,
            [completion("0xaaa", "0x101", at(18, 0), 2_000_000, 10)],
        );
        assert!(ranking.entries.is_empty());
    }
}
