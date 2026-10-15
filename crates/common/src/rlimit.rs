//! Open-file limit.
//!
//! Desktop sessions start apps with a soft `RLIMIT_NOFILE` of 1024. leaf needs
//! a socket (often two) per proxied connection, so a busy browser behind the
//! TUN exhausts that quickly and every new connection fails with "Too many
//! open files". Raise the soft limit, but only to a sane ceiling: a leak or
//! routing loop should fail fast rather than pile up a million sockets.

const WANT: libc::rlim_t = 65536;

/// Raise the soft open-file limit to `WANT` (bounded by the hard limit; as
/// root the hard limit is raised to `WANT` if lower). Returns the soft limit.
pub fn raise_nofile() -> Option<u64> {
    let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } != 0 {
        return None;
    }
    if lim.rlim_cur >= WANT {
        return Some(lim.rlim_cur as u64);
    }
    let max = if unsafe { libc::geteuid() } == 0 { lim.rlim_max.max(WANT) } else { lim.rlim_max };
    let want = libc::rlimit { rlim_cur: WANT.min(max), rlim_max: max };
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &want) } == 0 {
        lim = want;
    }
    Some(lim.rlim_cur as u64)
}
