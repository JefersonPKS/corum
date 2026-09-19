//! Som do sandbox: efeitos (`Data\Sound\<número>.wav`) e a música do mapa (`.mp3`).
//!
//! **As regras de qual arquivo tocar vêm de `DungeonProcess_Sound.cpp` e `Define.h` do cliente
//! original** (`_PlaySound`, `SelectBGM`): passos (`GAMEPLAY_HEABYSTONE_WALK/RUN`), voz do personagem por
//! classe (`SOUND_NUMBER_CHARACTER` = 3001, 10 sons por classe), golpe de arma (3101), impacto (3201),
//! sons da interface (4000 + tipo) e a música pelo número da camada. Os sons de **monstro** vêm do banco
//! de dados do servidor (`BaseMonsterInfo`), então não são tocados aqui.
//!
//! Sem dispositivo de áudio ou sem a pasta `Sound`, o jogo segue mudo.

use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

/// Um som que o jogo pede; o arquivo sai de `cue_file`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// Pé no chão (nos quadros-chave do movimento de andar/correr do `.cdt`).
    Footstep {
        run: bool,
    },
    /// O "vush" da arma (sem arma, o soco).
    Swing {
        armed: bool,
    },
    /// Grito de ataque do personagem (classe de 1 a 5).
    AttackVoice {
        class: u16,
    },
    /// Arma acertando o alvo.
    WeaponHit,
    /// Personagem ferido.
    Hurt {
        class: u16,
    },
    /// Personagem caindo.
    Death {
        class: u16,
    },
    WindowOpen,
    WindowClose,
}

/// Número do `.wav` para um som, dado um número qualquer para a escolha entre as variações
/// (`GetRandom` do cliente).
#[must_use]
pub fn cue_file(cue: Cue, roll: u32) -> u32 {
    let voice = |class: u16, offset: u32| 3001 + (u32::from(class.clamp(1, 5)) - 1) * 10 + offset;
    match cue {
        Cue::Footstep { run: false } => 2005 + roll % 4,
        Cue::Footstep { run: true } => 2001 + roll % 4,
        Cue::Swing { armed: false } => 3101 + roll % 3,
        // `dwWeapon * SOUND_PER_WEAPON` com `dwWeapon` = 1 para as armas comuns.
        Cue::Swing { armed: true } => 3111 + roll % 3,
        Cue::AttackVoice { class } => voice(class, roll % 3),
        Cue::Hurt { class } => voice(class, 3 + roll % 3),
        Cue::Death { class } => voice(class, 6 + roll % 2),
        Cue::WeaponHit => 3201 + roll % 2,
        Cue::WindowOpen => 4001,
        Cue::WindowClose => 4002,
    }
}

/// Música de um mapa pelo número da camada (`GetDungeonLayerProperty` + `SelectBGM`). O número do
/// `.ttb` é tomado como o da camada **[hipótese]**.
#[must_use]
pub fn music_for_map(map: u32) -> &'static str {
    match map {
        10_000.. => "Map_World1.mp3",
        0..=99 => "Town_World1.mp3",
        100..=199 => "Dungeon_Lair3.mp3",
        200..=299 => "Dungeon_Lair1.mp3",
        300..=400 => "Dungeon_Lair5.mp3",
        500..=599 => "Dungeon_Lair4.mp3",
        600..=699 => "Dungeon_Lair2.mp3",
        750 => "Dungeon_Ameritart.mp3",
        801 => "guild_battle.mp3",
        900..=999 => "Dungeon_Lair6.mp3",
        1100..=1199 => "Dungeon_Tower.mp3",
        1200..=1219 => "Dungeon_Aqua.mp3",
        1221..=1229 => "Dungeon_Lighthouse.mp3",
        // As demais camadas sorteiam `Dungeon_Hold1..3`; aqui fica a primeira.
        _ => "Dungeon_Hold1.mp3",
    }
}

/// Os quadros MP3 de um arquivo de música. **[confirmado]** Cinco dos 31 `.mp3` (`Dungeon_Lair1`, `_Lair2`,
/// `_Lair4`, `_Lair5`, `Dungeon_Tunnel1`) são na verdade um WAV com formato `0x55` (MPEG camada 3 dentro de
/// um contêiner RIFF), que o leitor de WAV não abre: aqui sai o conteúdo do bloco `data`. Os demais (com
/// ID3 ou quadros diretos) ficam como estão.
#[must_use]
pub fn mp3_stream(bytes: &[u8]) -> &[u8] {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return bytes;
    }
    let mut position = 12;
    let mut mpeg = false;
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = u32::from_le_bytes([
            bytes[position + 4],
            bytes[position + 5],
            bytes[position + 6],
            bytes[position + 7],
        ]) as usize;
        let body = position + 8;
        if id == b"fmt " && size >= 2 && body + 2 <= bytes.len() {
            mpeg = u16::from_le_bytes([bytes[body], bytes[body + 1]]) == 0x55;
        }
        if id == b"data" {
            let end = (body + size).min(bytes.len());
            return if mpeg { &bytes[body..end] } else { bytes };
        }
        position = body + size + (size & 1);
    }
    bytes
}

