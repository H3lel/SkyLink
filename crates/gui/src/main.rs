//! nthLink for Linux.
//!
//!   nthlink              GTK4 window (default)
//!   nthlink connect      connect from a terminal; Ctrl+C disconnects
//!   nthlink proxy [S [H]] local SOCKS5 (default 1080) / HTTP (1087) proxy, no root
//!   nthlink fetch        show what the directory service returns
//!   nthlink cleanup      remove routes/DNS left behind by a killed tunnel
//!   nthlink render [F]   print the leaf config generated from a directory
//!                        answer (default: the last cached one)

mod cli;
mod engine;
mod ui;

fn main() {
    let cmd = std::env::args().nth(1);
    let code = match cmd.as_deref() {
        Some("connect") => cli::connect(),
        Some("proxy") => cli::proxy(&std::env::args().skip(2).collect::<Vec<_>>()),
        Some("fetch") => cli::fetch(),
        Some("cleanup") => cli::cleanup(),
        Some("render") => cli::render(std::env::args().nth(2)),
        Some("--version") | Some("-V") => {
            println!("nthlink {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("--help") | Some("-h") | Some("help") => {
            println!("usage: nthlink [connect | proxy [socks-port [http-port]] | fetch | cleanup | --version]\n\nWithout arguments the graphical client starts.");
            0
        }
        Some(other) if !other.starts_with("--") => {
            eprintln!("unknown command: {other} (see --help)");
            64
        }
        _ => {
            let c = ui::run();
            if c == gtk::glib::ExitCode::SUCCESS { 0 } else { 1 }
        }
    };
    std::process::exit(code);
}
