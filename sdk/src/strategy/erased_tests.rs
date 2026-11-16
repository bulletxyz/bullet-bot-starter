use super::*;
use crate::{CandleInterval, ReportField};

#[derive(Debug)]
struct Quiet;

impl StrategyReport for Quiet {
    fn headline(&self) -> String {
        String::new()
    }
    fn fields(&self) -> Vec<ReportField> {
        Vec::new()
    }
}

/// A strategy with the one interval of the WIT world's `history`.
#[derive(Default)]
struct OneInterval {
    warmed: Vec<Warmup>,
}

const NEED: HistoryNeed = HistoryNeed { interval: CandleInterval::M5, want: 30, need: 10 };

impl StrategyAdapter for OneInterval {
    type Report = Quiet;
    fn start(&mut self, _: &View<'_>) -> Result<Vec<Intent>, StrategyError> {
        Ok(Vec::new())
    }
    fn on_event(&mut self, _: &Event, _: &View<'_>) -> Vec<Intent> {
        Vec::new()
    }
    fn stop(&mut self, _: &View<'_>) -> Vec<Intent> {
        Vec::new()
    }
    fn report(&self) -> Quiet {
        Quiet
    }
    fn history(&self) -> Option<HistoryNeed> {
        Some(NEED)
    }
    fn warm_up(&mut self, warmup: Warmup) {
        self.warmed.push(warmup);
    }
}

/// A host that loads every need of the list warms a one-interval strategy as it always did.
#[test]
fn the_list_of_history_needs_is_the_one_need_unless_a_strategy_names_more() {
    let mut s: Box<dyn ErasedAdapter> = Box::new(OneInterval::default());
    assert_eq!(s.histories(), [NEED]);
    let none = Warmup::Unavailable("no source".into());
    for need in s.histories() {
        s.warm_up_for(need, none.clone());
    }
    let s = s.as_any().downcast_ref::<OneInterval>().unwrap();
    assert_eq!(s.warmed, [none]);
}
