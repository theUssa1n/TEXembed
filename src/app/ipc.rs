use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const IPC_PORT: u16 = 47823;

const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const ACCEPT_INTERVAL: Duration = Duration::from_millis(25);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
const CLIENT_HELLO: &[u8; 17] = b"TEXembed/1 HELLO\n";
const SERVER_HELLO: &[u8; 17] = b"TEXembed/1 READY\n";
const ACKNOWLEDGED: &[u8; 17] = b"TEXembed/1 ACKED\n";
const REJECTED: &[u8; 17] = b"TEXembed/1 ERROR\n";

#[must_use]
pub struct IpcListener {
    stopping: Arc<AtomicBool>,
    context: Arc<Mutex<Option<egui::Context>>>,
    worker: Option<JoinHandle<()>>,
}

impl IpcListener {
    pub fn set_context(&self, ctx: &egui::Context) {
        *self
            .context
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(ctx.clone());
        ctx.request_repaint();
    }
}

impl Drop for IpcListener {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        let Some(worker) = self.worker.take() else {
            return;
        };
        worker.thread().unpark();
        let start = Instant::now();
        while !worker.is_finished() && start.elapsed() < SHUTDOWN_TIMEOUT {
            thread::sleep(Duration::from_millis(1));
        }
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
}

pub fn start_ipc_listener(
    listener: TcpListener,
    tx: Sender<String>,
) -> Result<IpcListener, String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Could not configure the TEXembed IPC listener: {error}"))?;
    let stopping = Arc::new(AtomicBool::new(false));
    let context = Arc::new(Mutex::new(None::<egui::Context>));
    let worker_stopping = Arc::clone(&stopping);
    let worker_context = Arc::clone(&context);
    let worker = thread::Builder::new()
        .name("texembed-ipc".to_string())
        .spawn(move || {
            let result = crate::runtime::run_guarded(|| {
                while !worker_stopping.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            if worker_stopping.load(Ordering::Acquire) {
                                break;
                            }
                            let result = crate::runtime::run_guarded(|| {
                                receive_request(stream, &tx, REQUEST_TIMEOUT, || {
                                    let context = worker_context
                                        .lock()
                                        .unwrap_or_else(|error| error.into_inner())
                                        .clone();
                                    if let Some(context) = context {
                                        context.request_repaint();
                                    }
                                })
                            });
                            if let Err(error) = result {
                                if !worker_stopping.load(Ordering::Acquire) {
                                    report_ipc_error(&format!(
                                        "IPC request from {peer} failed: {error}"
                                    ));
                                }
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::park_timeout(ACCEPT_INTERVAL);
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) => {
                            return Err(format!("The TEXembed IPC listener failed: {error}"));
                        }
                    }
                }
                Ok(())
            });
            if let Err(error) = result {
                report_ipc_error(&error);
            }
        })
        .map_err(|error| format!("Could not start the TEXembed IPC listener: {error}"))?;
    Ok(IpcListener {
        stopping,
        context,
        worker: Some(worker),
    })
}

pub fn send_path_to_running_instance(path: Option<&Path>) -> Result<(), String> {
    let message = match path {
        Some(path) => canonical_message(path)?,
        None => String::new(),
    };
    send_request(
        SocketAddr::from((Ipv4Addr::LOCALHOST, IPC_PORT)),
        &message,
        REQUEST_TIMEOUT,
    )
}

fn canonical_message(path: &Path) -> Result<String, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Could not resolve the IPC path {}: {error}", path.display()))?;
    let message = canonical
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "The IPC path cannot be represented as valid Unicode.".to_string())?;
    if message.len() > MAX_PAYLOAD_BYTES {
        return Err("The canonical TEXembed IPC path exceeds 64 KiB.".to_string());
    }
    Ok(message)
}

