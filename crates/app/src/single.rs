//! Single-instance guard. The first DownCraft process owns a loopback port; a second launch
//! connects to it, asks the running copy to show its window, and exits - so the tray never ends
//! up with several DownCraft icons. A crashed process releases the port automatically.
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    time::Duration,
};

const PORT: u16 = 19871;
const HELLO: &[u8] = b"DOWNCRAFT-SHOW\n";
const REPLY: &[u8] = b"OK\n";

pub enum Guard {
    /// We are the only instance; keep the listener alive for the whole run.
    First(TcpListener),
    /// Another DownCraft was found and has been told to show itself - quit.
    Already,
}

pub fn acquire() -> Guard {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT));
    match TcpListener::bind(addr) {
        Ok(l) => Guard::First(l),
        Err(_) => {
            // port taken: is it really DownCraft? (otherwise just start normally)
            if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(800)) {
                let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
                let mut buf = [0u8; 3];
                if s.write_all(HELLO).is_ok() && s.read_exact(&mut buf).is_ok() && buf == REPLY[..] {
                    return Guard::Already;
                }
            }
            // could not bind and nobody answered: run without the guard
            match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
                Ok(l) => Guard::First(l),
                Err(_) => Guard::Already,
            }
        }
    }
}

/// Answers later launches and brings the main window back each time.
pub fn listen(listener: TcpListener, ctx: eframe::egui::Context) {
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let mut s = s;
            let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
            let mut buf = [0u8; 32];
            if let Ok(n) = s.read(&mut buf) {
                if buf[..n].starts_with(b"DOWNCRAFT-SHOW") {
                    let _ = s.write_all(REPLY);
                    crate::tray::reveal(&ctx);
                }
            }
        }
    });
}
