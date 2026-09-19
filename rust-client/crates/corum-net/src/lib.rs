//! Transporte para a sessão do Corum.
//!
//! O `BaseNetwork.dll` do cliente antigo entrega mensagens já delimitadas ao callback, mas o
//! framing do TCP não está exposto nos fontes. Por isso este crate não presume que cada `read`
//! seja um pacote: ele acumula bytes e calcula o tamanho dos pacotes de login conhecidos a partir
//! do cabeçalho `status/comando`. O framing específico da DLL pode ser encaixado depois sem
//! alterar os codecs de `corum-wire`.

use corum_wire::{
    CHARACTER_SUMMARY_SIZE, CMD_CHARACTER_SELECT_FAIL, CMD_CREATE_CHARACTER_SUCCESS,
    CMD_ENCRYPTION_KEY, CMD_LOGIN_FAIL, CMD_LOGIN_SUCCESS, STATUS_CHARACTER_SELECT, STATUS_LOGIN,
};
use std::fmt;
use std::io::{self, Read, Write};

const MAX_PACKET_SIZE: usize = 4 + 4 * CHARACTER_SUMMARY_SIZE;

#[derive(Debug)]
pub enum NetError {
    Io(io::Error),
    InvalidPacket { status: u8, command: u8 },
    InvalidCharacterCount(u8),
    PacketTooLarge { size: usize, maximum: usize },
}

impl fmt::Display for NetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "network I/O: {error}"),
            Self::InvalidPacket { status, command } => {
                write!(
                    formatter,
                    "unknown session packet status={status} command={command}"
                )
            }
            Self::InvalidCharacterCount(count) => {
                write!(
                    formatter,
                    "login response contains {count} characters; maximum is 4"
                )
            }
            Self::PacketTooLarge { size, maximum } => {
                write!(formatter, "packet is {size} bytes; maximum is {maximum}")
            }
        }
    }
}

impl std::error::Error for NetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for NetError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Uma conexão sobre qualquer stream que implemente `Read + Write`.
///
/// O tipo é genérico para que o cliente real use `TcpStream` e os testes usem um stream em
/// memória, sem abrir uma porta nem depender do servidor legado.
pub struct PacketConnection<S> {
    stream: S,
}

impl<S> PacketConnection<S> {
    #[must_use]
    pub const fn new(stream: S) -> Self {
        Self { stream }
    }

    pub fn into_inner(self) -> S {
        self.stream
    }
}

