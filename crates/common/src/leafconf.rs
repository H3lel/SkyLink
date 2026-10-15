//! Builds the leaf configuration, ported from
//! `nthLink.Plugins.Leaf.Kernel.ConfigGenerator` in the Windows client.
//!
//! The only intentional difference is how traffic is captured: Windows injects
//! `nf = <driver>, nfapi.dll` (the NetFilter kernel driver), Linux gets a
//! `tun = ...` line instead and the privileged core routes traffic into it.

use crate::model::{DirectoryServerConfig, Server};
use crate::tunnel;
use rand::Rng;

/// Keys in `[General]` that select how leaf captures traffic or where it
/// listens. They are dropped from server-provided configs because the Linux
/// core owns that decision.
const CAPTURE_KEYS: &[&str] = &["tun", "tun-fd", "tun2socks-backend", "nf"];

/// Outbound that swallows traffic addressed to the TUN itself.
const TUN_GUARD: &str = "TunGuard";

/// DNS queries fake DNS can't answer (PTR, SOA, ... from systemd-resolved)
/// fall through to the rules. The usual `IP-CIDR, 172.16.0.0/12, Direct`
/// covers the TUN's DNS address, so leaf would send them straight back into
/// the TUN, endlessly, opening a socket each time. Reject them instead.
fn tun_guard_rule() -> String {
    format!("IP-CIDR, {}, {TUN_GUARD}", tunnel::TUN_CIDR)
}

#[derive(Debug, Clone)]
pub struct LeafOptions {
    /// leaf log level (`trace`, `debug`, `info`, `warn`, `error`).
    pub log_level: String,
    /// Optional log file; `None` lets leaf write to stdout.
    pub log_file: Option<String>,
}

impl Default for LeafOptions {
    fn default() -> Self {
        Self { log_level: "warn".into(), log_file: None }
    }
}

#[derive(Debug, Clone)]
pub struct BuiltConfig {
    pub text: String,
    /// Human readable description of what was selected, for the UI.
    pub summary: String,
}

/// Mirrors `VpnService.RunProxy`: a server-provided custom config wins,
/// otherwise one server is picked at random.
pub fn build(cfg: &DirectoryServerConfig, opts: &LeafOptions) -> Result<BuiltConfig, String> {
    if cfg.use_custom_config && !cfg.custom_config.trim().is_empty() {
        return Ok(BuiltConfig {
            text: modify_custom_config(&cfg.custom_config, opts),
            summary: "server-provided configuration".into(),
        });
    }
    if cfg.servers.is_empty() {
        return Err("The directory service returned no servers".into());
    }
    // Random pick like the Windows client, but skip protocols leaf can't use
    // instead of failing outright.
    let mut candidates: Vec<&Server> = cfg.servers.iter().filter(|s| proxy_line(0, s, "").is_some()).collect();
    if candidates.is_empty() {
        let protos: Vec<_> = cfg.servers.iter().map(|s| s.protocol.as_str()).collect();
        return Err(format!("The protocol does not exist, {}", protos.join(", ")));
    }
    let idx = rand::thread_rng().gen_range(0..candidates.len());
    let server = candidates.swap_remove(idx);
    Ok(BuiltConfig {
        text: generate_client_config(&[server], opts),
        summary: format!("{} server", server.protocol),
    })
}

fn tun_line() -> String {
    format!(
        "tun = {}, {}, {}, {}, {}",
        tunnel::TUN_NAME,
        tunnel::TUN_ADDR,
        tunnel::TUN_NETMASK,
        tunnel::TUN_PEER,
        tunnel::TUN_MTU
    )
}

fn key_of(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with('#') || t.starts_with(';') {
        return None;
    }
    t.split_once('=').map(|(k, _)| k.trim().to_ascii_lowercase())
}

/// `ModifyLeafConfig`: keep the server's sections in order, make sure fake-IP
/// DNS is on, and swap the capture method.
pub fn modify_custom_config(text: &str, opts: &LeafOptions) -> String {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            if !sections.iter().any(|(name, _)| name.eq_ignore_ascii_case(t)) {
                sections.push((t.to_string(), Vec::new()));
            }
            continue;
        }
        if let Some((_, lines)) = sections.last_mut() {
            lines.push(line.to_string());
        }
    }

    let general_idx = match sections.iter().position(|(n, _)| n.eq_ignore_ascii_case("[General]")) {
        Some(i) => i,
        None => {
            sections.insert(0, ("[General]".into(), Vec::new()));
            0
        }
    };
    let general = &mut sections[general_idx].1;
    general.retain(|l| match key_of(l) {
        Some(k) => !CAPTURE_KEYS.contains(&k.as_str()) && k != "loglevel" && k != "logoutput",
        None => true,
    });
    if !general.iter().any(|l| key_of(l).as_deref() == Some("always-fake-ip")) {
        general.push("always-fake-ip = *".into());
    }
    general.push(tun_line());
    general.push(format!("loglevel = {}", opts.log_level));
    if let Some(f) = &opts.log_file {
        general.push(format!("logoutput = {f}"));
    }
    section_mut(&mut sections, "[Proxy]").push(format!("{TUN_GUARD} = reject"));
    section_mut(&mut sections, "[Rule]").insert(0, tun_guard_rule());

    let mut out = String::new();
    for (name, lines) in &sections {
        out.push_str(name);
        out.push('\n');
        for l in lines {
            out.push_str(l);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

fn section_mut<'a>(sections: &'a mut Vec<(String, Vec<String>)>, name: &str) -> &'a mut Vec<String> {
    let i = match sections.iter().position(|(n, _)| n.eq_ignore_ascii_case(name)) {
        Some(i) => i,
        None => {
            sections.push((name.to_string(), Vec::new()));
            sections.len() - 1
        }
    };
    &mut sections[i].1
}

