//! Historical backfill planning and execution.
//!
//! `BackfillPlan::compute` is a pure function that reconciles the requested
//! `start_block` against an existing cursor and the current tip, producing
//! 0–2 `BackfillRange`s. The downward-then-upward ordering preserves the
//! single-contiguous-range cursor invariant across crashes.

use crate::models::IndexerCursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillDirection {
    Upward,
    Downward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillRange {
    /// Inclusive lower bound.
    pub from: u64,
    /// Inclusive upper bound. Always `from <= to`.
    pub to: u64,
    pub direction: BackfillDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackfillPlan {
    /// Ordered ranges to execute. Downward ranges always precede upward ranges
    /// to preserve the contiguous-range invariant under crashes.
    pub ranges: Vec<BackfillRange>,
}

impl BackfillPlan {
    pub fn compute(
        start_block: u64,
        existing_cursor: Option<&IndexerCursor>,
        tip: u64,
    ) -> Self {
        let mut ranges = Vec::new();

        match existing_cursor {
            None => {
                // Fresh DB. Single upward range from start_block to tip, if any.
                if start_block <= tip {
                    ranges.push(BackfillRange {
                        from: start_block,
                        to: tip,
                        direction: BackfillDirection::Upward,
                    });
                }
            }
            Some(c) if start_block >= c.first_indexed_block => {
                // Already have as much or more history. Resume upward from cursor top.
                if c.last_indexed_block < tip {
                    ranges.push(BackfillRange {
                        from: c.last_indexed_block + 1,
                        to: tip,
                        direction: BackfillDirection::Upward,
                    });
                }
            }
            Some(c) => {
                // start_block < c.first_indexed_block: two-phase extend.
                // Phase 1: gap-fill downward [start_block, first - 1].
                ranges.push(BackfillRange {
                    from: start_block,
                    to: c.first_indexed_block - 1,
                    direction: BackfillDirection::Downward,
                });
                // Phase 2: forward-extend upward [last + 1, tip] (if any).
                if c.last_indexed_block < tip {
                    ranges.push(BackfillRange {
                        from: c.last_indexed_block + 1,
                        to: tip,
                        direction: BackfillDirection::Upward,
                    });
                }
            }
        }

        BackfillPlan { ranges }
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    use crate::models::BlockHash;
    use chrono::Utc;

    fn cursor(first: u64, last: u64) -> IndexerCursor {
        IndexerCursor {
            chain_id: "test".into(),
            first_indexed_block: first,
            last_indexed_block: last,
            last_indexed_hash: BlockHash([0u8; 32]),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn fresh_db_backfills_from_start_to_tip() {
        let plan = BackfillPlan::compute(0, None, 100);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[0].from, 0);
        assert_eq!(plan.ranges[0].to, 100);
    }

    #[test]
    fn fresh_db_respects_non_zero_start_block() {
        let plan = BackfillPlan::compute(500, None, 1000);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[0].from, 500);
        assert_eq!(plan.ranges[0].to, 1000);
    }

    #[test]
    fn resume_ignores_start_when_covered() {
        let plan = BackfillPlan::compute(10, Some(&cursor(0, 50)), 100);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[0].from, 51);
        assert_eq!(plan.ranges[0].to, 100);
    }

    #[test]
    fn extend_below_triggers_two_phase_downward_then_upward() {
        let plan = BackfillPlan::compute(0, Some(&cursor(100, 200)), 300);
        assert_eq!(plan.ranges.len(), 2);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Downward);
        assert_eq!(plan.ranges[0].from, 0);
        assert_eq!(plan.ranges[0].to, 99);
        assert_eq!(plan.ranges[1].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[1].from, 201);
        assert_eq!(plan.ranges[1].to, 300);
    }

    #[test]
    fn already_at_tip_plans_nothing() {
        let plan = BackfillPlan::compute(0, Some(&cursor(0, 100)), 100);
        assert!(plan.ranges.is_empty());
    }

    #[test]
    fn downward_only_when_no_upward_needed() {
        let plan = BackfillPlan::compute(0, Some(&cursor(100, 200)), 200);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Downward);
        assert_eq!(plan.ranges[0].from, 0);
        assert_eq!(plan.ranges[0].to, 99);
    }
}
