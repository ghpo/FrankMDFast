//! Where the remote server is allowed to listen.
//!
//! The default is the machine's **Tailscale** address. Loopback is allowed for
//! `ssh -L` setups. Anything else (including `0.0.0.0`) requires the explicit
//! `--serve-bind-any` escape hatch, so a mistyped flag cannot expose a notes
//! vault to the LAN.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Requested bind target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindSpec {
    /// The machine's Tailscale address (discovered at startup).
    Tailscale,
    /// `127.0.0.1`.
    Loopback,
    /// `0.0.0.0` (only with `allow_any`).
    Any,
    /// An explicit address (`100.x.y.z`, `127.0.0.1`, ...).
    Explicit(IpAddr),
}

/// Why a bind target was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindError {
    /// Unknown `--serve-bind` value.
    #[error("invalid bind address: {0}")]
    Invalid(String),
    /// Tailscale is not up (or not installed).
    #[error(
        "no Tailscale address found; start Tailscale or pass --serve-bind loopback|127.0.0.1|<tailscale-ip>"
    )]
    NoTailscale,
    /// A non-Tailscale, non-loopback address without `--serve-bind-any`.
    #[error(
        "refusing to listen on {0}: only Tailscale or loopback addresses are allowed (pass --serve-bind-any to override)"
    )]
    NotPrivate(IpAddr),
}

/// Parse a `--serve-bind` value.
pub fn parse_bind(spec: &str) -> Result<BindSpec, BindError> {
    let spec = spec.trim();
    if spec.eq_ignore_ascii_case("tailscale") {
        return Ok(BindSpec::Tailscale);
    }
    if spec.eq_ignore_ascii_case("loopback") || spec.eq_ignore_ascii_case("localhost") {
        return Ok(BindSpec::Loopback);
    }
    if spec.eq_ignore_ascii_case("any") || spec == "0.0.0.0" || spec == "::" {
        return Ok(BindSpec::Any);
    }
    spec.parse::<IpAddr>()
        .map(BindSpec::Explicit)
        .map_err(|_| BindError::Invalid(spec.to_owned()))
}

/// Turn a spec into the concrete address to listen on.
pub fn resolve_bind(spec: BindSpec, allow_any: bool) -> Result<IpAddr, BindError> {
    match spec {
        BindSpec::Tailscale => tailscale_ipv4().ok_or(BindError::NoTailscale),
        BindSpec::Loopback => Ok(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        BindSpec::Any => {
            if allow_any {
                Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
            } else {
                Err(BindError::NotPrivate(IpAddr::V4(Ipv4Addr::UNSPECIFIED)))
            }
        }
        BindSpec::Explicit(addr) => {
            if addr.is_loopback() || is_tailscale_addr(addr) || allow_any {
                Ok(addr)
            } else {
                Err(BindError::NotPrivate(addr))
            }
        }
    }
}

/// `true` for Tailscale's CGNAT IPv4 range and its IPv6 ULA prefix.
pub fn is_tailscale_addr(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            a == 100 && (64..=127).contains(&b)
        }
        IpAddr::V6(v6) => {
            let o = v6.octets();
            o[..6] == [0xfd, 0x7a, 0x11, 0x5c, 0xa1, 0xe0]
        }
    }
}

