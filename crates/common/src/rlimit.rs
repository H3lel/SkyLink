//! Open-file limit.
//!
//! Desktop sessions start apps with a soft `RLIMIT_NOFILE` of 1024. leaf needs
//! a socket (often two) per proxied connection, so a busy browser behind the
//! TUN exhausts that quickly and every new connection fails with "Too many
//! open files". Raise the soft limit as far as we are allowed.

/// Raise the soft open-file limit to the hard limit. As root the hard limit
/// is raised too (up to `fs.nr_open`). Returns the resulting soft limit.
pub fn raise_nofile() -> Option<u64> {
    const WANT: libc::rlim_t = 1 << 20;
    let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } != 0 {
        return None;
    }
    if unsafe { libc::geteuid() } == 0 && lim.rlim_max != libc::RLIM_INFINITY && lim.rlim_max < WANT {
        let nr_open = std::fs::read_to_string("/proc/sys/fs/nr_open")
            .ok()
            .and_then(|s| s.trim().parse::<libc::rlim_t>().ok())
            .unwrap_or(WANT);
        let raised = libc::rlimit { rlim_cur: WANT.min(nr_open), rlim_max: WANT.min(nr_open) };
        if raised.rlim_max > lim.rlim_max && unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } == 0 {
            lim = raised;
        }
    }
    if lim.rlim_cur < lim.rlim_max {
        let want = libc::rlimit { rlim_cur: lim.rlim_max, rlim_max: lim.rlim_max };
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &want) } == 0 {
            lim.rlim_cur = lim.rlim_max;
        }
    }
    Some(lim.rlim_cur as u64)
}