/// O dispositivo de áudio, os efeitos já lidos e a música em laço.
pub struct Audio {
    sink: MixerDeviceSink,
    directory: PathBuf,
    effects: HashMap<u32, Option<Arc<[u8]>>>,
    music: Option<Player>,
    state: u32,
    effects_on: bool,
}

impl Audio {
    /// Abre o dispositivo padrão; `None` se não houver (o jogo fica mudo).
    pub fn new(sound_directory: PathBuf, effects_on: bool) -> Option<Self> {
        let sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|error| eprintln!("audio unavailable: {error}"))
            .ok()?;
        Some(Self {
            sink,
            directory: sound_directory,
            effects: HashMap::new(),
            music: None,
            state: 0x9E37_79B9,
            effects_on,
        })
    }

    /// Número pseudoaleatório (xorshift) para variar os sons, sem depender de outra biblioteca.
    pub fn roll(&mut self) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state >> 8
    }

    /// Toca um efeito (fogo e esquece). Arquivo ausente ou ilegível é ignorado.
    pub fn play(&mut self, cue: Cue) {
        if !self.effects_on {
            return;
        }
        let roll = self.roll();
        let number = cue_file(cue, roll);
        let directory = &self.directory;
        let bytes = self
            .effects
            .entry(number)
            .or_insert_with(|| {
                std::fs::read(directory.join(format!("{number}.wav")))
                    .ok()
                    .map(Arc::from)
            })
            .clone();
        let Some(bytes) = bytes else {
            return;
        };
        if let Ok(decoder) = Decoder::new(Cursor::new(bytes)) {
            self.sink.mixer().add(decoder.amplify(volume_of(cue)));
        }
    }

    /// Começa a música (em laço) no volume dado, trocando a que tocava.
    pub fn play_music(&mut self, file: &str, volume: f32) {
        self.stop_music();
        let path = self.directory.join(file);
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("audio: music {} not found", path.display());
            return;
        };
        match Decoder::new_looped(Cursor::new(mp3_stream(&bytes).to_vec())) {
            Ok(source) => {
                let player = Player::connect_new(self.sink.mixer());
                player.set_volume(volume);
                player.append(source);
                self.music = Some(player);
                eprintln!("audio: music {file}");
            }
            Err(error) => eprintln!("audio: {file}: {error}"),
        }
    }

    /// Silencia ou retoma a música (tecla `M`); devolve `true` se agora está tocando.
    pub fn toggle_music(&mut self) -> bool {
        match &self.music {
            Some(player) if player.is_paused() => {
                player.play();
                true
            }
            Some(player) => {
                player.pause();
                false
            }
            None => false,
        }
    }

    pub fn stop_music(&mut self) {
        if let Some(player) = self.music.take() {
            player.stop();
        }
    }
}

