use corum_net::{LoginReply, PacketConnection};
use corum_wire::{
    CharacterSelectRequest, ConnectWorldServer, LoginRequest, CMD_CONNECT_WORLD_SERVER,
};
use std::error::Error;

const DEFAULT_ADDRESS: &str = "127.0.0.1:13100";
const DEFAULT_VERSION: u32 = 0x0703_2101;

fn main() {
    if let Err(error) = run() {
        eprintln!("erro: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        print_help();
        return Ok(());
    }

    let address = arguments
        .first()
        .map(String::as_str)
        .unwrap_or(DEFAULT_ADDRESS);
    let id = arguments.get(1).map(String::as_str).ok_or("ID ausente")?;
    let password = arguments
        .get(2)
        .map(String::as_str)
        .ok_or("senha ausente")?;
    let version = arguments
        .get(3)
        .map(|value| parse_version(value))
        .transpose()?
        .unwrap_or(DEFAULT_VERSION);
    let character_slot = arguments
        .get(4)
        .map(|value| value.parse::<u8>())
        .transpose()?;

    println!("conectando ao LoginAgent em {address}...");
    let mut connection = PacketConnection::connect(address)?;
    let request = LoginRequest::new(id, password, version);
    println!("enviando login id={id:?}, versão=0x{version:08X}");

    match connection.authenticate(&request)? {
        LoginReply::Success(success) => {
            println!("LOGIN_SUCCESS: {} personagem(ns)", success.characters.len());
            for character in success.characters {
                println!(
                    "  slot={} índice={} nome={:?} nível={} mapa={}",
                    character.character_slot,
                    character.character_index,
                    character.name,
                    character.level,
                    character.recent_world_map
                );
            }

            if let Some(slot) = character_slot {
                println!("selecionando personagem no slot {slot}...");
                connection.select_character(CharacterSelectRequest::new(slot))?;
                let packet = connection.receive_session_packet()?;
                if packet.get(1).copied() == Some(CMD_CONNECT_WORLD_SERVER) {
                    let handoff = ConnectWorldServer::decode(&packet)?;
                    println!(
                        "WORLD_HANDOFF: ip=0x{:08X} porta={} personagem={} serial={} evento={}",
                        handoff.ip,
                        handoff.port,
                        handoff.character_index,
                        handoff.serial_code,
                        handoff.event_flag
                    );
                } else {
                    println!(
                        "CHARACTER_SELECT_REPLY: status={} comando={} tamanho={}",
                        packet.first().copied().unwrap_or_default(),
                        packet.get(1).copied().unwrap_or_default(),
                        packet.len()
                    );
                }
            }
        }
        LoginReply::Failure(failure) => {
            println!(
                "LOGIN_FAILURE: resultado={} dados_extras=0x{:08X}",
                failure.result, failure.extra_data
            );
            std::process::exit(2);
        }
        LoginReply::EncryptionKey(key) => {
            println!(
                "ENCRYPTION_KEY: servidor respondeu, mas a derivação legada ainda não está ligada"
            );
            println!("  chave={}", hex(&key.server_key));
            std::process::exit(3);
        }
    }

    Ok(())
}

fn parse_version(value: &str) -> Result<u32, Box<dyn Error>> {
    let (radix, digits) = value
        .strip_prefix("0x")
        .map_or((10, value), |digits| (16, digits));
    Ok(u32::from_str_radix(digits, radix)?)
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
        "Uso: corum-login-probe [endereço] <id> <senha> [versão] [slot]\n\n\
         Exemplo:\n  corum-login-probe 127.0.0.1:13100 teste senha 0x07032101 0\n\n\
         A versão padrão é 0x07032101. O probe não inicia servidores."
    );
}
