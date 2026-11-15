//! The model's checks on the ladder: at every step, and once everything has drained.

use super::*;

impl Model {
    pub fn check_final(&self) {
        let report = self.ladder.report();
        if report.fills != self.completions {
            self.fail(&format!(
                "fills {} but the model counted {} completions",
                report.fills, self.completions
            ));
        }
        for rung in &report.levels {
            let name =
                format!("ladder-{}{}", if rung.side == Side::Buy { "b" } else { "s" }, rung.index);
            let alive: Vec<&String> = self
                .orders
                .iter()
                .filter(|(_, o)| o.level == name && o.venue_live && !o.committed_refusal)
                .map(|(i, _)| i)
                .collect();
            if alive.len() > 1 {
                self.fail(&format!("{name}: {} live orders at the venue: {alive:?}", alive.len()));
            }
            // The venue's truth: nothing live at the venue may overfill the level, and the rung's
            // fill is the model's own sum of what the ladder was told.
            let known = self.known_fill(&name);
            for id in alive {
                let left = self
                    .orders
                    .get(id)
                    .map_or(Decimal::ZERO, |o| o.order.quantity.saturating_sub(o.known_fill()));
                if known.saturating_add(left) > self.size {
                    self.fail(&format!(
                        "{id} still rests at the venue and overfills {name} ({known} filled, {left} left)"
                    ));
                }
            }
            if matches!(rung.state, LevelState::Resting | LevelState::Pending)
                && rung.filled != known
            {
                self.fail(&format!(
                    "{name}: reports {} filled, the model knows {known}",
                    rung.filled
                ));
            }
            let possibly = self.orders.values().any(|o| o.level == name && !o.told_gone);
            match rung.state {
                LevelState::Resting | LevelState::Pending if !possibly => {
                    self.fail(&format!("{name} is {:?} with no live order", rung.state));
                }
                LevelState::Refused if !self.levels.get(&name).is_some_and(|l| l.limit_refused) => {
                    self.fail(&format!(
                        "{name} is left refused though nothing refused it for good"
                    ));
                }
                _ => {}
            }
        }
    }
    /// A possibly-live order that would overfill its level (its remainder on top of what the ladder
    /// knows is filled) has had its cancel asked: nothing is left resting to overfill the level.
    pub(super) fn check_overfill(&self) {
        // The ledger is a hard bound: a plugin's work and memory are metered.
        for level in &self.ladder.levels {
            assert!(
                level.order_count() <= crate::level::MAX_ORDERS,
                "a level's ledger grew to {}",
                level.order_count()
            );
        }
        for (id, o) in &self.orders {
            if o.told_gone || !o.asks.is_empty() {
                continue;
            }
            let left = o.order.quantity.saturating_sub(o.known_fill());
            let level_fill = self.known_fill(&o.level);
            if level_fill.saturating_add(left) > self.size {
                self.fail(&format!(
                    "{id} would overfill its level ({level_fill} filled, {left} left) and no cancel was asked"
                ));
            }
        }
    }
}
