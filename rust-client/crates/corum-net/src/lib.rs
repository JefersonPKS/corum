//! Transporte para a sessão do Corum.
//!
//! O cliente standalone usa um frame TCP simples: dois bytes little-endian com o tamanho do
//! pacote, seguidos pelos bytes do pacote. A captura do cliente original confirmou esse formato
//! (`33 00` + login de 51 bytes e `56 00` + resposta de 86 bytes). O frame é tratado aqui para
//! que os codecs de `corum-wire` recebam somente o payload legado.

use corum_wire::{
    CharacterSelectRequest, EncryptionKey, LoginFailure, LoginRequest, LoginSuccess,
    CHARACTER_SUMMARY_SIZE, CMD_CHARACTER_SELECT_FAIL, CMD_CONNECT_WORLD_SERVER,
    CMD_CREATE_CHARACTER_SUCCESS, CMD_ENCRYPTION_KEY, CMD_LOGIN_FAIL, CMD_LOGIN_SUCCESS,
    STATUS_CHARACTER_SELECT, STATUS_LOGIN,
};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};

const MAX_PACKET_SIZE: usize = 4 + 4 * CHARACTER_SUMMARY_SIZE;

#[derive(Debug)]
pub enum NetError {
    Io(io::Error),
    InvalidPacket { status: u8, command: u8 },
    InvalidFrameLength(usize),
    InvalidCharacterCount(u8),
    PacketTooLarge { size: usize, maximum: usize },
}

#[derive(Debug)]
pub enum SessionError {
    Transport(NetError),
    Wire(corum_wire::WireError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "session transport: {error}"),
            Self::Wire(error) => write!(formatter, "session packet: {error}"),
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Wire(error) => Some(error),
        }
    }
}

impl From<NetError> for SessionError {
    fn from(error: NetError) -> Self {
        Self::Transport(error)
    }
}

