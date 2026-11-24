//! The commands the market maker answers, and the close.

use bb_strategy_sdk::param_spec::BoundsMode;
use bb_strategy_sdk::testing::d;
use bb_strategy_sdk::{
    CloseReport, ControlCmd, ControlReply, Event, Intent, LimitChanges, SettingChanges, Side,
    StrategyAdapter,
};
use serde_json::json;

use super::tests::Rig;

fn control(r: &mut Rig, id: u64, cmd: ControlCmd) -> Vec<Intent> {
    r.send(&Event::Control { id, cmd })
}

fn set(changes: SettingChanges) -> ControlCmd {
    ControlCmd::SetSettings { changes, reason: "test".into(), expected_effect: None }
}

fn reply(intents: &[Intent], id: u64) -> ControlReply {
    let replies: Vec<_> = intents
        .iter()
        .filter_map(|i| match i {
            Intent::Reply { id: of, reply } if *of == id => Some(reply.clone()),
            _ => None,
        })
        .collect();
    let [one] = replies.as_slice() else { panic!("one reply to {id}: {intents:?}") };
    one.clone()
}

fn ok(reply: &ControlReply) -> &str {
    match reply {
        ControlReply::Ok(text) => text,
        ControlReply::Refused(why) => panic!("refused: {why}"),
    }
}

fn refused(reply: &ControlReply) -> &str {
    match reply {
        ControlReply::Refused(why) => why,
        ControlReply::Ok(text) => panic!("applied: {text}"),
    }
}