/// Discover the Tailscale IPv4 address.
///
/// Order: the `tailscale` CLI (`tailscale ip -4`), then the Linux `ip` command
/// for `tailscale0`, then any `100.64.0.0/10` address from `ip addr`. Returns
/// `None` when Tailscale is down or no tool is available.
pub fn tailscale_ipv4() -> Option<IpAddr> {
    if let Some(ip) = command_stdout("tailscale", &["ip", "-4"]) {
        for line in ip.lines() {
            if let Ok(addr) = line.trim().parse::<IpAddr>()
                && is_tailscale_addr(addr)
            {
                return Some(addr);
            }
        }
    }
    if cfg!(target_os = "linux") {
        if let Some(ip) = command_stdout("ip", &["-4", "-o", "addr", "show", "dev", "tailscale0"])
            && let Some(addr) = first_inet(&ip)
        {
            return Some(addr);
        }
        if let Some(ip) = command_stdout("ip", &["-4", "-o", "addr", "show"]) {
            for line in ip.lines() {
                if let Some(addr) = first_inet(line)
                    && is_tailscale_addr(addr)
                {
                    return Some(addr);
                }
            }
        }
    }
    None
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// `inet 100.64.3.2/32 ...` → `100.64.3.2`.
fn first_inet(line: &str) -> Option<IpAddr> {
    let mut words = line.split_whitespace();
    while let Some(word) = words.next() {
        if word == "inet" {
            let addr = words.next()?.split('/').next()?;
            return addr.parse().ok();
        }
    }
    None
}

/// A short human description for logs (`100.64.0.1:8787`).
pub fn describe(addr: IpAddr, port: u16) -> String {
    match addr {
        IpAddr::V4(v4) => format!("{v4}:{port}"),
        IpAddr::V6(v6) => format!("[{v6}]:{port}"),
    }
}

/// `tailscale0`-less environments (containers, macOS): keep the helper public
/// but mark it as a best-effort probe.
pub fn tailscale_ipv6() -> Option<Ipv6Addr> {
    let out = command_stdout("tailscale", &["ip", "-6"])?;
    out.lines()
        .filter_map(|line| line.trim().parse::<Ipv6Addr>().ok())
        .find(|addr| is_tailscale_addr(IpAddr::V6(*addr)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_cgnat_and_tailscale_ula_count() {
        for ok in ["100.64.0.1", "100.100.100.100", "100.127.255.254"] {
            assert!(is_tailscale_addr(ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "100.63.255.255",
            "100.128.0.0",
            "10.0.0.1",
            "192.168.0.1",
            "::1",
        ] {
            assert!(!is_tailscale_addr(bad.parse().unwrap()), "{bad}");
        }
        assert!(is_tailscale_addr("fd7a:115c:a1e0::1".parse().unwrap()));
        assert!(!is_tailscale_addr("fd7a:115c:a1e1::1".parse().unwrap()));
    }

    #[test]
    fn bind_specs_parse() {
        assert_eq!(parse_bind("tailscale").unwrap(), BindSpec::Tailscale);
        assert_eq!(parse_bind("loopback").unwrap(), BindSpec::Loopback);
        assert_eq!(parse_bind("localhost").unwrap(), BindSpec::Loopback);
        assert_eq!(parse_bind("any").unwrap(), BindSpec::Any);
        assert_eq!(parse_bind("0.0.0.0").unwrap(), BindSpec::Any);
        assert_eq!(
            parse_bind("100.64.1.2").unwrap(),
            BindSpec::Explicit("100.64.1.2".parse().unwrap())
        );
        assert!(parse_bind("nope").is_err());
    }

    #[test]
    fn wildcard_and_public_addresses_need_the_escape_hatch() {
        assert!(resolve_bind(BindSpec::Any, false).is_err());
        assert_eq!(
            resolve_bind(BindSpec::Any, true).unwrap(),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
        let lan: IpAddr = "192.168.1.10".parse().unwrap();
        assert!(resolve_bind(BindSpec::Explicit(lan), false).is_err());
        assert_eq!(resolve_bind(BindSpec::Explicit(lan), true).unwrap(), lan);
        let ts: IpAddr = "100.101.102.103".parse().unwrap();
        assert_eq!(resolve_bind(BindSpec::Explicit(ts), false).unwrap(), ts);
        assert_eq!(
            resolve_bind(BindSpec::Loopback, false).unwrap(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
    }

    #[test]
    fn first_inet_reads_ip_output() {
        assert_eq!(
            first_inet("3: tailscale0    inet 100.64.3.2/32 scope global tailscale0"),
            Some("100.64.3.2".parse().unwrap())
        );
        assert_eq!(first_inet("no address here"), None);
    }
}