fn send_request(address: SocketAddr, message: &str, timeout: Duration) -> Result<(), String> {
    if message.len() > MAX_PAYLOAD_BYTES {
        return Err("The TEXembed IPC payload exceeds 64 KiB.".to_string());
    }
    let deadline = request_deadline(timeout)
        .map_err(|error| format!("Invalid TEXembed IPC request timeout: {error}"))?;
    let connect_timeout = remaining_until(deadline)
        .map_err(|error| format!("The TEXembed IPC request expired before connecting: {error}"))?
        .min(CONNECT_TIMEOUT);
    let stream = TcpStream::connect_timeout(&address, connect_timeout)
        .map_err(|error| format!("Could not connect to TEXembed at {address}: {error}"))?;
    let mut connection = Connection::with_deadline(stream, deadline)
        .map_err(|error| format!("Could not configure the TEXembed IPC connection: {error}"))?;
    connection
        .write_all(CLIENT_HELLO)
        .map_err(|error| format!("Could not send the TEXembed handshake: {error}"))?;
    connection.expect_marker(SERVER_HELLO, "server handshake")?;
    write_request(&mut connection, message)?;
    read_acknowledgement(&mut connection)
}

fn write_request(connection: &mut Connection, message: &str) -> Result<(), String> {
    let size = u32::try_from(message.len())
        .map_err(|error| format!("Could not encode the TEXembed IPC payload length: {error}"))?;
    connection
        .write_all(&size.to_be_bytes())
        .and_then(|()| connection.write_all(message.as_bytes()))
        .and_then(|()| connection.stream.shutdown(Shutdown::Write))
        .map_err(|error| format!("Could not send the TEXembed IPC request: {error}"))
}

fn read_acknowledgement(connection: &mut Connection) -> Result<(), String> {
    connection.expect_marker(ACKNOWLEDGED, "acknowledgement")?;
    connection
        .expect_end()
        .map_err(|error| format!("Invalid TEXembed acknowledgement frame: {error}"))
}

fn receive_request(
    stream: TcpStream,
    tx: &Sender<String>,
    timeout: Duration,
    notify: impl FnOnce(),
) -> Result<(), String> {
    let mut connection = Connection::new(stream, timeout)
        .map_err(|error| format!("Could not configure an incoming TEXembed connection: {error}"))?;
    connection.expect_marker(CLIENT_HELLO, "client handshake")?;
    connection
        .write_all(SERVER_HELLO)
        .map_err(|error| format!("Could not reply to the TEXembed handshake: {error}"))?;
    receive_payload(&mut connection, tx, notify)
}

fn receive_payload(
    connection: &mut Connection,
    tx: &Sender<String>,
    notify: impl FnOnce(),
) -> Result<(), String> {
    let result = read_message(connection).and_then(|message| {
        connection.remaining().map_err(|error| {
            format!("The TEXembed IPC request expired before delivery: {error}")
        })?;
        tx.send(message)
            .map_err(|_| "The TEXembed UI is no longer accepting IPC requests.".to_string())
    });
    if let Err(error) = result {
        return match connection.write_all(REJECTED) {
            Ok(()) => Err(error),
            Err(reply_error) => Err(format!(
                "{error}; could not send the rejection acknowledgement: {reply_error}"
            )),
        };
    }
    notify();
    connection
        .write_all(ACKNOWLEDGED)
        .map_err(|error| format!("Could not acknowledge the TEXembed IPC request: {error}"))
}

fn read_message(connection: &mut Connection) -> Result<String, String> {
    let mut header = [0_u8; 4];
    connection
        .read_exact(&mut header)
        .map_err(|error| format!("Could not read the TEXembed IPC frame length: {error}"))?;
    let size = usize::try_from(u32::from_be_bytes(header))
        .map_err(|error| format!("Invalid TEXembed IPC frame length: {error}"))?;
    if size > MAX_PAYLOAD_BYTES {
        return Err("The incoming TEXembed IPC payload exceeds 64 KiB.".to_string());
    }
    let mut payload = vec![0; size];
    connection
        .read_exact(&mut payload)
        .and_then(|()| connection.expect_end())
        .map_err(|error| format!("Invalid TEXembed IPC payload frame: {error}"))?;
    let message = String::from_utf8(payload)
        .map_err(|error| format!("The TEXembed IPC path is not valid UTF-8: {error}"))?;
    if message.is_empty() {
        return Ok(message);
    }
    if message.contains('\0') || !Path::new(&message).is_absolute() {
        return Err("The TEXembed IPC path must be absolute and contain no NUL bytes.".to_string());
    }
    canonical_message(Path::new(&message))
}

