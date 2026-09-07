//! `host:port` assembly for the SRT open paths.
//!
//! `SrtUrl` hands back the bare host (IPv6 brackets stripped), while
//! `ToSocketAddrs` needs `[v6]:port` — joining with a plain `{host}:{port}`
//! produced `::1:9000` and every IPv6 `srt://` open failed to resolve. One
//! helper for the caller connect and both listener binds (mirror of the JVM
//! binding's `jutil::join_host_port`).

/// Join `host` and `port` for `ToSocketAddrs`, bracketing an IPv6 literal
/// that is not already bracketed.
pub(crate) fn join_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::join_host_port;

    #[test]
    fn ipv6_literal_is_bracketed() {
        assert_eq!(join_host_port("::1", 9000), "[::1]:9000");
        assert_eq!(join_host_port("fe80::1%eth0", 7000), "[fe80::1%eth0]:7000");
    }

    #[test]
    fn already_bracketed_ipv6_is_left_alone() {
        assert_eq!(join_host_port("[::1]", 9000), "[::1]:9000");
    }

    #[test]
    fn ipv4_and_hostnames_are_plain() {
        assert_eq!(join_host_port("127.0.0.1", 9000), "127.0.0.1:9000");
        assert_eq!(join_host_port("0.0.0.0", 7000), "0.0.0.0:7000");
        assert_eq!(join_host_port("camera.local", 9000), "camera.local:9000");
    }
}
