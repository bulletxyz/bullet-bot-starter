use super::*;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn money_shows_whole_dollars_bare_and_cents_to_two_places() {
    assert_eq!(money(d("250")), "$250");
    assert_eq!(money(d("250.5")), "$250.50");
    assert_eq!(text(d("0.00120")), "0.0012");
}

#[test]
fn a_percent_and_a_floor_round_down_never_up() {
    assert_eq!(pct(d("100"), 5), d("5"));
    assert_eq!(pct(d("33.33"), 5), d("1.66"));
    assert_eq!(floor_to(d("0.00125"), d("0.0001")), d("0.0012"));
    assert_eq!(floor_to(d("1"), Decimal::ZERO), Decimal::ZERO);
}

#[test]
fn sizing_needs_a_positive_price_and_step() {
    let good = PresetMarket { price: d("80000"), step: d("0.0001") };
    assert_eq!(usable("BTC", Some(good)), Ok(good));
    assert!(usable("BTC", None).unwrap_err().contains("needs the market's price"));
    let zero = PresetMarket { price: d("80000"), step: Decimal::ZERO };
    assert!(usable("BTC", Some(zero)).unwrap_err().contains("no usable price"));
}

/// A strategy with no profile (every plugin) has none: the page falls back to its summary.
#[test]
fn a_strategy_has_no_profile_unless_it_gives_one() {
    struct Bare;
    impl crate::Describe for Bare {
        const NAME: &'static str = "bare";
        const VERSION: &'static str = "0.1.0";
        fn describe() -> crate::Descriptor {
            crate::Descriptor {
                summary: String::new(),
                settings: Vec::new(),
                settings_schema: schemars::Schema::default(),
                legs: Vec::new(),
            }
        }
    }
    assert!(<Bare as crate::Describe>::profile().is_none());
}

#[test]
fn the_owner_page_types_serialize_as_the_bot_reads_them() {
    let about = About {
        title: "Grid",
        tagline: "t",
        blurb: "b",
        risk: Risk { level: RiskLevel::MediumHigh, text: "r" },
        featured: vec![
            Featured::new("amount_usd", "Amount", "h", "${}"),
            Featured::new("p", "P", "h", "{}").percent(),
        ],
    };
    let json = serde_json::to_value(&about).unwrap();
    assert_eq!(json["risk"]["level"], "medium_high");
    assert!(json["featured"][0].get("percent").is_none());
    assert_eq!(json["featured"][1]["percent"], true);
    let need = ExchangeNeed {
        role: "venue",
        setting: Some("exchange"),
        default: "bullet",
        tradable: true,
        required: true,
    };
    let json = serde_json::to_value(need).unwrap();
    let want = serde_json::json!({
        "role": "venue", "setting": "exchange", "default": "bullet", "tradable": true, "required": true
    });
    assert_eq!(json, want);
}
