use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_LISTEN: &str = "127.0.0.1:13101";
const DEFAULT_TARGET: &str = "127.0.0.1:13100";
const DEFAULT_CAPTURE_DIR: &str = "captures/login-proxy";

fn main() {
    if let Err(error) = run() {
        eprintln!("erro: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        print_help();
        return Ok(());
    }

    let listen_address = arguments
        .first()
        .map(String::as_str)
        .unwrap_or(DEFAULT_LISTEN);
    let target_address = arguments
        .get(1)
        .map(String::as_str)
        .unwrap_or(DEFAULT_TARGET);
    let capture_root = arguments
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CAPTURE_DIR));

    fs::create_dir_all(&capture_root)?;
    let listener = TcpListener::bind(listen_address)?;
    println!("proxy escutando em {listen_address} -> {target_address}");
    println!("aguardando uma conexão do cliente original...");

    let (client, client_address) = listener.accept()?;
    println!("cliente conectado: {client_address}");
    client.set_nodelay(true)?;

    let server = TcpStream::connect(target_address)?;
    server.set_nodelay(true)?;
    println!("conectado ao LoginAgent: {target_address}");

    let capture_dir = capture_root.join(format!("session-{}", capture_id()));
    fs::create_dir_all(&capture_dir)?;
    println!("captura: {}", capture_dir.display());

    let client_to_server = relay(
        client.try_clone()?,
        server.try_clone()?,
        capture_dir.join("client-to-server.bin"),
        capture_dir.join("client-to-server.log"),
        "cliente -> servidor",
    );
    let server_to_client = relay(
        server,
        client,
        capture_dir.join("server-to-client.bin"),
        capture_dir.join("server-to-client.log"),
        "servidor -> cliente",
    );

    let first = client_to_server.join();
    let second = server_to_client.join();
    first.map_err(|_| io::Error::other("thread cliente->servidor falhou"))??;
    second.map_err(|_| io::Error::other("thread servidor->cliente falhou"))??;
    println!("captura finalizada: {}", capture_dir.display());
    Ok(())
}

fn relay(
    mut input: TcpStream,
    mut output: TcpStream,
    binary_path: PathBuf,
    log_path: PathBuf,
    label: &'static str,
) -> thread::JoinHandle<io::Result<()>> {
    thread::spawn(move || {
        let mut binary = File::create(binary_path)?;
        let mut log = File::create(log_path)?;
        let mut buffer = [0_u8; 16 * 1024];
        let mut offset = 0_u64;

        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                let _ = output.shutdown(Shutdown::Write);
                writeln!(log, "EOF offset={offset}")?;
                println!("{label}: EOF, {offset} bytes");
                return Ok(());
            }

            output.write_all(&buffer[..read])?;
            output.flush()?;
            binary.write_all(&buffer[..read])?;
            writeln!(
                log,
                "offset={offset} len={read} hex={}",
                hex(&buffer[..read])
            )?;
            println!("{label}: {read} bytes (offset {offset})");
            offset += read as u64;
        }
    })
}

fn capture_id() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join("")
}

fn print_help() {
    println!(
        "Uso: corum-wire-proxy [escuta] [destino] [diretório-de-captura]\n\n\
         Padrões:\n  escuta: 127.0.0.1:13101\n  destino: 127.0.0.1:13100\n  captura: captures/login-proxy\n\n\
         O proxy aceita uma conexão, encaminha os bytes nos dois sentidos e encerra quando\n\
         o cliente ou o LoginAgent fecha a sessão."
    );
}