/// Volume relativo de cada tipo de som (0 a 1).
fn volume_of(cue: Cue) -> f32 {
    match cue {
        Cue::Footstep { .. } => 0.35,
        Cue::WindowOpen | Cue::WindowClose => 0.5,
        _ => 0.7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footsteps_pick_among_four_variants_by_gait() {
        // `GAMEPLAY_HEABYSTONE_WALK` = 5 e `_RUN` = 1, mais `GetRandom(4)`, mais 2000.
        let walk: Vec<u32> = (0..8)
            .map(|r| cue_file(Cue::Footstep { run: false }, r))
            .collect();
        assert!(walk.iter().all(|n| (2005..=2008).contains(n)));
        assert_eq!(
            walk.iter().collect::<std::collections::BTreeSet<_>>().len(),
            4
        );
        assert_eq!(cue_file(Cue::Footstep { run: true }, 0), 2001);
        assert_eq!(cue_file(Cue::Footstep { run: true }, 7), 2004);
    }

    #[test]
    fn character_voices_are_ten_files_apart_per_class() {
        // `SOUND_NUMBER_CHARACTER` 3001 + (classe − 1) × 10 + {ataque 0, dano 3, morte 6} + variante.
        assert_eq!(cue_file(Cue::AttackVoice { class: 1 }, 0), 3001);
        assert_eq!(cue_file(Cue::AttackVoice { class: 2 }, 2), 3013);
        assert_eq!(cue_file(Cue::Hurt { class: 1 }, 0), 3004);
        assert_eq!(cue_file(Cue::Hurt { class: 5 }, 1), 3045);
        assert_eq!(cue_file(Cue::Death { class: 1 }, 1), 3008);
        assert_eq!(cue_file(Cue::Death { class: 3 }, 0), 3027);
        // Uma classe fora da faixa não sai das cinco.
        assert_eq!(cue_file(Cue::AttackVoice { class: 9 }, 0), 3041);
    }

    #[test]
    fn weapon_and_interface_sounds_use_their_ranges() {
        assert_eq!(cue_file(Cue::Swing { armed: false }, 0), 3101);
        assert_eq!(cue_file(Cue::Swing { armed: true }, 2), 3113);
        assert_eq!(cue_file(Cue::WeaponHit, 1), 3202);
        assert_eq!(cue_file(Cue::WindowOpen, 99), 4001);
        assert_eq!(cue_file(Cue::WindowClose, 99), 4002);
    }

    fn riff(format: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF    WAVE".to_vec();
        bytes.extend(b"fmt ");
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(format.to_le_bytes());
        bytes.extend([0, 0]);
        bytes.extend(b"fact");
        bytes.extend(3u32.to_le_bytes()); // tamanho ímpar: o bloco é preenchido com um byte
        bytes.extend([1, 2, 3, 0]);
        bytes.extend(b"data");
        bytes.extend((payload.len() as u32).to_le_bytes());
        bytes.extend(payload);
        bytes
    }

    #[test]
    fn mp3_inside_a_wav_container_is_unwrapped() {
        let frames = [0xFF, 0xFB, 0x90, 0x44, 1, 2, 3];
        assert_eq!(mp3_stream(&riff(0x55, &frames)), frames);
        // WAV comum (PCM) e arquivos que não são RIFF passam sem mudar.
        let pcm = riff(1, &frames);
        assert_eq!(mp3_stream(&pcm), pcm.as_slice());
        let id3 = b"ID3rest";
        assert_eq!(mp3_stream(id3), id3);
        assert_eq!(mp3_stream(&[]), &[] as &[u8]);
        // Um bloco `data` truncado não estoura.
        let mut cut = riff(0x55, &frames);
        cut.truncate(cut.len() - 3);
        assert_eq!(mp3_stream(&cut), &frames[..4]);
    }

    /// Com `CORUM_DATA`: os 31 arquivos de música abrem no decodificador.
    #[test]
    fn every_real_music_file_decodes() {
        let Some(data) = std::env::var_os("CORUM_DATA") else {
            return;
        };
        let sound = std::path::Path::new(&data).join("Sound");
        let mut failed = Vec::new();
        let mut count = 0;
        for entry in std::fs::read_dir(sound).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case("mp3"))
            {
                continue;
            }
            count += 1;
            let bytes = std::fs::read(&path).unwrap();
            if Decoder::new(Cursor::new(mp3_stream(&bytes).to_vec())).is_err() {
                failed.push(path.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
        assert_eq!(count, 31);
        assert!(failed.is_empty(), "cannot decode: {failed:?}");
    }

    #[test]
    fn music_follows_the_layer_number() {
        assert_eq!(music_for_map(604), "Dungeon_Lair2.mp3");
        assert_eq!(music_for_map(1100), "Dungeon_Tower.mp3");
        assert_eq!(music_for_map(5), "Town_World1.mp3");
        assert_eq!(music_for_map(750), "Dungeon_Ameritart.mp3");
        assert_eq!(music_for_map(10_001), "Map_World1.mp3");
        assert_eq!(music_for_map(450), "Dungeon_Hold1.mp3");
    }

    /// Com `CORUM_DATA`: todos os arquivos que as regras podem pedir existem em `Data\Sound`.
    #[test]
    fn every_file_the_rules_can_ask_for_exists() {
        let Some(data) = std::env::var_os("CORUM_DATA") else {
            return;
        };
        let sound = std::path::Path::new(&data).join("Sound");
        let mut missing = Vec::new();
        for class in 1..=5 {
            for cue in [
                Cue::AttackVoice { class },
                Cue::Hurt { class },
                Cue::Death { class },
            ] {
                for roll in 0..6 {
                    let name = format!("{}.wav", cue_file(cue, roll));
                    if !sound.join(&name).is_file() {
                        missing.push(name);
                    }
                }
            }
        }
        for cue in [
            Cue::Footstep { run: false },
            Cue::Footstep { run: true },
            Cue::Swing { armed: false },
            Cue::Swing { armed: true },
            Cue::WeaponHit,
            Cue::WindowOpen,
            Cue::WindowClose,
        ] {
            for roll in 0..6 {
                let name = format!("{}.wav", cue_file(cue, roll));
                if !sound.join(&name).is_file() {
                    missing.push(name);
                }
            }
        }
        // Existir não basta: cada efeito tem de abrir no decodificador.
        let mut undecodable = Vec::new();
        for number in (2001..=2008)
            .chain(3001..=3048)
            .chain(3101..=3153)
            .chain(3201..=3202)
            .chain(4001..=4002)
        {
            let Ok(bytes) = std::fs::read(sound.join(format!("{number}.wav"))) else {
                continue;
            };
            if Decoder::new(Cursor::new(bytes)).is_err() {
                undecodable.push(number);
            }
        }
        assert!(undecodable.is_empty(), "cannot decode: {undecodable:?}");
        for map in [
            5, 100, 250, 350, 550, 604, 750, 801, 950, 1100, 1210, 1225, 10_001, 450,
        ] {
            let name = music_for_map(map);
            if !sound.join(name).is_file() {
                missing.push(name.to_owned());
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "missing sound files: {missing:?}");
    }
}