impl<S: Read + Write> PacketConnection<S> {
    /// Envia um pacote completo. O TCP pode dividir essa escrita internamente; `write_all`
    /// garante que nenhum byte seja perdido antes de devolver o controle ao estado da sessão.
    pub fn send(&mut self, packet: &[u8]) -> Result<(), NetError> {
        if packet.len() > MAX_PACKET_SIZE {
            return Err(NetError::PacketTooLarge {
                size: packet.len(),
                maximum: MAX_PACKET_SIZE,
            });
        }
        self.stream.write_all(packet)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Lê um pacote de login/seleção, mesmo quando chega fragmentado ou junto com o seguinte.
    pub fn receive_session_packet(&mut self) -> Result<Vec<u8>, NetError> {
        let mut packet = [0_u8; 4];
        self.stream.read_exact(&mut packet[..2])?;
        let prefix_len =
            usize::from(packet[0] == STATUS_LOGIN && packet[1] == CMD_LOGIN_SUCCESS) * 2 + 2;
        let size = packet_size(packet[0], packet[1], &mut self.stream, &mut packet[2..4])?;
        let mut output = packet[..prefix_len].to_vec();
        let mut rest = vec![0_u8; size - output.len()];
        self.stream.read_exact(&mut rest)?;
        output.extend(rest);
        Ok(output)
    }
}

/// Determina o tamanho dos pacotes de sessão conhecidos. Para o login-success, lê também o
/// contador de personagens (os bytes 2 e 3), porque o pacote tem tamanho variável.
fn packet_size<R: Read>(
    status: u8,
    command: u8,
    stream: &mut R,
    prefix: &mut [u8],
) -> Result<usize, NetError> {
    match (status, command) {
        (STATUS_LOGIN, CMD_LOGIN_FAIL) => Ok(7),
        (STATUS_LOGIN, CMD_ENCRYPTION_KEY) => Ok(12),
        (STATUS_LOGIN, CMD_LOGIN_SUCCESS) => {
            stream.read_exact(&mut prefix[..2])?;
            let count = prefix[1];
            if count > 4 {
                return Err(NetError::InvalidCharacterCount(count));
            }
            Ok(4 + usize::from(count) * CHARACTER_SUMMARY_SIZE)
        }
        (STATUS_CHARACTER_SELECT, CMD_CREATE_CHARACTER_SUCCESS) => Ok(22),
        (STATUS_CHARACTER_SELECT, CMD_CHARACTER_SELECT_FAIL) => Ok(3),
        _ => Err(NetError::InvalidPacket { status, command }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, ErrorKind};

    #[derive(Debug)]
    struct Chunked {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
        chunk_size: usize,
    }

    impl Read for Chunked {
        fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
            let limit = target.len().min(self.chunk_size);
            self.input.read(&mut target[..limit])
        }
    }

    impl Write for Chunked {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn login_success_packet() -> Vec<u8> {
        let mut packet = vec![STATUS_LOGIN, CMD_LOGIN_SUCCESS, 0, 1];
        packet.extend([0; CHARACTER_SUMMARY_SIZE]);
        packet
    }

    #[test]
    fn receives_a_fragmented_login_success() {
        let packet = login_success_packet();
        let stream = Chunked {
            input: Cursor::new(packet.clone()),
            output: Vec::new(),
            chunk_size: 1,
        };
        let mut connection = PacketConnection::new(stream);
        assert_eq!(connection.receive_session_packet().unwrap(), packet);
    }

    #[test]
    fn receives_two_coalesced_fixed_size_packets_in_order() {
        let first = vec![STATUS_LOGIN, CMD_LOGIN_FAIL, 6, 1, 0, 0, 0];
        let second = vec![STATUS_CHARACTER_SELECT, CMD_CHARACTER_SELECT_FAIL, 3];
        let stream = Cursor::new([first.clone(), second.clone()].concat());
        let mut connection = PacketConnection::new(stream);
        assert_eq!(connection.receive_session_packet().unwrap(), first);
        assert_eq!(connection.receive_session_packet().unwrap(), second);
    }

    #[test]
    fn sends_all_bytes_and_rejects_unknown_packets() {
        let stream = Chunked {
            input: Cursor::new(vec![9, 9]),
            output: Vec::new(),
            chunk_size: 8,
        };
        let mut connection = PacketConnection::new(stream);
        connection.send(&[STATUS_LOGIN, 0, 1]).unwrap();
        let stream = connection.into_inner();
        assert_eq!(stream.output, [STATUS_LOGIN, 0, 1]);

        let mut connection = PacketConnection::new(Cursor::new(vec![9, 9]));
        assert!(matches!(
            connection.receive_session_packet(),
            Err(NetError::InvalidPacket {
                status: 9,
                command: 9
            })
        ));
    }

    #[test]
    fn rejects_a_login_response_with_too_many_characters() {
        let mut connection =
            PacketConnection::new(Cursor::new(vec![STATUS_LOGIN, CMD_LOGIN_SUCCESS, 0, 5]));
        assert!(matches!(
            connection.receive_session_packet(),
            Err(NetError::InvalidCharacterCount(5))
        ));
    }

    #[test]
    fn propagates_eof_from_a_truncated_header() {
        let mut connection = PacketConnection::new(Cursor::new(vec![STATUS_LOGIN]));
        assert!(matches!(
            connection.receive_session_packet(),
            Err(NetError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof
        ));
    }
}
