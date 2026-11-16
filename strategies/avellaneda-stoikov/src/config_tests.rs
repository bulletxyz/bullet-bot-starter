use super::*;

/// The `exchange` setting is advertised, but the build always trades on Bullet: any other
/// value would be accepted and ignored, so it is refused here.
#[test]
fn an_exchange_other_than_bullet_is_refused_not_ignored() {
    let config = |exchange: Option<&str>| -> AvellanedaStoikovConfig {
        let mut v = serde_json::json!({
            "symbol": "BTC-USD", "order_size": "0.001", "max_position": "0.01",
            "gamma": "0.1", "kappa": "1.5"
        });
        if let Some(e) = exchange {
            v["exchange"] = e.into();
        }
        serde_json::from_value(v).unwrap()
    };
    assert!(config(None).validate().is_ok());
    assert!(config(Some("bullet")).validate().is_ok());
    let e = config(Some("hyperliquid")).validate().unwrap_err();
    assert!(e.contains("exchange") && e.contains("bullet"), "{e}");
}

#[test]
fn every_declaration_is_well_formed() {
    for s in settings() {
        s.spec.check_declaration().unwrap_or_else(|e| panic!("{}: {e}", s.path));
    }
}