/// `GenerateClientConfigAsync`, with LAN traffic kept direct and a proper
/// `FINAL` rule.
pub fn generate_client_config(servers: &[&Server], opts: &LeafOptions) -> String {
    let mut s = String::new();
    s.push_str("[General]\n");
    s.push_str(&format!("loglevel = {}\n", opts.log_level));
    if let Some(f) = &opts.log_file {
        s.push_str(&format!("logoutput = {f}\n"));
    }
    s.push_str("dns-server = 8.8.8.8, 1.1.1.1\n");
    s.push_str("always-fake-ip = *\n");
    s.push_str(&tun_line());
    s.push_str("\n\n[Env]\nHTTP_USER_AGENT = nthLink\n\n");

    let mut hosts = String::new();
    let mut proxies = String::new();
    let mut names = Vec::new();
    for (i, server) in servers.iter().enumerate() {
        let mut host_alias = String::new();
        if !server.ips.is_empty() {
            host_alias = format!("host_{i}");
            hosts.push_str(&format!("{host_alias} = {}\n", server.ips.join(", ")));
        }
        if let Some(line) = proxy_line(i, server, &host_alias) {
            proxies.push_str(&line);
            proxies.push('\n');
            names.push(format!("proxy_{i}"));
        }
    }
    if !hosts.is_empty() {
        s.push_str("[Host]\n");
        s.push_str(&hosts);
        s.push('\n');
    }
    s.push_str(&format!("[Proxy]\nDirect = direct\n{TUN_GUARD} = reject\n"));
    s.push_str(&proxies);
    s.push('\n');
    s.push_str("[Proxy Group]\n");
    s.push_str(&format!("Random = random, {}\n\n", names.join(", ")));
    s.push_str("[Rule]\n");
    s.push_str(&tun_guard_rule());
    s.push('\n');
    for cidr in ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "169.254.0.0/16", "224.0.0.0/4", "255.255.255.255/32"] {
        s.push_str(&format!("IP-CIDR, {cidr}, Direct\n"));
    }
    s.push_str("DOMAIN-SUFFIX, local, Direct\n");
    s.push_str("FINAL, Random\n");
    s
}

/// `GetProxyString`. Returns `None` for protocols the Windows client doesn't
/// handle either.
fn proxy_line(index: usize, server: &Server, ips_host: &str) -> Option<String> {
    let host = if ips_host.is_empty() { server.host.as_str() } else { ips_host };
    match server.protocol.to_ascii_lowercase().as_str() {
        "ss" | "outline" => Some(format!(
            "proxy_{index} = ss, {host}, {}, encrypt-method={}, password={}",
            server.port, server.encrypt_method, server.password
        )),
        "trojan" => {
            let mut l = format!(
                "proxy_{index} = trojan, {host}, {}, password={}, sni={}",
                server.port, server.password, server.sni
            );
            if server.ws {
                l.push_str(&format!(", ws=true, ws-path={}", server.ws_path));
            }
            let ws_host = if server.ws_host.is_empty() { &server.sni } else { &server.ws_host };
            l.push_str(&format!(", ws-host={ws_host}"));
            Some(l)
        }
        _ => None,
    }
}

