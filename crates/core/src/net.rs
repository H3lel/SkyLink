//! Policy routing around the TUN device (see the crate docs for the design).

use nthlink_common::tunnel::{FWMARK, ROUTE_TABLE, RULE_PREF_MARK, RULE_PREF_SUPPRESS, RULE_PREF_TABLE, TUN_NAME};
use std::process::{Command, Stdio};

const SRC_VALID_MARK: &str = "/proc/sys/net/ipv4/conf/all/src_valid_mark";

fn ip(args: &[&str]) -> Result<String, String> {
    let out = Command::new("ip")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ip: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("ip {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

pub fn tun_exists() -> bool {
    std::path::Path::new("/sys/class/net").join(TUN_NAME).exists()
}

fn rules(fam: &str) -> Result<(), String> {
    let mark = format!("{FWMARK:#x}");
    let table = ROUTE_TABLE.to_string();
    ip(&[fam, "rule", "add", "pref", &RULE_PREF_MARK.to_string(), "fwmark", &mark, "lookup", "main"])?;
    ip(&[fam, "rule", "add", "pref", &RULE_PREF_SUPPRESS.to_string(), "lookup", "main", "suppress_prefixlength", "0"])?;
    ip(&[fam, "rule", "add", "pref", &RULE_PREF_TABLE.to_string(), "lookup", &table])?;
    Ok(())
}

fn has_rules(fam: &str) -> bool {
    ip(&[fam, "rule", "show", "pref", &RULE_PREF_TABLE.to_string()]).map(|s| !s.trim().is_empty()).unwrap_or(false)
}

fn has_route(fam: &str) -> bool {
    ip(&[fam, "route", "show", "table", &ROUTE_TABLE.to_string()]).map(|s| s.contains(TUN_NAME)).unwrap_or(false)
}

/// Route everything into the TUN. Returns whether IPv6 is covered too.
pub fn setup() -> Result<bool, String> {
    let table = ROUTE_TABLE.to_string();
    ip(&["-4", "route", "replace", "default", "dev", TUN_NAME, "table", &table])?;
    rules("-4")?;

    // Marked replies must pass reverse-path filtering (as wg-quick does).
    if let Ok(old) = std::fs::read_to_string(SRC_VALID_MARK) {
        let state = std::path::Path::new(crate::STATE_DIR).join("src_valid_mark");
        if !state.exists() {
            let _ = std::fs::write(&state, old.trim());
        }
        let _ = std::fs::write(SRC_VALID_MARK, "1");
    }

    // IPv6: send it into the TUN as well (leaf drops what it can't proxy), so
    // nothing leaks around the tunnel on dual-stack networks.
    let v6_sysctl = format!("/proc/sys/net/ipv6/conf/{TUN_NAME}/disable_ipv6");
    let ipv6 = std::path::Path::new(&v6_sysctl).exists()
        && std::fs::write(&v6_sysctl, "0").is_ok()
        && ip(&["-6", "route", "replace", "default", "dev", TUN_NAME, "table", &table]).is_ok()
        && rules("-6").is_ok();
    if !ipv6 {
        del_rules("-6");
    }
    Ok(ipv6)
}

/// Re-add anything a network manager removed while connected.
pub fn ensure() -> Result<(), String> {
    let table = ROUTE_TABLE.to_string();
    if !has_route("-4") {
        ip(&["-4", "route", "replace", "default", "dev", TUN_NAME, "table", &table])?;
    }
    if !has_rules("-4") {
        del_rules("-4");
        rules("-4")?;
    }
    Ok(())
}

fn del_rules(fam: &str) {
    for pref in [RULE_PREF_MARK, RULE_PREF_SUPPRESS, RULE_PREF_TABLE] {
        // A pref can hold several rules if something went wrong earlier.
        for _ in 0..16 {
            if ip(&[fam, "rule", "del", "pref", &pref.to_string()]).is_err() {
                break;
            }
        }
    }
}

/// Remove everything `setup` (or a crashed earlier run) left behind.
/// Idempotent and safe to call when nothing is set up.
pub fn cleanup() {
    for fam in ["-4", "-6"] {
        del_rules(fam);
        let _ = ip(&[fam, "route", "flush", "table", &ROUTE_TABLE.to_string()]);
    }
    if tun_exists() {
        let _ = ip(&["link", "del", TUN_NAME]);
    }
    let state = std::path::Path::new(crate::STATE_DIR).join("src_valid_mark");
    if let Ok(old) = std::fs::read_to_string(&state) {
        let _ = std::fs::write(SRC_VALID_MARK, old.trim());
        let _ = std::fs::remove_file(&state);
    }
}