struct Connection {
    stream: TcpStream,
    deadline: Instant,
}

impl Connection {
    fn new(stream: TcpStream, timeout: Duration) -> io::Result<Self> {
        Self::with_deadline(stream, request_deadline(timeout)?)
    }

    fn with_deadline(stream: TcpStream, deadline: Instant) -> io::Result<Self> {
        let timeout = remaining_until(deadline)?;
        stream.set_nonblocking(false)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        Ok(Self { stream, deadline })
    }

    fn remaining(&self) -> io::Result<Duration> {
        remaining_until(self.deadline)
    }

    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            self.stream.set_read_timeout(Some(self.remaining()?))?;
            match self.stream.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(timeout_error());
                }
                result => return result,
            }
        }
    }

    fn read_exact(&mut self, mut buffer: &mut [u8]) -> io::Result<()> {
        while !buffer.is_empty() {
            let count = self.read(buffer)?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "The IPC peer closed an incomplete frame",
                ));
            }
            buffer = &mut buffer[count..];
        }
        Ok(())
    }

    fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            self.stream.set_write_timeout(Some(self.remaining()?))?;
            match self.stream.write(bytes) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "The IPC peer stopped accepting data",
                    ));
                }
                Ok(count) => bytes = &bytes[count..],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(timeout_error());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn expect_marker(&mut self, expected: &[u8; 17], phase: &str) -> Result<(), String> {
        let mut received = [0_u8; 17];
        self.read_exact(&mut received)
            .map_err(|error| format!("Could not read the TEXembed {phase}: {error}"))?;
        if &received == expected {
            Ok(())
        } else if &received == REJECTED {
            Err(format!("TEXembed rejected the IPC request during {phase}."))
        } else {
            Err(format!(
                "Invalid TEXembed {phase}: the peer is not compatible with TEXembed IPC version 1."
            ))
        }
    }

    fn expect_end(&mut self) -> io::Result<()> {
        if self.read(&mut [0_u8; 1])? == 0 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unexpected bytes after the IPC frame",
            ))
        }
    }
}

fn request_deadline(timeout: Duration) -> io::Result<Instant> {
    if timeout.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IPC timeout must be positive",
        ));
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "IPC timeout is too large"))
}

fn remaining_until(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(timeout_error)
}

fn timeout_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "The TEXembed IPC request timed out",
    )
}