/// A change is checked, applied, said back old to new, and quoted with on the next refresh.
#[test]
fn a_settings_change_applies_and_the_next_refresh_quotes_with_it() {
    let mut r = Rig::new(&json!({}));
    let changes = SettingChanges::default().set("bid_spread_bps", "50").set("ask_spread_bps", "50");
    let out = control(&mut r, 1, set(changes));
    assert_eq!(
        ok(&reply(&out, 1)),
        "Applied ask_spread_bps 10 → 50, bid_spread_bps 10 → 50. Takes effect on the next quote \
         refresh."
    );
    let sent = r.book("99", "101");
    let prices: Vec<_> = sent
        .iter()
        .map(|i| match i {
            Intent::Place(o) => o.price,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(prices, [d("99.5"), d("100.5")]);
}

/// Inside the guarded range: applied. Past it, inside the hard range: refused for the agent alone
/// (Guarded), applied once the config store authorized it (Extended). Past the hard range, never.
#[test]
fn a_change_is_held_to_its_range_by_the_mode_it_was_authorized_under() {
    let mut r = Rig::new(&json!({}));
    let mode = |bounds| SettingChanges { bounds, ..SettingChanges::default() };
    let tight = mode(BoundsMode::Guarded).set("bid_spread_bps", "0.5");
    let why = refused(&reply(&control(&mut r, 1, set(tight)), 1)).to_string();
    assert_eq!(why, "Not applied: bid_spread_bps=0.5 outside [2, 200].");
    let tight = mode(BoundsMode::Extended).set("bid_spread_bps", "0.5");
    ok(&reply(&control(&mut r, 2, set(tight)), 2));
    assert_eq!(r.mm.params().bid_spread_bps, d("0.5"));
    let past = mode(BoundsMode::Extended).set("skew_bps", "501");
    refused(&reply(&control(&mut r, 3, set(past)), 3));
    assert_eq!(r.mm.params().skew_bps, d("0"), "a refused change changes nothing");
}

/// The order size answers to the venue's step and half the position cap, in either mode.
#[test]
fn the_order_size_is_held_to_the_step_and_half_the_cap() {
    let mut r = Rig::new(&json!({}));
    r.m.market.step_size = Some(d("0.01"));
    r.book("99", "101");
    for (id, size) in [(1, "0.6"), (2, "0.005"), (3, "0")] {
        let c = SettingChanges { bounds: BoundsMode::Extended, ..SettingChanges::default() }
            .set("order_size", size);
        refused(&reply(&control(&mut r, id, set(c)), id));
    }
    let c = SettingChanges::default().set("order_size", "0.5");
    ok(&reply(&control(&mut r, 4, set(c)), 4));
}

/// A name that is no param, a word where a number goes, or a fair value that is no choice: refused,
/// in words.
#[test]
fn an_unknown_name_or_a_value_that_is_no_number_is_refused() {
    let mut r = Rig::new(&json!({}));
    for (id, (name, value, why)) in [
        ("max_position", "5", "Not applied: unknown parameter max_position."),
        ("bid_spread_bps", "wide", "Not applied: bid_spread_bps=wide must be a number."),
        (
            "fair_value",
            "coinbase",
            "Not applied: fair_value=coinbase must be one of binance, bullet.",
        ),
        ("refresh_secs", "1.5", "Not applied: refresh_secs=1.5 must be an integer."),
        (
            "vol_widen_enabled",
            "2",
            "Not applied: vol_widen_enabled=2 must be one of 0 (off), 1 (on).",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let id = u64::try_from(id).unwrap();
        let c = SettingChanges::default().set(name, value);
        assert_eq!(refused(&reply(&control(&mut r, id, set(c)), id)), why);
    }
}

/// The strategy's own params and the fair value change live too.
#[test]
fn its_own_params_and_the_fair_value_change_live() {
    let mut r = Rig::new(&json!({}));
    let c = SettingChanges::default().set("vol_widen_enabled", "1").set("fair_value", "binance");
    let text = ok(&reply(&control(&mut r, 1, set(c)), 1)).to_string();
    assert!(text.contains("fair_value bullet → binance"), "{text}");
    assert!(r.mm.params().flag("vol_widen_enabled"));
}

/// A pause, when the host leaves it to the market maker: its own orders are pulled, the reply
/// follows the pull's answer, nothing is quoted until the resume, which quotes at once.
#[test]
fn a_pause_pulls_and_replies_once_the_pull_is_back_and_a_resume_quotes_again() {
    let mut r = Rig::new(&json!({ "refresh_secs": 60 }));
    let sent = r.book("99", "101");
    r.take(&sent);
    r.m.position = d("0.3");
    assert_eq!(control(&mut r, 1, ControlCmd::Pause), [Intent::CancelOwn]);
    let out = r.take(&[Intent::CancelOwn]);
    assert_eq!(ok(&reply(&out, 1)), "Paused. All quotes cancelled; position 0.3 kept.");
    assert!(r.book("99", "101").is_empty());
    let out = control(&mut r, 2, ControlCmd::Resume);
    assert_eq!(ok(&reply(&out, 2)), "Resumed. Quoting restarts on the next book update.");
    assert_eq!(r.book("99", "101").len(), 2);
}

/// A pause whose pull failed still answers, and says the pull is retried.
#[test]
fn a_pause_whose_pull_failed_says_so() {
    let mut r = Rig::new(&json!({}));
    let out = control(&mut r, 1, ControlCmd::Pause);
    let out = r.refuse(&out[0], "down");
    assert!(ok(&reply(&out, 1)).contains("cancelling open orders failed; retrying every 10s"));
}

/// A close: no quote from then on, the host's close, and the reply from its report.
#[test]
fn a_close_stops_quoting_asks_the_host_and_words_the_reply_from_its_report() {
    let mut r = Rig::new(&json!({}));
    r.m.market.step_size = Some(d("0.001"));
    let sent = r.book("99", "101");
    r.take(&sent);
    assert_eq!(control(&mut r, 7, ControlCmd::ClosePosition), [Intent::ClosePosition { id: 7 }]);
    let again = control(&mut r, 8, ControlCmd::ClosePosition);
    assert!(refused(&reply(&again, 8)).contains("already in progress"));
    let report = CloseReport {
        cancel_error: None,
        start: Ok(d("0.5")),
        remaining: d("0"),
        attempts: 1,
        stopped_early: None,
        client_ids: Vec::new(),
        order_ids: Vec::new(),
    };
    let out = r.send(&Event::Closed { id: 7, report: report.clone() });
    assert_eq!(ok(&reply(&out, 7)), "Closed 0.500 BTC; 0.000 left. The bot is paused.");
    assert!(r.slot(Side::Buy).is_none(), "the host's close cancelled them");
    r.wait(5_000);
    assert!(r.book("99", "101").is_empty(), "paused");

    let mut words = |report: CloseReport| {
        control(&mut r, 9, ControlCmd::ClosePosition);
        reply(&r.send(&Event::Closed { id: 9, report }), 9)
    };
    let left = words(CloseReport {
        remaining: d("0.2"),
        attempts: 3,
        stopped_early: Some("slippage".into()),
        ..report.clone()
    });
    assert_eq!(
        ok(&left),
        "0.200 BTC still open after 3 attempts (closed 0.300 BTC). The bot is paused. Stopped \
         early: slippage."
    );
    let flat = words(CloseReport { start: Ok(d("0")), ..report.clone() });
    assert_eq!(ok(&flat), "Paused and pulled the quotes. The position is flat; nothing to close.");
    let dust = words(CloseReport { start: Ok(d("0.0004")), ..report.clone() });
    assert!(ok(&dust).contains("below the venue's minimum size"));
    let unread = words(CloseReport { start: Err("timeout".into()), ..report.clone() });
    assert!(refused(&unread).contains("could not read the position"));
    let stuck = words(CloseReport { cancel_error: Some("down".into()), ..report });
    assert!(refused(&stuck).contains("cancelling open orders failed"));
}

/// A close the host could not carry out is refused; the market maker stays paused.
#[test]
fn a_close_that_failed_as_a_whole_is_refused() {
    let mut r = Rig::new(&json!({}));
    let out = control(&mut r, 3, ControlCmd::ClosePosition);
    let out = r.send(&Event::Failed { intent: out[0].clone(), error: "down".into() });
    assert!(refused(&reply(&out, 3)).contains("could not be carried out"));
    assert!(r.book("99", "101").is_empty());
}

/// What is the host's or the supervisor's is refused, never half done.
#[test]
fn limits_the_kill_switch_and_staging_are_refused() {
    let mut r = Rig::new(&json!({}));
    let limits = LimitChanges { max_position: Some(d("0.5")), ..LimitChanges::default() };
    for (id, cmd) in [
        (1, ControlCmd::SetLimits(limits)),
        (
            2,
            ControlCmd::KillSwitch {
                scope: bb_strategy_sdk::instance::InstanceScope::All,
                engage: true,
            },
        ),
        (3, ControlCmd::Unstage { name: None }),
    ] {
        refused(&reply(&control(&mut r, id, cmd), id));
    }
}

/// Every command is answered exactly once, whether the host's answers succeed or fail.
#[test]
fn every_command_is_answered_exactly_once() {
    for fail in [false, true] {
        let mut r = Rig::new(&json!({}));
        let sent = r.book("99", "101");
        r.take(&sent);
        let commands = [
            ControlCmd::Pause,
            ControlCmd::Resume,
            ControlCmd::SetLimits(LimitChanges::default()),
            set(SettingChanges::default()),
            ControlCmd::ClosePosition,
            ControlCmd::Unstage { name: None },
        ];
        let mut all = Vec::new();
        for (id, cmd) in commands.into_iter().enumerate() {
            let id = u64::try_from(id).unwrap();
            let out = control(&mut r, id, cmd);
            all.extend(out.clone());
            for intent in &out {
                let answer = match intent {
                    Intent::CancelOwn if fail => r.refuse(intent, "down"),
                    Intent::CancelOwn => r.take(&[Intent::CancelOwn]),
                    Intent::ClosePosition { id } => {
                        let report = CloseReport {
                            cancel_error: fail.then(|| "down".into()),
                            start: Ok(d("0")),
                            remaining: d("0"),
                            attempts: 0,
                            stopped_early: None,
                            client_ids: Vec::new(),
                            order_ids: Vec::new(),
                        };
                        r.send(&Event::Closed { id: *id, report })
                    }
                    _ => Vec::new(),
                };
                all.extend(answer);
            }
        }
        for id in 0..6 {
            reply(&all, id);
        }
        assert!(r.mm.may_keep() || fail);
    }
}