/// Turn a generated (TUN) config into a local-proxy config: no TUN, no fake
/// IPs, SOCKS5 + HTTP listeners on loopback. Needs no root.
pub fn to_proxy_mode(text: &str, socks_port: u16, http_port: u16) -> String {
    let mut out = String::new();
    let mut in_general = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_general = t.eq_ignore_ascii_case("[General]");
            out.push_str(line);
            out.push('\n');
            if in_general {
                out.push_str(&format!("socks-interface = 127.0.0.1\nsocks-port = {socks_port}\n"));
                out.push_str(&format!("http-interface = 127.0.0.1\nhttp-port = {http_port}\n"));
            }
            continue;
        }
        if in_general {
            if let Some(k) = key_of(line) {
                let drop = CAPTURE_KEYS.contains(&k.as_str())
                    || matches!(k.as_str(), "always-fake-ip" | "socks-interface" | "socks-port" | "http-interface" | "http-port" | "interface" | "port");
                if drop {
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The outbound the `FINAL` rule sends traffic to (what health checks should
/// probe), falling back to the first proxy group, then the first proxy.
pub fn final_outbound(text: &str) -> Option<String> {
    let mut section = String::new();
    let mut first_group = None;
    let mut first_proxy = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.starts_with(';') || t.is_empty() {
            continue;
        }
        if t.starts_with('[') && t.ends_with(']') {
            section = t.to_ascii_lowercase();
            continue;
        }
        match section.as_str() {
            "[rule]" => {
                let parts: Vec<&str> = t.split(',').map(str::trim).collect();
                if parts.len() == 2 && parts[0].eq_ignore_ascii_case("FINAL") {
                    return Some(parts[1].to_string());
                }
            }
            "[proxy group]" if first_group.is_none() => first_group = key_of(t).map(|_| t.split('=').next().unwrap().trim().to_string()),
            "[proxy]" if first_proxy.is_none() => {
                if let Some((k, v)) = t.split_once('=') {
                    let kind = v.trim().split(',').next().unwrap_or("").trim();
                    if kind != "direct" && kind != "reject" && kind != "drop" {
                        first_proxy = Some(k.trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    first_group.or(first_proxy)
}

/// `ReadDnsAsync`: the DNS servers a config asks for.
pub fn read_dns(config_text: &str) -> Vec<String> {
    for line in config_text.lines() {
        if key_of(line).as_deref() == Some("dns-server") {
            if let Some((_, v)) = line.split_once('=') {
                let list: Vec<String> = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                if !list.is_empty() {
                    return list;
                }
            }
        }
    }
    vec!["8.8.8.8".into(), "1.1.1.1".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_config_swaps_capture() {
        let src = "[General]\nloglevel = error\nnf = netfilter2, C:\\nfapi.dll\ndns-server = 1.1.1.1\n\n[Proxy]\nDirect = direct\n\n[Rule]\nFINAL, Direct\n";
        let out = modify_custom_config(src, &LeafOptions::default());
        assert!(!out.contains("nf ="));
        assert!(out.contains("tun = nthlink0"));
        assert!(out.contains("always-fake-ip = *"));
        assert!(out.contains("loglevel = warn"));
        assert!(!out.contains("loglevel = error"));
        assert!(out.contains("[Rule]\nIP-CIDR, 172.19.0.0/30, TunGuard\nFINAL, Direct"));
        assert!(out.contains("TunGuard = reject"));
    }

    #[test]
    fn tun_subnet_is_never_direct() {
        let src = "[Proxy]\nDirect = direct\na = trojan, x, 443\n[Rule]\nIP-CIDR, 172.16.0.0/12, Direct\nFINAL, a\n";
        let out = modify_custom_config(src, &LeafOptions::default());
        let rules: Vec<&str> = out.split("[Rule]\n").nth(1).unwrap().lines().collect();
        assert_eq!(rules[0], "IP-CIDR, 172.19.0.0/30, TunGuard");
        assert_eq!(final_outbound(&out).as_deref(), Some("a"));
        let gen = generate_client_config(&[], &LeafOptions::default());
        assert!(gen.contains("[Rule]\nIP-CIDR, 172.19.0.0/30, TunGuard\n"));
    }

    #[test]
    fn final_outbound_found() {
        assert_eq!(final_outbound("[Proxy]\nDirect = direct\na = trojan, x, 443\n[Proxy Group]\nProxy = failover, a\n[Rule]\nIP-CIDR, 10.0.0.0/8, Direct\nFINAL, Proxy\n").as_deref(), Some("Proxy"));
        assert_eq!(final_outbound("[Proxy]\nDirect = direct\na = trojan, x, 443\n[Proxy Group]\nG = random, a\n").as_deref(), Some("G"));
        assert_eq!(final_outbound("[Proxy]\nDirect = direct\na = trojan, x, 443\n").as_deref(), Some("a"));
    }

    #[test]
    fn trojan_line_matches_windows_format() {
        let s = Server {
            protocol: "trojan".into(),
            host: "h.example".into(),
            port: "443".into(),
            password: "pw".into(),
            sni: "h.example".into(),
            ws: true,
            ws_path: "/p".into(),
            ..Default::default()
        };
        assert_eq!(
            proxy_line(0, &s, "").unwrap(),
            "proxy_0 = trojan, h.example, 443, password=pw, sni=h.example, ws=true, ws-path=/p, ws-host=h.example"
        );
    }
}
