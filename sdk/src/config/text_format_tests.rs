//! The tests of `text_format`, in modules by what they check.

#[cfg(test)]
mod tests {
    use crate::text_format::*;

    #[test]
    fn evm_addresses() {
        let ok = "0x0123456789abcdefABCDEF0123456789abcdef01";
        assert!(TextFormat::EvmAddress.check(ok).is_ok());
        for bad in ["", "0x12", "0123456789abcdefABCDEF0123456789abcdef01", &ok.replace('a', "g")] {
            assert!(TextFormat::EvmAddress.check(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_key_is_not_a_path() {
        assert!(TextFormat::KeyPath.check("/data/hl.key").is_ok());
        assert!(TextFormat::KeyPath.check("keys/hl").is_ok());
        let hex = "ab".repeat(32);
        assert!(TextFormat::KeyPath.check(&hex).is_err());
        assert!(TextFormat::KeyPath.check(&format!("0x{hex}")).is_err());
        assert!(TextFormat::KeyPath.check("").is_err());
    }

    /// Review tracks-8: every format refuses what looks like a key: 64 hex digits as a host, a
    /// header or a log target, and a Solana CLI keypair's content (a JSON byte array) as a path.
    #[test]
    fn a_key_is_never_any_format() {
        let hex = "ab".repeat(32);
        for f in [TextFormat::Host, TextFormat::Header, TextFormat::LogFilter, TextFormat::KeyPath]
        {
            assert!(f.check(&hex).is_err(), "{f:?}");
        }
        assert!(TextFormat::LogFilter.check(&format!("{hex}=info")).is_err());
        let keypair = format!("[{}]", vec!["174"; 64].join(","));
        assert!(TextFormat::KeyPath.check(&keypair).is_err(), "a keypair file's content");
        assert!(TextFormat::Free.check(&keypair).is_err());
        assert!(TextFormat::Host.check("bot.example.com").is_ok());
        assert!(TextFormat::Header.check("Fly-Client-IP").is_ok());
    }

    #[test]
    fn urls_are_https_but_for_localhost() {
        assert!(TextFormat::Url.check("https://api.example.com").is_ok());
        assert!(TextFormat::Url.check("http://localhost:8080").is_ok());
        assert!(TextFormat::Url.check("http://example.com").is_err());
        assert!(TextFormat::Url.check("ftp://x").is_err());
        assert!(
            TextFormat::Url.check("http://localhost:1@evil.example/").is_err(),
            "userinfo is not the host"
        );
        assert!(TextFormat::Url.check("http://127.0.0.1@evil.example").is_err());
        assert!(TextFormat::Url.check("http://[::1]:3030").is_ok());
        assert!(TextFormat::Url.check("http://[::1]/x").is_ok());
        assert!(TextFormat::Url.check("http://[::1]evil.example/").is_err());
    }
}

#[cfg(test)]
mod solana_tests {
    use crate::text_format::*;

    const WALLET: &str = "5ZWj7a1f8tWkjBESHKgrLmXshuXxqeY9SYcfbshpAqPG";

    #[test]
    fn a_wallet_is_base58_and_a_list_of_them_is_never_empty_or_a_key() {
        assert!(TextFormat::SolanaAddress.check(WALLET).is_ok());
        for bad in ["", "short", "0OIl0OIl0OIl0OIl0OIl0OIl0OIl0OIl0OIl", &"1".repeat(45), "0x12"] {
            assert!(TextFormat::SolanaAddress.check(bad).is_err(), "{bad}");
        }
        assert!(TextFormat::SolanaAddress.check_list(&[], true).is_err());
        assert!(TextFormat::SolanaAddress.check_list(&[WALLET.into()], true).is_ok());
        let twice = [WALLET.into(), WALLET.into()];
        assert!(TextFormat::SolanaAddress.check_list(&twice, true).is_err(), "a set");
        assert!(TextFormat::SolanaAddress.check_list(&twice, false).is_ok(), "a plain list");
        let names = ["A=1".into(), "A=2".into()];
        assert!(TextFormat::Pairs.check_list(&names, false).is_err(), "a map's name repeats");
        let key = "5".repeat(88);
        assert!(TextFormat::SolanaAddress.check(&key).is_err(), "a base58 key is no address");
    }
}

#[cfg(test)]
mod platform_tests {
    use crate::text_format::*;

    #[test]
    fn socket_host_header_and_log_filter_formats() {
        for (f, good, bad) in [
            (TextFormat::SocketAddr, "127.0.0.1:3030", "localhost"),
            (TextFormat::SocketAddr, "[::]:9091", "0.0.0.0:99999"),
            (TextFormat::Host, "bot.example.com", "https://bot.example.com"),
            (TextFormat::Host, "bot.example.com:8443", "bot.example.com/x"),
            (TextFormat::Header, "Fly-Client-IP", "bad header"),
            (TextFormat::LogFilter, "info", "verbose"),
            (TextFormat::LogFilter, "warn,bb_http=debug", "bb_http=loud"),
        ] {
            assert!(f.check(good).is_ok(), "{f:?} {good}");
            assert!(f.check(bad).is_err(), "{f:?} {bad}");
        }
    }
}
