//! The format a text or list setting must have, checked where a value enters (a config change
//! over MCP or the web API, `bb setup --set`), so a bad address never reaches a connection.

use serde::Serialize;

/// What a `Text` setting's value (or each item of a `List`) has to look like.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    /// Anything non-blank.
    #[default]
    Free,
    /// An EVM address: `0x` and 40 hex digits.
    EvmAddress,
    /// A path to a key file. Never the key itself: key-looking content is refused.
    KeyPath,
    /// `NAME=value`: one entry of a map setting, stored as a table.
    Pairs,
    /// An HTTP(S) URL; plain `http` only for a local host.
    Url,
    /// A Solana wallet address: base58, 32 to 44 characters. A list of them is never empty.
    SolanaAddress,
    /// A socket address, `host-ip:port` (`127.0.0.1:3030`, `[::]:9091`).
    SocketAddr,
    /// A host name for URLs and Host checks: letters, digits, `.` and `-`, an optional `:port`;
    /// no scheme and no path.
    Host,
    /// An HTTP header name.
    Header,
    /// A log filter: `info`, or `target=level` directives joined by commas.
    LogFilter,
}

impl TextFormat {
    /// The `NAME=value` items of a [`Pairs`](Self::Pairs) list as (name, value) entries, in order.
    #[must_use]
    pub fn pairs(items: &[String]) -> Vec<(String, String)> {
        items
            .iter()
            .filter_map(|i| i.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect()
    }

    /// A [`LogFilter`](Self::LogFilter) that logs less than `info` for some target (a `warn`,
    /// `error` or `off` level, bare or `target=level`, or no bare level at all: every other target
    /// then logs errors only): it hides what the owner reads.
    #[must_use]
    pub fn quieter_than_info(filter: &str) -> bool {
        let quiet = filter.split(',').any(|d| {
            let level = d.rsplit_once('=').map_or(d, |(_, l)| l).trim().to_ascii_lowercase();
            matches!(level.as_str(), "warn" | "error" | "off")
        });
        quiet || !filter.split(',').any(|d| !d.contains('='))
    }

    /// `Err(why)` in plain words when `value` does not fit. A refusal never repeats the value.
    ///
    /// # Errors
    /// The value is blank, too long, looks like a private key (except an address) or is not of
    /// this format.
    pub fn check(self, value: &str) -> Result<(), String> {
        let v = value.trim();
        if v.is_empty() {
            return Err("must not be blank".into());
        }
        if v.len() > MAX_LEN {
            return Err(format!("is too long (at most {MAX_LEN} characters)"));
        }
        // Whatever the format: a key is never a config value (a host, a header, a log target).
        no_key(v)?;
        match self {
            Self::Free => Ok(()),
            Self::EvmAddress => {
                let hex = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X"));
                match hex {
                    Some(h) if h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit()) => {
                        if h.bytes().all(|b| b == b'0') {
                            Err("is the zero address, which is no account".into())
                        } else {
                            Ok(())
                        }
                    }
                    _ => Err("must be an EVM address: 0x and 40 hex digits".into()),
                }
            }
            Self::KeyPath => {
                if v.contains('\0') || v.contains('\n') {
                    Err("must be a file path".into())
                } else {
                    Ok(())
                }
            }
            Self::Pairs => match v.split_once('=') {
                Some((k, val)) if !k.trim().is_empty() && !val.trim().is_empty() => {
                    no_key(k.trim()).and_then(|()| no_key(val.trim()))
                }
                _ => Err("must be NAME=value".into()),
            },
            Self::SocketAddr => v.parse::<std::net::SocketAddr>().map(drop).map_err(|_| {
                "must be an address and port such as 127.0.0.1:3030 or [::]:9091".into()
            }),
            Self::Host => host(v),
            Self::Header => header(v),
            Self::LogFilter => {
                v.split([',', '=']).try_for_each(|part| no_key(part.trim()))?;
                log_filter(v)
            }
            Self::SolanaAddress => {
                const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
                if (32..=44).contains(&v.len()) && v.chars().all(|c| B58.contains(c)) {
                    Ok(())
                } else {
                    Err("must be a Solana wallet address (base58, 32 to 44 characters)".into())
                }
            }
            Self::Url => {
                let local = |rest: &str| {
                    let authority = rest.split('/').next().unwrap_or_default();
                    // A `user@` before the host would make a remote host look local.
                    let host = if authority.contains('@') {
                        ""
                    } else if authority.starts_with('[') {
                        let ip = authority.split_inclusive(']').next().unwrap_or_default();
                        let port = authority.strip_prefix(ip).unwrap_or_default();
                        if port.is_empty() || port.starts_with(':') { ip } else { "" }
                    } else {
                        authority.split(':').next().unwrap_or_default()
                    };
                    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
                };
                if let Some(rest) = v.strip_prefix("https://") {
                    if rest.is_empty() { Err("must name a host".into()) } else { Ok(()) }
                } else if v.strip_prefix("http://").is_some_and(local) {
                    Ok(())
                } else {
                    Err("must be an https:// URL (http only for localhost)".into())
                }
            }
        }
    }

