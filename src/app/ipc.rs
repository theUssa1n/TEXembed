use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

pub const IPC_PORT: u16 = 47823;

pub fn start_ipc_listener(
    listener: TcpListener,
    tx: std::sync::mpsc::Sender<String>,
    ctx: &egui::Context,
) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buffer = String::new();
            if stream.read_to_string(&mut buffer).is_ok() && !buffer.trim().is_empty() {
                let _ = tx.send(buffer.trim().to_string());
                ctx.request_repaint();
            }
        }
    });
}

pub fn send_path_to_running_instance(path: &std::path::Path) {
    if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", IPC_PORT)) {
        if let Some(path) = path.to_str() {
            let _ = stream.write_all(path.as_bytes());
        }
    }
}
