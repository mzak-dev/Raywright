use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// Where a bridge listens, written as a URI:
///
/// | URI | Transport |
/// |---|---|
/// | `pipe://raywright-1234` | Windows named pipe `\\.\pipe\raywright-1234` |
/// | `unix:///tmp/raywright-1234.sock` | Unix domain socket |
/// | `tcp://127.0.0.1:4567` | TCP, loopback only |
/// | `memory://name` | In-process, for tests |
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BridgeAddr {
    Pipe(String),
    Unix(PathBuf),
    Tcp(String),
    Memory(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddrError(pub String);

impl fmt::Display for AddrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad bridge address: {}", self.0)
    }
}

impl std::error::Error for AddrError {}

impl BridgeAddr {
    pub fn scheme(&self) -> &'static str {
        match self {
            BridgeAddr::Pipe(_) => "pipe",
            BridgeAddr::Unix(_) => "unix",
            BridgeAddr::Tcp(_) => "tcp",
            BridgeAddr::Memory(_) => "memory",
        }
    }

    /// The full Windows pipe path for a `pipe://` address.
    pub fn pipe_path(&self) -> Option<String> {
        match self {
            BridgeAddr::Pipe(name) => Some(format!(r"\\.\pipe\{name}")),
            _ => None,
        }
    }

    /// The platform's usual address for a bridge named `name`: a named pipe on Windows,
    /// loopback TCP on an OS-picked port elsewhere (a Unix socket transport may replace it).
    pub fn platform_default(name: &str) -> BridgeAddr {
        if cfg!(windows) { BridgeAddr::Pipe(name.to_string()) } else { BridgeAddr::Tcp("127.0.0.1:0".into()) }
    }
}

impl fmt::Display for BridgeAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeAddr::Pipe(n) => write!(f, "pipe://{n}"),
            BridgeAddr::Unix(p) => write!(f, "unix://{}", p.display()),
            BridgeAddr::Tcp(a) => write!(f, "tcp://{a}"),
            BridgeAddr::Memory(n) => write!(f, "memory://{n}"),
        }
    }
}

impl FromStr for BridgeAddr {
    type Err = AddrError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (scheme, rest) = s.split_once("://").ok_or_else(|| AddrError(format!("`{s}` has no scheme (pipe://, unix://, tcp://)")))?;
        if rest.is_empty() {
            return Err(AddrError(format!("`{s}` has nothing after the scheme")));
        }
        match scheme {
            "pipe" => {
                let name = rest.strip_prefix(r"\\.\pipe\").unwrap_or(rest);
                if name.contains(['\\', '/']) {
                    return Err(AddrError(format!("pipe name `{name}` may not contain slashes")));
                }
                Ok(BridgeAddr::Pipe(name.to_string()))
            }
            "unix" => Ok(BridgeAddr::Unix(PathBuf::from(rest))),
            "tcp" => {
                let host = rest.rsplit_once(':').map(|(h, _)| h).ok_or_else(|| AddrError(format!("`{rest}` needs host:port")))?;
                if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
                    return Err(AddrError(format!("tcp bridges listen on loopback only, not `{host}`")));
                }
                Ok(BridgeAddr::Tcp(rest.to_string()))
            }
            "memory" => Ok(BridgeAddr::Memory(rest.to_string())),
            other => Err(AddrError(format!("unknown scheme `{other}`"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_round_trip() {
        for s in ["pipe://raywright-12", "unix:///tmp/r.sock", "tcp://127.0.0.1:80", "memory://a"] {
            assert_eq!(BridgeAddr::from_str(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn pipe_accepts_the_full_path() {
        let a: BridgeAddr = r"pipe://\\.\pipe\x".parse().unwrap();
        assert_eq!(a.pipe_path().unwrap(), r"\\.\pipe\x");
    }

    #[test]
    fn tcp_is_loopback_only() {
        assert!("tcp://0.0.0.0:1".parse::<BridgeAddr>().is_err());
        assert!("tcp://localhost:1".parse::<BridgeAddr>().is_ok());
        assert!("ftp://x".parse::<BridgeAddr>().is_err());
        assert!("x".parse::<BridgeAddr>().is_err());
    }
}