    /// [`check`](Self::check) for each item of a list setting, with at most [`MAX_ITEMS`] items,
    /// and none twice in a `set`. A [`Pairs`](Self::Pairs) list never names a name twice.
    ///
    /// # Errors
    /// An item does not fit, there are too many, or one repeats.
    pub fn check_list(self, items: &[String], set: bool) -> Result<(), String> {
        if self == Self::SolanaAddress && items.is_empty() {
            return Err("must name at least one address".into());
        }
        if items.len() > MAX_ITEMS {
            return Err(format!("has too many items (at most {MAX_ITEMS})"));
        }
        let mut seen = std::collections::HashSet::new();
        for item in items {
            self.check(item)?;
            let name = if self == Self::Pairs {
                item.split_once('=').map_or("", |(k, _)| k).trim()
            } else if set {
                item.trim()
            } else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                return Err("names an item twice".into());
            }
        }
        Ok(())
    }
}

fn host(v: &str) -> Result<(), String> {
    let (name, port) = v.rsplit_once(':').map_or((v, None), |(n, p)| (n, Some(p)));
    let ok = !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && port.is_none_or(|p| p.parse::<u16>().is_ok());
    if ok {
        Ok(())
    } else {
        Err("must be a host name such as bot.example.com (no scheme, no path)".into())
    }
}

fn header(v: &str) -> Result<(), String> {
    if !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        Ok(())
    } else {
        Err("must be a header name (letters, digits and -)".into())
    }
}

fn log_filter(v: &str) -> Result<(), String> {
    const LEVELS: [&str; 6] = ["trace", "debug", "info", "warn", "error", "off"];
    let target = |t: &str| t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b':');
    let each = v.split(',').all(|d| {
        let d = d.trim();
        let level = match d.rsplit_once('=') {
            Some((t, l)) if target(t.trim()) => l.trim(),
            Some(_) => "",
            None => d,
        };
        LEVELS.contains(&level.to_ascii_lowercase().as_str())
    });
    if each {
        Ok(())
    } else {
        Err("must be a level (trace, debug, info, warn, error, off), or target=level, comma \
             separated"
            .into())
    }
}

/// The longest value or list item.
const MAX_LEN: usize = 512;
/// The most items a list setting takes.
pub const MAX_ITEMS: usize = 64;

/// Refuse what looks like a private key: 64 hex digits (with or without `0x`), a base58 secret of
/// 87-88 characters, or a JSON byte array of 32 or more bytes (a Solana CLI keypair file's
/// content). A key is never a config value, whatever the setting is for.
fn no_key(v: &str) -> Result<(), String> {
    const B58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let bare = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")).unwrap_or(v);
    let hex = bare.len() == 64 && bare.bytes().all(|b| b.is_ascii_hexdigit());
    let b58 = (87..=88).contains(&v.len()) && v.chars().all(|c| B58.contains(c));
    let bytes = v.strip_prefix('[').and_then(|r| r.strip_suffix(']')).is_some_and(|inner| {
        let items: Vec<&str> = inner.split(',').map(str::trim).collect();
        items.len() >= 32 && items.iter().all(|i| i.parse::<u8>().is_ok())
    });
    if hex || b58 || bytes {
        Err("looks like a private key: a key is never set here (the bot makes its own)".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "text_format_tests.rs"]
mod checks;