fn report_ipc_error(message: &str) {
    if let Err(error) = crate::runtime::log_error(message) {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("TEXembed IPC error")
            .set_description(format!("{message}\n\nCould not save crash.log: {error}"))
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc::{self, Receiver};
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_TIMEOUT: Duration = Duration::from_secs(15);
    const SHORT_TIMEOUT: Duration = Duration::from_millis(250);
    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "texembed-ipc-test-{}-{unique}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn file(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"synthetic test file").unwrap();
            path.canonicalize().unwrap()
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct TestWorker<T>(JoinHandle<T>);

    impl<T: Send + 'static> TestWorker<T> {
        fn spawn(operation: impl FnOnce() -> T + Send + 'static) -> Self {
            Self(thread::spawn(operation))
        }

        fn join(self) -> thread::Result<T> {
            let start = Instant::now();
            while !self.0.is_finished() {
                assert!(
                    start.elapsed() < TEST_TIMEOUT,
                    "IPC test worker did not finish"
                );
                thread::sleep(Duration::from_millis(1));
            }
            self.0.join()
        }
    }

    struct TestServer {
        address: SocketAddr,
        messages: Receiver<String>,
        notified: Arc<AtomicBool>,
        worker: TestWorker<Result<(), String>>,
    }

    impl TestServer {
        fn new(timeout: Duration) -> Self {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let (tx, messages) = mpsc::channel();
            let notified = Arc::new(AtomicBool::new(false));
            let worker_notified = Arc::clone(&notified);
            let worker = TestWorker::spawn(move || {
                receive_request(accept_test_stream(&listener), &tx, timeout, || {
                    worker_notified.store(true, Ordering::Release);
                })
            });
            Self {
                address,
                messages,
                notified,
                worker,
            }
        }
    }

    fn accept_test_stream(listener: &TcpListener) -> TcpStream {
        listener.set_nonblocking(true).unwrap();
        let start = Instant::now();
        loop {
            match listener.accept() {
                Ok((stream, _)) => return stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(start.elapsed() < TEST_TIMEOUT, "No test peer connected");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => panic!("Test listener failed: {error}"),
            }
        }
    }

    fn spawn_peer(
        handler: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (SocketAddr, TestWorker<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = TestWorker::spawn(move || handler(accept_test_stream(&listener)));
        (address, worker)
    }

    fn socket_pair() -> (Connection, Connection) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = connect_test_peer(listener.local_addr().unwrap());
        let server = Connection::new(accept_test_stream(&listener), TEST_TIMEOUT).unwrap();
        (client, server)
    }

    fn connect_test_peer(address: SocketAddr) -> Connection {
        let stream = TcpStream::connect_timeout(&address, TEST_TIMEOUT).unwrap();
        Connection::new(stream, TEST_TIMEOUT).unwrap()
    }

    fn open_request(address: SocketAddr) -> Connection {
        let mut connection = connect_test_peer(address);
        connection.write_all(CLIENT_HELLO).unwrap();
        connection
            .expect_marker(SERVER_HELLO, "test server handshake")
            .unwrap();
        connection
    }

    fn accept_request(stream: TcpStream) -> Connection {
        let mut connection = Connection::new(stream, TEST_TIMEOUT).unwrap();
        connection
            .expect_marker(CLIENT_HELLO, "test client handshake")
            .unwrap();
        connection.write_all(SERVER_HELLO).unwrap();
        assert_eq!(read_message(&mut connection).unwrap(), "");
        connection
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    fn assert_rejected_frame(bytes: &[u8], expected_error: &str) {
        let server = TestServer::new(TEST_TIMEOUT);
        let mut connection = open_request(server.address);
        connection.write_all(bytes).unwrap();
        connection.stream.shutdown(Shutdown::Write).unwrap();
        let mut response = [0; 17];
        connection.read_exact(&mut response).unwrap();
        assert_eq!(&response, REJECTED);
        let error = server.worker.join().unwrap().unwrap_err();
        assert!(error.contains(expected_error), "{error}");
        assert!(server.messages.try_recv().is_err());
        assert!(!server.notified.load(Ordering::Acquire));
    }

    #[test]
    fn successful_forwarding_preserves_unicode_and_whitespace_paths() {
        let directory = TestDirectory::new();
        let path = directory.file("  سلام 世界 texture .textures ");
        let message = canonical_message(&path).unwrap();
        assert!(message.ends_with(" .textures "));
        let server = TestServer::new(TEST_TIMEOUT);
        send_request(server.address, &message, TEST_TIMEOUT).unwrap();
        assert_eq!(server.messages.recv_timeout(TEST_TIMEOUT).unwrap(), message);
        assert!(server.notified.load(Ordering::Acquire));
        server.worker.join().unwrap().unwrap();
    }

    #[test]
    fn activation_is_delivered_as_an_empty_message() {
        let server = TestServer::new(TEST_TIMEOUT);
        send_request(server.address, "", TEST_TIMEOUT).unwrap();
        assert_eq!(server.messages.recv_timeout(TEST_TIMEOUT).unwrap(), "");
        assert!(server.notified.load(Ordering::Acquire));
        server.worker.join().unwrap().unwrap();
    }

    #[test]
    fn listener_accepts_before_ui_initialization_and_stops_on_drop() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let listener = start_ipc_listener(listener, tx).unwrap();
        send_request(address, "", TEST_TIMEOUT).unwrap();
        assert_eq!(rx.recv_timeout(TEST_TIMEOUT).unwrap(), "");
        listener.set_context(&egui::Context::default());
        let directory = TestDirectory::new();
        let message = canonical_message(&directory.file("texture.textures")).unwrap();
        send_request(address, &message, TEST_TIMEOUT).unwrap();
        assert_eq!(rx.recv_timeout(TEST_TIMEOUT).unwrap(), message);
        drop(listener);
        assert_eq!(
            rx.recv_timeout(TEST_TIMEOUT),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn receiver_canonicalizes_existing_absolute_paths() {
        let directory = TestDirectory::new();
        let path = directory.file("texture.textures");
        let message = directory.0.join(".").join("texture.textures");
        let server = TestServer::new(TEST_TIMEOUT);
        send_request(server.address, message.to_str().unwrap(), TEST_TIMEOUT).unwrap();
        assert_eq!(
            server.messages.recv_timeout(TEST_TIMEOUT).unwrap(),
            path.to_str().unwrap()
        );
        server.worker.join().unwrap().unwrap();
    }

    #[test]
    fn unknown_services_do_not_receive_path_payloads() {
        let (address, worker) = spawn_peer(|stream| {
            let mut connection = Connection::new(stream, TEST_TIMEOUT).unwrap();
            connection
                .expect_marker(CLIENT_HELLO, "test client handshake")
                .unwrap();
            connection.write_all(&[b'?'; 17]).unwrap();
            assert_eq!(connection.read(&mut [0; 1]).unwrap(), 0);
        });
        let error = send_request(address, "private path contents", TEST_TIMEOUT).unwrap_err();
        assert!(error.contains("server handshake"));
        worker.join().unwrap();
    }

    #[test]
    fn receiver_rejects_wrong_protocol_versions() {
        let server = TestServer::new(TEST_TIMEOUT);
        let mut connection = connect_test_peer(server.address);
        connection.write_all(b"TEXembed/2 HELLO\n").unwrap();
        let error = server.worker.join().unwrap().unwrap_err();
        assert!(error.contains("client handshake"));
        assert!(server.messages.try_recv().is_err());
    }

    #[test]
    fn receiver_rejects_unversioned_legacy_requests() {
        let server = TestServer::new(TEST_TIMEOUT);
        let mut connection = connect_test_peer(server.address);
        connection.write_all(b"old.textures").unwrap();
        connection.stream.shutdown(Shutdown::Write).unwrap();
        let error = server.worker.join().unwrap().unwrap_err();
        assert!(error.contains("client handshake"));
        assert!(server.messages.try_recv().is_err());
    }

    #[test]
    fn sender_requires_the_correct_acknowledgement() {
        let (address, worker) = spawn_peer(|stream| {
            accept_request(stream).write_all(SERVER_HELLO).unwrap();
        });
        let error = send_request(address, "", TEST_TIMEOUT).unwrap_err();
        assert!(error.contains("acknowledgement"));
        worker.join().unwrap();
    }

    #[test]
    fn sender_rejects_truncated_acknowledgements() {
        let (address, worker) = spawn_peer(|stream| {
            accept_request(stream)
                .write_all(&ACKNOWLEDGED[..8])
                .unwrap();
        });
        let error = send_request(address, "", TEST_TIMEOUT).unwrap_err();
        assert!(error.contains("acknowledgement"));
        worker.join().unwrap();
    }

    #[test]
    fn sender_rejects_trailing_acknowledgement_bytes() {
        let (address, worker) = spawn_peer(|stream| {
            let mut connection = accept_request(stream);
            let mut response = ACKNOWLEDGED.to_vec();
            response.push(b'!');
            connection.write_all(&response).unwrap();
        });
        let error = send_request(address, "", TEST_TIMEOUT).unwrap_err();
        assert!(error.contains("acknowledgement frame"));
        worker.join().unwrap();
    }

    #[test]
    fn disconnected_ui_is_not_acknowledged_as_success() {
        let (address, worker) = spawn_peer(|stream| {
            let (tx, rx) = mpsc::channel();
            drop(rx);
            let error = receive_request(stream, &tx, TEST_TIMEOUT, || {
                panic!("Rejected requests must not notify the UI");
            })
            .unwrap_err();
            assert!(error.contains("no longer accepting"));
        });
        let error = send_request(address, "", TEST_TIMEOUT).unwrap_err();
        assert!(error.contains("rejected"));
        worker.join().unwrap();
    }

    #[test]
    fn oversized_lengths_are_rejected_before_reading_payloads() {
        for size in [u32::try_from(MAX_PAYLOAD_BYTES + 1).unwrap(), u32::MAX] {
            assert_rejected_frame(&size.to_be_bytes(), "exceeds 64 KiB");
        }
    }

    #[test]
    fn outgoing_oversized_payloads_are_rejected_before_connecting() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let error = send_request(
            listener.local_addr().unwrap(),
            &"x".repeat(MAX_PAYLOAD_BYTES + 1),
            TEST_TIMEOUT,
        )
        .unwrap_err();
        assert!(error.contains("exceeds 64 KiB"));
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn payload_size_limit_is_inclusive() {
        assert_rejected_frame(&frame(&[0xff; MAX_PAYLOAD_BYTES]), "not valid UTF-8");
    }

    #[test]
    fn invalid_utf8_and_relative_paths_are_rejected() {
        assert_rejected_frame(&frame(&[0xff, 0xfe]), "not valid UTF-8");
        assert_rejected_frame(&frame(b"relative path.textures"), "must be absolute");
    }

    #[test]
    fn nul_bytes_and_missing_paths_are_rejected() {
        let directory = TestDirectory::new();
        let path = directory.0.join("missing.textures");
        let message = path.to_str().unwrap();
        assert_rejected_frame(&frame(message.as_bytes()), "Could not resolve the IPC path");
        let invalid = format!("{message}\0");
        assert_rejected_frame(&frame(invalid.as_bytes()), "contain no NUL bytes");
    }

    #[test]
    fn truncated_headers_and_payloads_are_rejected() {
        assert_rejected_frame(&[0, 0, 0], "frame length");
        let mut bytes = 10_u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"short");
        assert_rejected_frame(&bytes, "payload frame");
    }

    #[test]
    fn trailing_bytes_are_rejected_without_delivering_activation() {
        let mut bytes = frame(b"");
        bytes.push(b'!');
        assert_rejected_frame(&bytes, "Unexpected bytes after the IPC frame");
    }

    #[test]
    fn stalled_server_handshakes_time_out() {
        let (mut client, mut server) = socket_pair();
        client.write_all(CLIENT_HELLO).unwrap();
        server
            .expect_marker(CLIENT_HELLO, "test client handshake")
            .unwrap();
        let worker = TestWorker::spawn(move || {
            client.deadline = request_deadline(SHORT_TIMEOUT).unwrap();
            client.expect_marker(SERVER_HELLO, "server handshake")
        });
        let error = worker.join().unwrap().unwrap_err();
        assert!(error.contains("server handshake") && error.contains("timed out"));
        drop(server);
    }

    #[test]
    fn forwarding_to_an_unresponsive_service_times_out() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = TestWorker::spawn(move || send_request(address, "", REQUEST_TIMEOUT));
        let error = worker.join().unwrap().unwrap_err();
        assert!(error.contains("server handshake") && error.contains("timed out"));
        drop(listener);
    }

    #[test]
    fn stalled_acknowledgements_time_out() {
        let (mut client, mut server) = socket_pair();
        write_request(&mut client, "").unwrap();
        assert_eq!(read_message(&mut server).unwrap(), "");
        let worker = TestWorker::spawn(move || {
            client.deadline = request_deadline(SHORT_TIMEOUT).unwrap();
            read_acknowledgement(&mut client)
        });
        let error = worker.join().unwrap().unwrap_err();
        assert!(error.contains("acknowledgement") && error.contains("timed out"));
        drop(server);
    }

    #[test]
    fn acknowledgement_without_connection_close_times_out() {
        let (mut client, mut server) = socket_pair();
        server.write_all(ACKNOWLEDGED).unwrap();
        let worker = TestWorker::spawn(move || {
            client.deadline = request_deadline(SHORT_TIMEOUT).unwrap();
            read_acknowledgement(&mut client)
        });
        let error = worker.join().unwrap().unwrap_err();
        assert!(error.contains("acknowledgement frame") && error.contains("timed out"));
        drop(server);
    }

    #[test]
    fn stalled_client_handshakes_time_out() {
        let (mut client, server) = socket_pair();
        let (tx, rx) = mpsc::channel();
        client.write_all(&CLIENT_HELLO[..3]).unwrap();
        let worker = TestWorker::spawn(move || {
            receive_request(server.stream, &tx, SHORT_TIMEOUT, || {
                panic!("Incomplete handshakes must not notify the UI");
            })
        });
        let error = worker.join().unwrap().unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(rx.try_recv().is_err());
        drop(client);
    }

    #[test]
    fn stalled_payloads_and_missing_frame_termination_time_out() {
        for bytes in [10_u32.to_be_bytes(), 0_u32.to_be_bytes()] {
            let (mut client, mut server) = socket_pair();
            let (tx, rx) = mpsc::channel();
            client.write_all(&bytes).unwrap();
            let worker = TestWorker::spawn(move || {
                server.deadline = request_deadline(SHORT_TIMEOUT).unwrap();
                receive_payload(&mut server, &tx, || {
                    panic!("Incomplete payloads must not notify the UI");
                })
            });
            let error = worker.join().unwrap().unwrap_err();
            assert!(error.contains("timed out"), "{error}");
            assert!(rx.try_recv().is_err());
            drop(client);
        }
    }

    #[test]
    fn stalled_writes_use_the_same_finite_deadline() {
        let (mut client, server) = socket_pair();
        let worker = TestWorker::spawn(move || {
            client.deadline = request_deadline(SHORT_TIMEOUT).unwrap();
            let payload = [0; MAX_PAYLOAD_BYTES];
            loop {
                if let Err(error) = client.write_all(&payload) {
                    return error;
                }
            }
        });
        let error = worker.join().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(server);
    }

    #[test]
    fn partial_progress_does_not_reset_the_request_deadline() {
        let (mut client, mut server) = socket_pair();
        server.write_all(b"x").unwrap();
        let worker = TestWorker::spawn(move || {
            let original_deadline = client.deadline;
            client.read_exact(&mut [0; 1]).unwrap();
            assert_eq!(client.deadline, original_deadline);
            client.deadline = request_deadline(SHORT_TIMEOUT).unwrap();
            let read_error = client.read_exact(&mut [0; 1]).unwrap_err();
            let write_error = client.write_all(b"x").unwrap_err();
            (read_error.kind(), write_error.kind())
        });
        let (read_error, write_error) = worker.join().unwrap();
        assert_eq!(read_error, io::ErrorKind::TimedOut);
        assert_eq!(write_error, io::ErrorKind::TimedOut);
        drop(server);
    }

    #[test]
    fn connection_setup_preserves_the_original_deadline() {
        let (client, server) = socket_pair();
        let deadline = request_deadline(TEST_TIMEOUT).unwrap();
        let connection = Connection::with_deadline(client.stream, deadline).unwrap();
        assert_eq!(connection.deadline, deadline);
        drop(server);
    }

    #[test]
    fn invalid_timeouts_are_rejected_before_connecting() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        for timeout in [Duration::ZERO, Duration::MAX] {
            let error = send_request(address, "", timeout).unwrap_err();
            assert!(error.contains("request timeout"));
        }
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn expired_requests_are_not_delivered() {
        let (mut client, mut server) = socket_pair();
        write_request(&mut client, "").unwrap();
        let (tx, rx) = mpsc::channel();
        server.deadline = Instant::now();
        let error = receive_payload(&mut server, &tx, || {
            panic!("Expired requests must not notify the UI");
        })
        .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn listener_drop_does_not_wait_for_a_stuck_worker() {
        let (release, released) = mpsc::channel();
        let (finished, completion) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let worker = thread::spawn(move || {
            let _ = released.recv_timeout(TEST_TIMEOUT);
            finished.send(()).unwrap();
        });
        let listener = IpcListener {
            stopping,
            context: Arc::new(Mutex::new(None)),
            worker: Some(worker),
        };
        let start = Instant::now();
        drop(listener);
        let elapsed = start.elapsed();
        release.send(()).unwrap();
        completion.recv_timeout(TEST_TIMEOUT).unwrap();
        assert!(worker_stopping.load(Ordering::Acquire));
        assert!(elapsed < TEST_TIMEOUT);
    }

    #[test]
    fn refused_connections_are_reported() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = send_request(address, "", TEST_TIMEOUT).unwrap_err();
        assert!(error.contains("Could not connect"));
    }
}