impl From<corum_wire::WireError> for SessionError {
    fn from(error: corum_wire::WireError) -> Self {
        Self::Wire(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoginReply {
    Success(LoginSuccess),
    Failure(LoginFailure),
    EncryptionKey(EncryptionKey),
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
            Self::InvalidFrameLength(length) => {
                write!(formatter, "invalid network frame length: {length} bytes")
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

impl PacketConnection<TcpStream> {
    pub fn connect(address: impl ToSocketAddrs) -> Result<Self, NetError> {
        Ok(Self::new(TcpStream::connect(address)?))
    }
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
    /// Envia um pacote completo com o prefixo de tamanho little-endian usado pelo cliente legado.
    /// O TCP pode dividir essa escrita internamente; `write_all` garante que nenhum byte seja
    /// perdido antes de devolver o controle ao estado da sessão.
    pub fn send(&mut self, packet: &[u8]) -> Result<(), NetError> {
        if packet.len() > MAX_PACKET_SIZE {
            return Err(NetError::PacketTooLarge {
                size: packet.len(),
                maximum: MAX_PACKET_SIZE,
            });
        }
        let frame_length = u16::try_from(packet.len()).map_err(|_| NetError::PacketTooLarge {
            size: packet.len(),
            maximum: usize::from(u16::MAX),
        })?;
        self.stream.write_all(&frame_length.to_le_bytes())?;
        self.stream.write_all(packet)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Lê um frame de login/seleção, mesmo quando chega fragmentado ou junto com o seguinte.
    pub fn receive_session_packet(&mut self) -> Result<Vec<u8>, NetError> {
        let mut frame_header = [0_u8; 2];
        self.stream.read_exact(&mut frame_header)?;
        let frame_length = usize::from(u16::from_le_bytes(frame_header));
        if frame_length < 2 {
            return Err(NetError::InvalidFrameLength(frame_length));
        }
        if frame_length > MAX_PACKET_SIZE {
            return Err(NetError::PacketTooLarge {
                size: frame_length,
                maximum: MAX_PACKET_SIZE,
            });
        }

        let mut packet = vec![0_u8; frame_length];
        self.stream.read_exact(&mut packet)?;
        let expected_length = packet_size(&packet)?;
        if expected_length != frame_length {
            return Err(NetError::InvalidFrameLength(frame_length));
        }
        Ok(packet)
    }

    /// Envia o primeiro pacote do estado de login e decodifica a resposta do `LoginAgent`.
    ///
    /// Quando o servidor responde com `EncryptionKey`, a conexão está viva, mas a derivação da
    /// chave precisa ser ligada ao algoritmo legado antes de reenviar o login. Retornar essa
    /// variante evita tratar uma sessão criptografada como uma falha de senha.
    pub fn authenticate(&mut self, request: &LoginRequest) -> Result<LoginReply, SessionError> {
        self.send(&request.encode()?)?;
        let packet = self.receive_session_packet()?;
        match packet.get(1).copied() {
            Some(CMD_LOGIN_SUCCESS) => Ok(LoginReply::Success(LoginSuccess::decode(&packet)?)),
            Some(CMD_LOGIN_FAIL) => Ok(LoginReply::Failure(LoginFailure::decode(&packet)?)),
            Some(CMD_ENCRYPTION_KEY) => {
                Ok(LoginReply::EncryptionKey(EncryptionKey::decode(&packet)?))
            }
            _ => Err(NetError::InvalidPacket {
                status: packet[0],
                command: packet[1],
            }
            .into()),
        }
    }

    /// Envia a seleção de um dos quatro slots. A resposta de entrada no WorldServer será
    /// adicionada quando o pacote `WORLD_USER_INFO` fizer parte do próximo bloco de codecs.
    pub fn select_character(&mut self, request: CharacterSelectRequest) -> Result<(), NetError> {
        self.send(&request.encode())
    }
}

/// Determina o tamanho esperado dos pacotes de sessão conhecidos. Para o login-success, usa o
/// contador de personagens nos bytes 2 e 3, porque o pacote tem tamanho variável.
fn packet_size(packet: &[u8]) -> Result<usize, NetError> {
    let status = packet.first().copied().unwrap_or_default();
    let command = packet.get(1).copied().unwrap_or_default();
    match (status, command) {
        (STATUS_LOGIN, CMD_LOGIN_FAIL) => Ok(7),
        (STATUS_LOGIN, CMD_ENCRYPTION_KEY) => Ok(12),
        (STATUS_LOGIN, CMD_LOGIN_SUCCESS) => {
            let count = packet
                .get(3)
                .copied()
                .ok_or(NetError::InvalidFrameLength(packet.len()))?;
            if count > 4 {
                return Err(NetError::InvalidCharacterCount(count));
            }
            Ok(4 + usize::from(count) * CHARACTER_SUMMARY_SIZE)
        }
        (STATUS_CHARACTER_SELECT, CMD_CREATE_CHARACTER_SUCCESS) => Ok(22),
        (STATUS_CHARACTER_SELECT, CMD_CHARACTER_SELECT_FAIL) => Ok(3),
        (STATUS_CHARACTER_SELECT, CMD_CONNECT_WORLD_SERVER) => Ok(21),
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

    fn frame(packet: &[u8]) -> Vec<u8> {
        [
            u16::try_from(packet.len())
                .unwrap()
                .to_le_bytes()
                .as_slice(),
            packet,
        ]
        .concat()
    }

    #[test]
    fn receives_a_fragmented_login_success() {
        let packet = login_success_packet();
        let stream = Chunked {
            input: Cursor::new(frame(&packet)),
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
        let stream = Cursor::new([frame(&first), frame(&second)].concat());
        let mut connection = PacketConnection::new(stream);
        assert_eq!(connection.receive_session_packet().unwrap(), first);
        assert_eq!(connection.receive_session_packet().unwrap(), second);
    }

    #[test]
    fn sends_all_bytes_and_rejects_unknown_packets() {
        let stream = Chunked {
            input: Cursor::new(frame(&[9, 9])),
            output: Vec::new(),
            chunk_size: 8,
        };
        let mut connection = PacketConnection::new(stream);
        connection.send(&[STATUS_LOGIN, 0, 1]).unwrap();
        let stream = connection.into_inner();
        assert_eq!(stream.output, frame(&[STATUS_LOGIN, 0, 1]));

        let mut connection = PacketConnection::new(Cursor::new(frame(&[9, 9])));
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
            PacketConnection::new(Cursor::new(frame(&[STATUS_LOGIN, CMD_LOGIN_SUCCESS, 0, 5])));
        assert!(matches!(
            connection.receive_session_packet(),
            Err(NetError::InvalidCharacterCount(5))
        ));
    }

    #[test]
    fn propagates_eof_from_a_truncated_header() {
        let mut connection = PacketConnection::new(Cursor::new(vec![1]));
        assert!(matches!(
            connection.receive_session_packet(),
            Err(NetError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn authenticates_against_an_in_memory_login_agent_reply() {
        let input = frame(&[STATUS_LOGIN, CMD_LOGIN_FAIL, 6, 0, 0, 0, 0]);
        let stream = Chunked {
            input: Cursor::new(input),
            output: Vec::new(),
            chunk_size: 2,
        };
        let mut connection = PacketConnection::new(stream);
        let request = LoginRequest::new("tester", "secret", 0x0703_2101);
        let reply = connection.authenticate(&request).unwrap();
        assert_eq!(
            reply,
            LoginReply::Failure(LoginFailure {
                result: 6,
                extra_data: 0
            })
        );
        assert_eq!(connection.into_inner().output.len(), 53);
    }
}
